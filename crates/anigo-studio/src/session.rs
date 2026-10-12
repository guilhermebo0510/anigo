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
    fn old_sessions_without_character_fields_still_load() {
        let mut v = serde_json::to_value(sample()).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("character_document");
        o.remove("character_camera");
        let back = SessionState::from_json(&v.to_string()).unwrap();
        assert!(back.character_document.is_none() && back.character_camera.is_none());
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
