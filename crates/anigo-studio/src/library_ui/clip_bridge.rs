//! Ponte entre o workspace Animação e a Biblioteca: salvar o clip como ativo (`.anigoclip.json`, tipo `animation`)
//! e listar/ler os clips guardados para aplicá-los. Mesma ideia da `pose_bridge`: mesmo nome = nova revisão.

use super::*;
use anigo_core::clip_asset::{ClipAsset, CLIP_ASSET_SUFFIX};

/// Um clip lido da Biblioteca.
#[derive(Debug, Clone)]
pub struct ClipCard {
    pub id: AssetId,
    pub name: String,
    pub revision: u32,
    pub asset: ClipAsset,
}

impl LibraryWorkspace {
    /// Grava `asset` como ativo `Animation` autoral (tags `animacao`, `clip`, `autoral`). Se já existe um clip da
    /// Biblioteca com o mesmo nome, o conteúdo novo entra como NOVA REVISÃO.
    pub fn save_clip_asset(&mut self, asset: &ClipAsset) -> Result<String, String> {
        self.open_blocking()?;
        let rt = self.rt.as_ref().ok_or("a biblioteca não está aberta")?;
        let lib = rt.lib.clone();
        let name = asset.name.trim();
        if name.is_empty() {
            return Err("o clip precisa de um nome".into());
        }
        let json = asset.to_json().map_err(|e| e.to_string())?;
        let dir = std::env::temp_dir().join(format!("anigo-clip-{}-{}", std::process::id(), blake3::hash(name.as_bytes()).to_hex()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
        let file = dir.join(format!("{}{CLIP_ASSET_SUFFIX}", safe.trim()));
        let result = (|| {
            std::fs::write(&file, json).map_err(|e| e.to_string())?;
            let q = Query { text: Some(name.to_string()), kinds: vec![AssetKind::Animation], limit: 50, ..Default::default() };
            let existing = lib
                .query(&q)
                .ok()
                .and_then(|p| p.items.into_iter().find(|r| r.name == name && r.format == "anigoclip" && r.state == AssetState::Active))
                .map(|r| r.id);
            let mut tags = vec!["animacao".to_string(), "clip".to_string(), "autoral".to_string()];
            tags.extend(asset.tags.iter().cloned());
            let opts = ImportOptions { name: Some(name.to_string()), tags, kind: Some(AssetKind::Animation), author: Some("ANIGO Studio".into()), replace: existing, ..Default::default() };
            let rep = lib.import_blocking(&file, opts).map_err(|e| e.to_string())?;
            if rep.imported.first().is_some() {
                Ok(format!("clip \"{name}\" salvo na Biblioteca"))
            } else if rep.revised.first().is_some() {
                Ok(format!("clip \"{name}\": nova revisão"))
            } else if rep.duplicates.first().is_some() {
                Ok(format!("clip \"{name}\": já existe com o mesmo conteúdo"))
            } else if let Some((p, e)) = rep.failed.first() {
                Err(format!("{}: {e}", p.display()))
            } else {
                Err("nada foi importado".into())
            }
        })();
        let _ = std::fs::remove_dir_all(&dir);
        if result.is_ok() {
            self.reload_rows(false);
            self.reload_facets();
        }
        result
    }

    /// Clips `.anigoclip.json` ativos da Biblioteca (mais recentes primeiro), já lidos e validados.
    pub fn list_clip_assets(&mut self, limit: usize) -> Result<Vec<ClipCard>, String> {
        self.open_blocking()?;
        let lib = self.rt.as_ref().ok_or("a biblioteca não está aberta")?.lib.clone();
        let q = Query { kinds: vec![AssetKind::Animation], limit: limit.max(1), ..Default::default() };
        let page = lib.query(&q).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in page.items {
            if row.state != AssetState::Active || row.format != "anigoclip" {
                continue;
            }
            let Ok(files) = lib.resolve_files(&row.id) else { continue };
            let Ok(bytes) = std::fs::read(&files.primary) else { continue };
            if let Ok(asset) = ClipAsset::from_json(&bytes) {
                out.push(ClipCard { id: row.id, name: row.name.clone(), revision: row.revision, asset });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::animation::{canonicalize_clip, Channel, ChannelTarget, Key, KeyValue};
    use anigo_core::project::AnimationClip;

    fn clip(elbow_deg: f32) -> AnimationClip {
        let mut c = AnimationClip::new(anigo_core::AnimationClipId::from_slug("acenar"), "Acenar");
        c.range = Some([0, 24]);
        let q = |d: f32| KeyValue::Quat(glam::Quat::from_rotation_x(d.to_radians()).to_array());
        c.channels = vec![Channel { target: ChannelTarget::bone("LeftElbow"), keys: vec![Key::new(0, q(0.0)), Key::new(24, q(elbow_deg))] }];
        canonicalize_clip(&mut c);
        c
    }

    #[test]
    fn clips_are_saved_listed_revised_and_applied_as_undoable_commands() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path().join("lib")).unwrap();
        let mut ws = LibraryWorkspace::with_library(lib.clone());
        assert!(ws.list_clip_assets(50).unwrap().is_empty());
        let asset = ClipAsset::from_clip("Acenar", &clip(-80.0), 24.0).unwrap();
        let msg = ws.save_clip_asset(&asset).unwrap();
        assert!(msg.contains("salvo"), "{msg}");
        let rows = lib.query(&Query { kinds: vec![AssetKind::Animation], ..Default::default() }).unwrap().items;
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].name.as_str(), rows[0].format.as_str()), ("Acenar", "anigoclip"));
        assert!(lib.tags_of(&rows[0].id).unwrap().contains(&"clip".to_string()));
        let cards = ws.list_clip_assets(50).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].asset, asset);
        // mesmo conteúdo não duplica; conteúdo novo = revisão 2 do MESMO ativo
        let again = ws.save_clip_asset(&asset).unwrap();
        assert!(again.contains("já existe") || again.contains("revisão"), "{again}");
        let changed = ClipAsset::from_clip("Acenar", &clip(-120.0), 24.0).unwrap();
        assert!(ws.save_clip_asset(&changed).unwrap().contains("revisão"));
        let cards = ws.list_clip_assets(50).unwrap();
        assert_eq!((cards.len(), cards[0].revision), (1, 2));
        assert!(ws.save_clip_asset(&ClipAsset::from_clip("  ", &clip(-10.0), 24.0).unwrap()).is_err(), "nome vazio");
        // aplicar o card num projeto: um comando, um passo de desfazer
        let mut session = anigo_session::CoreSession::new();
        let id = anigo_core::AnimationClipId::from_label("Acenar");
        let new_clip = cards[0].asset.to_clip(id.clone(), session.project().animation.fps);
        session.apply(anigo_core::Command::EditAnimation { edit: anigo_core::animation::AnimEdit::PutClip { clip: new_clip } }).unwrap();
        assert_eq!(session.project().animation.clips[&id].key_count(), 2);
        session.undo().unwrap();
        assert!(session.project().animation.clips.is_empty());
    }

    #[test]
    fn a_clip_thumbnail_is_a_4_pose_strip_or_degrades_without_gpu() {
        let renderer = super::super::cel_thumb::CelThumbnailRenderer::new();
        let asset = ClipAsset::from_clip("Acenar", &clip(-100.0), 24.0).unwrap();
        match renderer.render_clip_strip(&asset, 128) {
            Ok(img) => {
                assert_eq!(img.dimensions(), (128, 128));
                let still = ClipAsset::from_clip("Parado", &clip(0.0), 24.0).unwrap();
                let rest = renderer.render_clip_strip(&still, 128).unwrap();
                assert_ne!(img.as_raw(), rest.as_raw(), "o clip muda a tira");
                if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = img.save(std::path::Path::new(&dir).join("clip_strip.png"));
                }
            }
            Err(e) => eprintln!("sem GPU: {e}"),
        }
    }
}
