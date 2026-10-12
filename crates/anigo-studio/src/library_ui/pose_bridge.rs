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
