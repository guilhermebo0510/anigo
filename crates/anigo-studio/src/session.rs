//! Persistência da sessão do app (restaurar após reinício = "hot restart").
//!
//! Salva workspace, idioma, câmera e o documento de modelagem em JSON versionado.
//! Nunca é a fonte de verdade do projeto (isso é o `ProjectState`); é só estado de UI/edição.

use crate::i18n::Lang;
use crate::modeling::Document;
use crate::viewport::OrbitCamera;
use crate::workspace::WorkspaceId;
use serde::{Deserialize, Serialize};

pub const SESSION_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub version: u32,
    pub workspace: WorkspaceId,
    pub lang: Lang,
    pub camera: OrbitCamera,
    pub document: Document,
    /// Documento canônico do núcleo (`ProjectState` em JSON) do workspace Personagem.
    /// Opcional para sessões antigas continuarem abrindo.
    #[serde(default)]
    pub character_document: Option<String>,
    #[serde(default)]
    pub character_camera: Option<OrbitCamera>,
    /// Câmera de edição do workspace Cenário (o projeto da cena vive em `character_document`).
    #[serde(default)]
    pub scene_camera: Option<OrbitCamera>,
    /// Snap, ferramenta, sombreamento e painéis do Cenário.
    #[serde(default)]
    pub scene_prefs: Option<crate::app::ScenePrefs>,
    /// Pasta de saída, template de nome, contador e guias do workspace Render.
    #[serde(default)]
    pub render_prefs: Option<crate::app::RenderPrefs>,
    /// Layout dockável de cada workspace + presets (T-2.12). Opcional: sessões antigas abrem com o layout padrão.
    /// Tolerante: um bloco de layouts ilegível/de esquema futuro vira `None` (layout padrão) e NUNCA derruba a sessão.
    #[serde(default, deserialize_with = "lenient_layouts")]
    pub layouts: Option<crate::layout::LayoutStore>,
    /// Multi-vista (layout, divisórias, câmeras e vista ativa) por viewport: `modelagem`, `cenario`, `posing`, `iluminacao`.
    /// Tolerante como os layouts: um bloco ilegível vira "sem vistas salvas" e NUNCA derruba a sessão.
    #[serde(default, deserialize_with = "lenient_multiviews")]
    pub multiviews: std::collections::HashMap<String, crate::app::MultiViewState>,
}

fn lenient_multiviews<'de, D: serde::Deserializer<'de>>(d: D) -> Result<std::collections::HashMap<String, crate::app::MultiViewState>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    let Some(map) = v.as_object() else { return Ok(Default::default()) };
    let mut out = std::collections::HashMap::new();
    for (k, val) in map {
        match serde_json::from_value(val.clone()) {
            Ok(st) => {
                out.insert(k.clone(), st);
            }
            Err(e) => tracing::warn!("vistas de '{k}' ignoradas na sessão: {e}"),
        }
    }
    Ok(out)
}

fn lenient_layouts<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<crate::layout::LayoutStore>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    if v.is_null() {
        return Ok(None);
    }
    match crate::layout::LayoutStore::parse(&v.to_string()) {
        Ok(l) => {
            for w in &l.warnings {
                tracing::warn!("layout da sessão: {w}");
            }
            Ok(Some(l.store))
        }
        Err(e) => {
            tracing::warn!("layouts da sessão ignorados: {e}");
            Ok(None)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("versão de sessão {0} não suportada (esperado {SESSION_VERSION})")]
    Version(u32),
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
}

impl SessionState {
    pub fn to_json(&self) -> Result<String, SessionError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Carrega validando versão e a integridade das malhas (sessão corrompida nunca derruba o app).
    pub fn from_json(raw: &str) -> Result<Self, SessionError> {
        let mut s: SessionState = serde_json::from_str(raw)?;
        if s.version != SESSION_VERSION {
            return Err(SessionError::Version(s.version));
        }
        for o in &s.document.objects {
            o.mesh
                .validate()
                .map_err(|e| SessionError::Json(serde::de::Error::custom(format!("malha '{}' inválida: {e}", o.name))))?;
        }
        // penteado corrompido nunca derruba a sessão: as mechas são descartadas (as malhas dos grupos ficam)
        if let Err(e) = s.document.hair.validate() {
            tracing::warn!("cabelo da sessão ignorado: {e}");
            s.document.hair = Default::default();
        }
        Ok(s)
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<(), SessionError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // escrita atômica: grava ao lado e renomeia
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, self.to_json()?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn load_from(path: &std::path::Path) -> Result<Self, SessionError> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modeling::{ModelingSession, PrimitiveKind};

    fn sample() -> SessionState {
        let mut m = ModelingSession::with_default_cube();
        m.add_primitive(PrimitiveKind::Capsule, glam::Vec3::X);
        SessionState {
            version: SESSION_VERSION,
            workspace: WorkspaceId::Modelagem,
            lang: Lang::En,
            camera: OrbitCamera::default(),
            document: m.doc.clone(),
            character_document: None,
            character_camera: None,
            scene_camera: None,
            scene_prefs: None,
            render_prefs: None,
            layouts: None,
            multiviews: Default::default(),
        }
    }

    #[test]
    fn roundtrip_preserves_everything() {
        let s = sample();
        let back = SessionState::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(back.workspace, WorkspaceId::Modelagem);
        assert_eq!(back.lang, Lang::En);
        assert_eq!(back.camera, s.camera);
        assert_eq!(back.document.objects.len(), 2);
        assert_eq!(back.document.objects[1].mesh, s.document.objects[1].mesh);
    }

    #[test]
    fn multiviews_roundtrip_and_a_broken_block_never_breaks_the_session() {
        use crate::app::MultiViewState;
        let mut s = sample();
        let mut mv = crate::app::multiview_for_tests();
        mv.ratio = [0.3, 0.7];
        s.multiviews.insert("cenario".into(), mv);
        let back = SessionState::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(back.multiviews["cenario"].ratio, [0.3, 0.7]);
        // um workspace com bloco ilegível é ignorado; os outros e a sessão continuam
        let mut v = serde_json::to_value(&s).unwrap();
        v["multiviews"]["posing"] = serde_json::json!({ "layout": "dodecaedro" });
        v["multiviews"]["modelagem"] = serde_json::to_value::<MultiViewState>(s.multiviews["cenario"].clone()).unwrap();
        let back = SessionState::from_json(&v.to_string()).unwrap();
        assert!(back.multiviews.contains_key("modelagem") && back.multiviews.contains_key("cenario"));
        assert!(!back.multiviews.contains_key("posing"));
        // `multiviews` com tipo errado vira "sem vistas salvas"
        v["multiviews"] = serde_json::json!("lixo");
        assert!(SessionState::from_json(&v.to_string()).unwrap().multiviews.is_empty());
    }

    #[test]
    fn sessions_without_multiviews_still_load() {
        let mut v = serde_json::to_value(sample()).unwrap();
        v.as_object_mut().unwrap().remove("multiviews");
        assert!(SessionState::from_json(&v.to_string()).unwrap().multiviews.is_empty());
    }

    #[test]
    fn old_sessions_without_character_fields_still_load() {
        let mut v = serde_json::to_value(sample()).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("character_document");
        o.remove("character_camera");
        let back = SessionState::from_json(&v.to_string()).unwrap();
        assert!(back.character_document.is_none() && back.character_camera.is_none());
    }

    #[test]
    fn layouts_roundtrip_with_the_session_and_old_sessions_have_none() {
        use crate::layout::{registry, LayoutStore};
        let mut store = LayoutStore::default();
        let n = registry::default_layout(WorkspaceId::Cenario, (1234.0, 700.0), &|_| true);
        store.workspaces.insert("cenario".into(), n.clone());
        store.save_preset("meu layout", "cenario", n.clone());
        let mut s = sample();
        s.layouts = Some(store.clone());
        let back = SessionState::from_json(&s.to_json().unwrap()).unwrap();
        assert_eq!(back.layouts, Some(store));
        // sessão antiga (sem o campo) abre sem layouts
        let mut v = serde_json::to_value(sample()).unwrap();
        v.as_object_mut().unwrap().remove("layouts");
        assert!(SessionState::from_json(&v.to_string()).unwrap().layouts.is_none());
    }

    #[test]
    fn garbage_or_future_layouts_never_break_the_session() {
        let base = serde_json::to_value(sample()).unwrap();
        for bad in [
            serde_json::json!("lixo"),
            serde_json::json!([1, 2, 3]),
            serde_json::json!({"schema": 99, "workspaces": {}}),
            serde_json::json!({"schema": 1, "workspaces": {"cenario": {"kind": "bolo"}}}),
            serde_json::json!(42),
        ] {
            let mut v = base.clone();
            v["layouts"] = bad.clone();
            let s = SessionState::from_json(&v.to_string()).unwrap_or_else(|e| panic!("sessão quebrou com layouts={bad}: {e}"));
            // entradas ruins viram "sem layout"; o resto da sessão continua inteiro
            assert_eq!(s.workspace, WorkspaceId::Modelagem);
            assert!(s.layouts.is_none() || s.layouts.as_ref().is_some_and(|l| l.workspaces.is_empty()), "{bad}");
        }
    }

    #[test]
    fn character_document_roundtrips_with_the_session() {
        use anigo_session::CoreSession;
        let mut core = CoreSession::new();
        core.apply(anigo_core::command::Command::RenameProject { name: "Persistido".into() }).unwrap();
        let mut s = sample();
        s.character_document = Some(core.document().unwrap());
        let back = SessionState::from_json(&s.to_json().unwrap()).unwrap();
        let mut restored = CoreSession::new();
        restored.load_document(back.character_document.as_deref().unwrap()).unwrap();
        assert_eq!(restored.project(), core.project());
    }

    #[test]
    fn wrong_version_and_garbage_are_rejected() {
        let mut s = sample();
        s.version = 99;
        assert!(matches!(
            SessionState::from_json(&serde_json::to_string(&s).unwrap()),
            Err(SessionError::Version(99))
        ));
        assert!(SessionState::from_json("{ not json").is_err());
    }

    #[test]
    fn corrupted_mesh_index_is_rejected() {
        let mut s = sample();
        s.document.objects[0].mesh.faces[0].corners[0].vert = 9999;
        assert!(SessionState::from_json(&serde_json::to_string(&s).unwrap()).is_err());
    }

    #[test]
    fn hair_survives_the_session_and_a_corrupted_hairstyle_is_dropped_without_losing_the_scene() {
        use crate::modeling::hair::{HairState, ScriptStep};
        use crate::viewport::OrbitCamera;
        let mut m = ModelingSession::new();
        m.add_primitive(PrimitiveKind::UvSphere, glam::Vec3::new(0.0, 1.0, 0.0));
        m.set_mode(crate::modeling::Mode::Hair);
        let mut cam = OrbitCamera::default();
        cam.target = glam::Vec3::new(0.0, 1.0, 0.0);
        cam.distance = 4.0;
        let view = crate::modeling::sculpt::ViewInfo { camera: cam, size: glam::Vec2::new(1000.0, 800.0) };
        let pts: Vec<glam::Vec3> = (0..=12).map(|k| {
            let lat = 1.0 - 0.9 * k as f32 / 12.0;
            glam::Vec3::new(lat.cos() * 0.3f32.sin(), lat.sin(), lat.cos() * 0.3f32.cos())
        }).collect();
        HairState::new().run_script(&mut m, &view, &[ScriptStep::NewGroup("A"), ScriptStep::Draw(pts)]);
        assert_eq!(m.doc.hair.strand_count(), 1);
        let mut st = sample();
        st.document = m.doc.clone();
        let back = SessionState::from_json(&st.to_json().unwrap()).unwrap();
        assert_eq!(back.document.hair.strand_count(), 1);
        assert_eq!(back.document.objects.len(), m.doc.objects.len());
        // penteado corrompido (guia com NaN vira null no JSON): descarta só o cabelo
        let mut v = serde_json::to_value(&st).unwrap();
        v["document"]["hair"]["groups"][0]["strands"][0]["resolution"] = serde_json::json!(0);
        let loaded = SessionState::from_json(&v.to_string()).unwrap();
        assert!(loaded.document.hair.is_empty(), "cabelo inválido descartado");
        assert_eq!(loaded.document.objects.len(), m.doc.objects.len(), "a cena fica");
    }

    #[test]
    fn file_roundtrip_is_atomic_and_creates_dirs() {
        let dir = std::env::temp_dir().join(format!("anigo_sess_{}/nested", std::process::id()));
        let path = dir.join("session.json");
        sample().save_to(&path).unwrap();
        assert!(!path.with_extension("tmp").exists());
        assert!(SessionState::load_from(&path).is_ok());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }
}
