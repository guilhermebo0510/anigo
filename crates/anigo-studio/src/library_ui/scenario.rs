//! Cenários de screenshot da Biblioteca (`--scenario biblioteca-*`): constroem uma biblioteca temporária
//! SINTÉTICA grande (mais ativos reais gerados pelo próprio studio) e levam a UI a um estado conhecido.
//! O screenshot só é tirado quando a UI assentou (consulta, miniaturas, jobs) — ver `is_done`.

use super::synth::{self, ScenarioAssets};
use super::*;

pub const SCENARIOS: &[&str] = &[
    "biblioteca-grid",
    "biblioteca-lista",
    "biblioteca-detalhes",
    "biblioteca-3d",
    "biblioteca-busca",
    "biblioteca-import",
    "biblioteca-gestao",
    "biblioteca-duplicados",
    "biblioteca-lixeira",
    "biblioteca-ausentes",
];

pub fn is_library_scenario(name: &str) -> bool {
    name.starts_with("biblioteca-")
}

/// Quantos ativos sintéticos (padrão 6 000 para o CI; `ANIGO_SCENARIO_N=50000` para o teste de escala).
pub fn synthetic_count() -> usize {
    std::env::var("ANIGO_SCENARIO_N").ok().and_then(|v| v.parse().ok()).unwrap_or(6000)
}

pub struct Built {
    pub lib: Library,
    pub assets: ScenarioAssets,
    pub dir: PathBuf,
}

/// Cria (do zero) a biblioteca temporária do cenário.
pub fn build(name: &str) -> Result<Built, String> {
    let dir = std::env::var_os("ANIGO_SCENARIO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("anigo-lib-scenario-{}-{}", std::process::id(), name.trim_start_matches("biblioteca-"))));
    let _ = std::fs::remove_dir_all(&dir);
    let n = synthetic_count();
    let t = Instant::now();
    let lib = synth::build_synthetic(&dir.join("lib"), n, 42).map_err(|e| e.to_string())?;
    let assets = synth::populate_scenario(&lib, &dir.join("work")).map_err(|e| e.to_string())?;
    eprintln!("cenário {name}: {n} ativos sintéticos + reais em {:.1}s ({})", t.elapsed().as_secs_f32(), dir.display());
    Ok(Built { lib, assets, dir })
}

pub struct Run {
    pub name: String,
    pub assets: ScenarioAssets,
    stage: u32,
    stage_frame: u64,
    pub done: bool,
}

impl Run {
    pub fn new(name: &str, assets: ScenarioAssets) -> Self {
        Self { name: name.to_string(), assets, stage: 0, stage_frame: 0, done: false }
    }
}

impl LibraryWorkspace {
    pub fn start_scenario(&mut self, name: &str, assets: ScenarioAssets) {
        self.scenario = Some(Run::new(name, assets));
    }

    /// O cenário terminou de montar e a UI assentou (hora do screenshot).
    pub fn scenario_done(&self) -> bool {
        self.scenario.as_ref().is_none_or(|r| r.done)
    }

    /// Avança a máquina de estados do cenário (chamada a cada quadro).
    pub(super) fn scenario_tick(&mut self) {
        let Some(mut run) = self.scenario.take() else { return };
        if run.done || !self.is_ready() {
            self.scenario = Some(run);
            return;
        }
        let settled = self.is_settled();
        let waited = self.frame.saturating_sub(run.stage_frame);
        let mut next = false;
        match run.stage {
            // 0: ordenação por "mais recentes" (os ativos reais ficam no topo) e tipo de visão
            0 => {
                self.filters.sort = Sort { by: SortBy::ImportedAt, desc: true };
                self.filters.user_sorted = true;
                match run.name.as_str() {
                    "biblioteca-lista" => self.view = ViewMode::List,
                    "biblioteca-lixeira" => self.filters.section = Section::Trash,
                    "biblioteca-ausentes" => self.filters.section = Section::Missing,
                    "biblioteca-busca" => {
                        self.search_text = "noir -esfera".into();
                        self.filters.text = self.search_text.clone();
                        self.filters.kinds = vec![AssetKind::Model, AssetKind::Character, AssetKind::Accessory];
                        self.filters.user_sorted = false;
                        self.filters.sort = Sort::default();
                    }
                    _ => {}
                }
                self.reload_rows(true);
                self.reload_facets();
                next = true;
            }
            // 1: espera a lista e as miniaturas
            1 => {
                if settled && waited > 3 {
                    next = true;
                }
            }
            // 2: ação específica do cenário
            2 => {
                match run.name.as_str() {
                    "biblioteca-detalhes" | "biblioteca-3d" => {
                        if let Some(id) = run.assets.models.first().copied() {
                            self.selection.set_one(id, 0);
                            self.sync_detail_target();
                            self.preview3d.enabled = run.name == "biblioteca-3d";
                        }
                    }
                    "biblioteca-grid" | "biblioteca-lista" => {
                        // seleção múltipla visível no screenshot
                        for (i, id) in run.assets.models.iter().take(3).enumerate() {
                            self.selection.ids.insert(*id);
                            self.selection.primary = Some(*id);
                            self.selection.cursor = Some(i);
                        }
                        self.sync_detail_target();
                    }
                    "biblioteca-import" => {
                        self.jobs.log.clear();
                        let dir = run.assets.import_dir.clone();
                        self.queue_import(vec![dir], ImportOptions { recursive: true, ..Default::default() });
                        self.jobs.log.open = true;
                    }
                    "biblioteca-gestao" => {
                        self.dlg.manage_open = true;
                        self.dlg.disk_busy = true;
                        self.spawn_task(|lib| TaskOut::DiskUsage(dialogs::measure_disk(&lib)));
                        self.start_verify(VerifyOptions { rehash: false, fix: false });
                    }
                    "biblioteca-duplicados" => self.open_duplicates(),
                    "biblioteca-ausentes" => {
                        if let Some(id) = run.assets.linked_missing {
                            self.selection.set_one(id, 0);
                            self.sync_detail_target();
                        }
                    }
                    _ => {}
                }
                next = true;
            }
            // 3: espera os efeitos (jobs, detalhe, preview 3D) assentarem
            3 => {
                let preview_ok = run.name != "biblioteca-3d" || self.preview3d.is_shown();
                let dups_ok = run.name != "biblioteca-duplicados" || self.dlg.dups.is_some();
                let disk_ok = run.name != "biblioteca-gestao" || self.dlg.disk.is_some();
                if settled && preview_ok && dups_ok && disk_ok && waited > 4 {
                    next = true;
                }
            }
            _ => {
                if waited > 3 {
                    run.done = true;
                }
            }
        }
        if next {
            run.stage += 1;
            run.stage_frame = self.frame;
        }
        // limite de segurança: nunca trava o screenshot
        if self.frame > 4000 {
            run.done = true;
        }
        self.scenario = Some(run);
    }
}
