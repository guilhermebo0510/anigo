//! Ponte entre o workspace Posing e a Biblioteca: salvar a pose como ativo `Pose` (`.anigopose.json`) e listar/ler
//! os ativos de pose para aplicá-los. Mesma ideia do `import_authored` do GLB: mesmo nome = nova revisão.

use super::*;
use anigo_core::pose::{PoseAsset, POSE_ASSET_SUFFIX};

/// Um ativo de pose lido da Biblioteca.
#[derive(Debug, Clone)]
pub struct PoseCard {
    pub id: AssetId,
    pub name: String,
    pub revision: u32,
    pub asset: PoseAsset,
}

impl LibraryWorkspace {
    /// Grava `asset` como ativo `Pose` autoral (tags `pose`, `autoral`, `posing`). Se já existe um ativo `pose`
    /// com o mesmo nome, o conteúdo novo entra como NOVA REVISÃO.
    pub fn save_pose_asset(&mut self, asset: &PoseAsset) -> Result<String, String> {
        self.open_blocking()?;
        let rt = self.rt.as_ref().ok_or("a biblioteca não está aberta")?;
        let lib = rt.lib.clone();
        let name = asset.name.trim();
        if name.is_empty() {
            return Err("a pose precisa de um nome".into());
        }
        let json = asset.to_json().map_err(|e| e.to_string())?;
        let dir = std::env::temp_dir().join(format!("anigo-pose-{}-{}", std::process::id(), blake3::hash(name.as_bytes()).to_hex()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '_' }).collect();
        let file = dir.join(format!("{}{POSE_ASSET_SUFFIX}", safe.trim()));
        let result = (|| {
            std::fs::write(&file, json).map_err(|e| e.to_string())?;
            let q = Query { text: Some(name.to_string()), kinds: vec![AssetKind::Pose], limit: 50, ..Default::default() };
            let existing = lib.query(&q).ok().and_then(|p| p.items.into_iter().find(|r| r.name == name && r.state == AssetState::Active)).map(|r| r.id);
            let mut tags = vec!["pose".to_string(), "autoral".to_string(), "posing".to_string()];
            tags.extend(asset.tags.iter().cloned());
            let opts = ImportOptions { name: Some(name.to_string()), tags, kind: Some(AssetKind::Pose), author: Some("ANIGO Studio".into()), replace: existing, ..Default::default() };
            let rep = lib.import_blocking(&file, opts).map_err(|e| e.to_string())?;
            if rep.imported.first().is_some() {
                Ok(format!("pose \"{name}\" salva na Biblioteca"))
            } else if rep.revised.first().is_some() {
                Ok(format!("pose \"{name}\": nova revisão"))
            } else if rep.duplicates.first().is_some() {
                Ok(format!("pose \"{name}\": já existe com o mesmo conteúdo"))
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

    /// Ativos `Pose` ativos da Biblioteca (mais recentes primeiro), já lidos e validados. Arquivos ilegíveis são ignorados.
    pub fn list_pose_assets(&mut self, limit: usize) -> Result<Vec<PoseCard>, String> {
        self.open_blocking()?;
        let lib = self.rt.as_ref().ok_or("a biblioteca não está aberta")?.lib.clone();
        let q = Query { kinds: vec![AssetKind::Pose], limit: limit.max(1), ..Default::default() };
        let page = lib.query(&q).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for row in page.items {
            if row.state != AssetState::Active {
                continue;
            }
            let Ok(files) = lib.resolve_files(&row.id) else { continue };
            let Ok(bytes) = std::fs::read(&files.primary) else { continue };
            if let Ok(asset) = PoseAsset::from_json(&bytes) {
                out.push(PoseCard { id: row.id, name: row.name.clone(), revision: row.revision, asset });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_core::pose::{bone, PoseRig, PoseState};
    use glam::Quat;

    fn pose_with(elbow_deg: f32) -> PoseState {
        let mut p = PoseState::default();
        p.set_rotation(bone::L_ELBOW, Quat::from_rotation_x(-elbow_deg.to_radians()));
        p.set_rotation(bone::HEAD, Quat::from_rotation_y(0.3));
        p
    }

    #[test]
    fn pose_assets_are_saved_listed_revised_and_masked_through_the_library() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open(dir.path().join("lib")).unwrap();
        let mut ws = LibraryWorkspace::with_library(lib.clone());
        // vazio no começo
        assert!(ws.list_pose_assets(50).unwrap().is_empty());
        // salvar: vira ativo `pose` com as tags certas
        let rig = PoseRig::default();
        let asset = PoseAsset::from_mask("Saudação", &pose_with(80.0), &[bone::L_ELBOW].into_iter().collect(), &rig);
        let msg = ws.save_pose_asset(&asset).unwrap();
        assert!(msg.contains("salva"), "{msg}");
        let rows = lib.query(&Query { kinds: vec![AssetKind::Pose], ..Default::default() }).unwrap().items;
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].name.as_str(), rows[0].format.as_str()), ("Saudação", "anigopose"));
        assert!(lib.tags_of(&rows[0].id).unwrap().contains(&"pose".to_string()));
        // listar devolve o ativo lido do disco, com a máscara
        let cards = ws.list_pose_assets(50).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].asset.bones, vec!["LeftElbow"]);
        assert!(cards[0].asset.pose.rotation(bone::HEAD).dot(Quat::IDENTITY).abs() > 0.9999, "só o cotovelo foi salvo");
        // mesmo conteúdo: não duplica; conteúdo novo: mesma identidade, revisão 2
        let again = ws.save_pose_asset(&asset).unwrap();
        assert!(again.contains("já existe") || again.contains("revisão"), "{again}");
        let changed = PoseAsset::from_mask("Saudação", &pose_with(120.0), &[bone::L_ELBOW].into_iter().collect(), &rig);
        assert!(ws.save_pose_asset(&changed).unwrap().contains("revisão"));
        let cards = ws.list_pose_assets(50).unwrap();
        assert_eq!(cards.len(), 1, "ainda um ativo só");
        assert_eq!(cards[0].revision, 2);
        // nome vazio é recusado; pose inteira vira máscara vazia (todos os ossos)
        let nameless = PoseAsset::new("  ", pose_with(10.0));
        assert!(ws.save_pose_asset(&nameless).is_err());
        let full = PoseAsset::new("Corpo inteiro", pose_with(10.0));
        ws.save_pose_asset(&full).unwrap();
        let cards = ws.list_pose_assets(50).unwrap();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards.iter().find(|c| c.name == "Corpo inteiro").unwrap().asset.mask().len(), 24);
        // aplicar um ativo da Biblioteca vira um comando desfazível no núcleo
        let card = cards.iter().find(|c| c.name == "Saudação").unwrap();
        let mut session = anigo_session::CoreSession::new();
        let target = card.asset.apply_to(session.pose(), 1.0, true, None);
        let cmd = anigo_core::pose::plan_pose_change(session.pose(), &target, anigo_core::pose::PoseOp::Apply).unwrap();
        session.apply(cmd).unwrap();
        assert!(session.pose().rotation(bone::R_ELBOW).dot(Quat::IDENTITY).abs() < 0.9999, "espelhado: cotovelo D dobrado");
        session.undo().unwrap();
        assert!(session.pose().is_rest());
    }

    #[test]
    fn a_pose_thumbnail_renders_the_posed_character_or_degrades_without_gpu() {
        let renderer = super::super::cel_thumb::CelThumbnailRenderer::new();
        match renderer.render_pose(&pose_with(90.0), 96) {
            Ok(img) => {
                let rest = renderer.render_pose(&PoseState::default(), 96).unwrap();
                assert_ne!(img.as_raw(), rest.as_raw(), "a pose muda a miniatura");
                if let Ok(dir) = std::env::var("ANIGO_TEST_OUT") {
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = img.save(format!("{dir}/pose_thumb.png"));
                    let _ = renderer.render_pose(&anigo_core::pose::presets::body("action", &PoseRig::default()).unwrap(), 256).map(|i| i.save(format!("{dir}/pose_thumb_action.png")));
                }
                let lit = img.pixels().filter(|p| p.0[0] as u32 + p.0[1] as u32 + p.0[2] as u32 > 60).count();
                assert!(lit > 500, "personagem visível: {lit} px");
            }
            Err(e) => assert!(!e.is_empty(), "sem GPU a falha tem mensagem"),
        }
    }
}
