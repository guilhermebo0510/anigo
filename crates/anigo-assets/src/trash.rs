//! Lixeira: soft delete, restaurar, purge. Ativos `linked` so tem o MANIFESTO movido/apagado: os arquivos
//! externos nunca sao tocados.

use std::collections::HashSet;

use rusqlite::params;

use crate::error::{AssetError, IoContext, Result};
use crate::library::{layout, read_manifest, write_manifest, Library};
use crate::model::*;
use crate::util::{dir_size, now_secs, yyyymmdd};

/// Opcoes de `trash`.
#[derive(Debug, Clone, Copy, Default)]
pub struct TrashOptions {
    /// Manda para a lixeira mesmo que outros ativos dependam deste.
    pub force: bool,
}

/// Resultado de `trash`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrashReport {
    pub trashed: Vec<AssetId>,
    /// Ativos NAO removidos porque outros dependem deles: `(ativo, dependentes)`.
    pub blocked: Vec<(AssetId, Vec<AssetId>)>,
    /// Ja estavam na lixeira.
    pub already: Vec<AssetId>,
}

/// Resultado de `purge`/`empty_trash`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PurgeReport {
    pub purged: Vec<AssetId>,
    pub bytes_freed: u64,
}

impl Library {
    /// Envia para a lixeira (reversivel). Ativos com dependentes ativos sao bloqueados, salvo `force`.
    pub fn trash(&self, ids: &[AssetId], opts: TrashOptions) -> Result<TrashReport> {
        let mut rep = TrashReport::default();
        let batch: HashSet<AssetId> = ids.iter().copied().collect();
        for id in ids {
            let _g = self.inner.mutate_lock.lock().map_err(|_| AssetError::Internal("lock de mutacao envenenado".into()))?;
            let loc = self.loc(id)?;
            if loc.state == AssetState::Trashed {
                rep.already.push(*id);
                continue;
            }
            if !opts.force {
                // dependentes ativos que NAO estao sendo removidos junto
                let users: Vec<AssetId> = self.used_by(id)?.into_iter().filter(|u| !batch.contains(u)).collect();
                if !users.is_empty() {
                    rep.blocked.push((*id, users));
                    continue;
                }
            }
            let now = now_secs();
            let base = self.manifest_base(loc.storage, loc.root_id)?;
            let src = base.join(&loc.manifest_dir);
            let rel = layout::trash_dir_rel(&yyyymmdd(now), id);
            let dst = base.join(&rel);
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).at(parent)?;
            }
            if dst.exists() {
                return Err(AssetError::Conflict(format!("destino da lixeira ja existe: {}", dst.display())));
            }
            std::fs::rename(&src, &dst).at(&src)?;
            let mut m = read_manifest(&dst)?;
            m.trashed_at = Some(now);
            m.modified_at = now;
            write_manifest(&dst, &m)?;
            self.inner.db.write(|c| {
                c.execute(
                    "UPDATE asset SET state = 1, manifest_dir = ?2, trashed_at = ?3, modified_at = ?3 WHERE id = ?1",
                    params![id.as_bytes().as_slice(), rel, now],
                )?;
                Ok(())
            })?;
            rep.trashed.push(*id);
        }
        Ok(rep)
    }

    /// Restaura ativos da lixeira para o lugar original.
    pub fn restore(&self, ids: &[AssetId]) -> Result<Vec<AssetId>> {
        let mut out = Vec::new();
        for id in ids {
            let _g = self.inner.mutate_lock.lock().map_err(|_| AssetError::Internal("lock de mutacao envenenado".into()))?;
            let loc = self.loc(id)?;
            if loc.state != AssetState::Trashed {
                continue;
            }
            let base = self.manifest_base(loc.storage, loc.root_id)?;
            let src = base.join(&loc.manifest_dir);
            let rel = match loc.storage {
                StorageMode::Managed => layout::managed_dir_rel(id),
                StorageMode::Linked => layout::linked_dir_rel(id),
            };
            let dst = base.join(&rel);
            if dst.exists() {
                return Err(AssetError::Conflict(format!("ja existe uma pasta em {}", dst.display())));
            }
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent).at(parent)?;
            }
            std::fs::rename(&src, &dst).at(&src)?;
            let mut m = read_manifest(&dst)?;
            m.trashed_at = None;
            m.modified_at = now_secs();
            write_manifest(&dst, &m)?;
            self.inner.db.write(|c| {
                c.execute(
                    "UPDATE asset SET state = 0, manifest_dir = ?2, trashed_at = NULL, modified_at = ?3 WHERE id = ?1",
                    params![id.as_bytes().as_slice(), rel, m.modified_at],
                )?;
                Ok(())
            })?;
            out.push(*id);
        }
        Ok(out)
    }

    /// Apaga DEFINITIVAMENTE ativos que ja estao na lixeira. Managed: remove a pasta com todos os arquivos.
    /// Linked: remove so o manifesto (arquivos externos intactos). Dependencias que apontavam para o ativo sao
    /// removidas dos manifestos dos dependentes.
    pub fn purge(&self, ids: &[AssetId]) -> Result<PurgeReport> {
        let mut rep = PurgeReport::default();
        for id in ids {
            let _g = self.inner.mutate_lock.lock().map_err(|_| AssetError::Internal("lock de mutacao envenenado".into()))?;
            let loc = self.loc(id)?;
            if loc.state != AssetState::Trashed {
                return Err(AssetError::InvalidInput(format!("{id} nao esta na lixeira; envie-o primeiro com trash()")));
            }
            let base = self.manifest_base(loc.storage, loc.root_id)?;
            let dir = base.join(&loc.manifest_dir);
            // so apaga dentro da lixeira (defesa contra manifest_dir adulterado no indice)
            if !loc.manifest_dir.starts_with("trash/") || loc.manifest_dir.contains("..") {
                return Err(AssetError::UnsafePath(format!("manifest_dir fora da lixeira: {}", loc.manifest_dir)));
            }
            let hash = self.row(id)?.map(|r| r.content_hash);
            let freed = dir_size(&dir);
            if dir.exists() {
                std::fs::remove_dir_all(&dir).at(&dir)?;
            }
            // limpa dependencias nos dependentes
            let users = self.used_by_filtered(id, true)?;
            for u in users {
                if self.row(&u)?.is_none() {
                    continue;
                }
                // dentro do lock atual: mutate_manifest tomaria o mesmo lock -> editar direto
                let uloc = self.loc(&u)?;
                let udir = self.loc_dir(&uloc)?;
                if let Ok(mut m) = read_manifest(&udir) {
                    let before = m.dependencies.len();
                    m.dependencies.retain(|d| d.id != *id);
                    if m.dependencies.len() != before {
                        m.modified_at = now_secs();
                        let _ = write_manifest(&udir, &m);
                    }
                }
            }
            self.inner.db.write(|c| {
                let tx = c.transaction()?;
                if let Some(seq) = crate::store::seq_of(&tx, id)? {
                    crate::store::delete_asset_rows(&tx, seq, id)?;
                }
                tx.execute("DELETE FROM collection_item WHERE asset_id = ?1", [id.as_bytes().as_slice()])?;
                tx.execute("DELETE FROM dep WHERE dep_id = ?1", [id.as_bytes().as_slice()])?;
                tx.commit()?;
                Ok(())
            })?;
            if let Some(h) = hash {
                // miniaturas orfas (se nenhum outro ativo usa o mesmo conteudo)
                if self.find_by_hash(&h)?.is_none() {
                    let _ = crate::thumbs::remove_cached(self.cache_dir(), &h);
                }
            }
            rep.bytes_freed += freed;
            rep.purged.push(*id);
        }
        self.write_collections_snapshot()?;
        Ok(rep)
    }

    /// Esvazia a lixeira. Com `older_than_secs`, apaga so o que esta la ha mais tempo.
    pub fn empty_trash(&self, older_than_secs: Option<u64>) -> Result<PurgeReport> {
        let cutoff = older_than_secs.map(|s| now_secs() - s as i64);
        let ids: Vec<AssetId> = self.inner.db.read(|c| {
            let mut st = c.prepare("SELECT id FROM asset WHERE state = 1 AND (?1 IS NULL OR trashed_at <= ?1)")?;
            let rows = st.query_map([cutoff], |r| r.get::<_, Vec<u8>>(0))?;
            Ok(rows.filter_map(|r| r.ok()).filter_map(|b| AssetId::from_bytes(&b)).collect())
        })?;
        self.purge(&ids)
    }
}
