//! Os 9 workspaces do ANIGO e a tabela de ferramentas de cada um.

use crate::icons;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum WorkspaceId {
    #[default]
    Personagem,
    Modelagem,
    Posing,
    Shading,
    Iluminacao,
    Cenario,
    Animacao,
    Render,
    Biblioteca,
}

impl WorkspaceId {
    /// Ordem canônica da barra superior.
    pub const ALL: [WorkspaceId; 9] = [
        WorkspaceId::Personagem,
        WorkspaceId::Modelagem,
        WorkspaceId::Posing,
        WorkspaceId::Shading,
        WorkspaceId::Iluminacao,
        WorkspaceId::Cenario,
        WorkspaceId::Animacao,
        WorkspaceId::Render,
        WorkspaceId::Biblioteca,
    ];

    /// Identificador estável (mesmos ids do app web: `personagem`, `iluminacao`…).
    pub fn key(self) -> &'static str {
        match self {
            WorkspaceId::Personagem => "personagem",
            WorkspaceId::Modelagem => "modelagem",
            WorkspaceId::Posing => "posing",
            WorkspaceId::Shading => "shading",
            WorkspaceId::Iluminacao => "iluminacao",
            WorkspaceId::Cenario => "cenario",
            WorkspaceId::Animacao => "animacao",
            WorkspaceId::Render => "render",
            WorkspaceId::Biblioteca => "biblioteca",
        }
    }

    pub fn from_key(key: &str) -> Option<WorkspaceId> {
        Self::ALL.into_iter().find(|w| w.key() == key)
    }

    pub fn icon(self) -> &'static str {
        match self {
            WorkspaceId::Personagem => icons::ani::WS_PERSONAGEM,
            WorkspaceId::Modelagem => icons::ani::WS_MODELAGEM,
            WorkspaceId::Posing => icons::ani::WS_POSING,
            WorkspaceId::Shading => icons::ani::WS_SHADING,
            WorkspaceId::Iluminacao => icons::ani::WS_ILUMINACAO,
            WorkspaceId::Cenario => icons::ani::WS_CENARIO,
            WorkspaceId::Animacao => icons::ani::WS_ANIMACAO,
            WorkspaceId::Render => icons::ani::WS_RENDER,
            WorkspaceId::Biblioteca => icons::ani::WS_BIBLIOTECA,
        }
    }

    /// A Biblioteca não tem viewport 3D (Asset Browser em grade).
    pub fn has_viewport(self) -> bool {
        self != WorkspaceId::Biblioteca
    }

    pub fn tools(self) -> &'static [ToolDef] {
        match self {
            WorkspaceId::Personagem => PERSONAGEM,
            WorkspaceId::Modelagem => MODELAGEM_OBJECT, // as ferramentas de edit/sculpt vêm do modo
            WorkspaceId::Posing => POSING,
            WorkspaceId::Shading => SHADING,
            WorkspaceId::Iluminacao => ILUMINACAO,
            WorkspaceId::Cenario => CENARIO,
            WorkspaceId::Animacao => ANIMACAO,
            WorkspaceId::Render => RENDER,
            WorkspaceId::Biblioteca => BIBLIOTECA,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDef {
    /// Id estável (compatível com os ids do app web onde existirem).
    pub id: &'static str,
    pub icon: &'static str,
    /// Chave de i18n do rótulo (`tool.<id>`).
    pub label_key: &'static str,
}

const fn tool(id: &'static str, icon: &'static str, label_key: &'static str) -> ToolDef {
    ToolDef { id, icon, label_key }
}

pub const PERSONAGEM: &[ToolDef] = &[
    tool("body", icons::ani::T_BODY, "tool.body"),
    tool("face", icons::ani::T_FACE, "tool.face"),
    tool("hair", icons::ani::T_HAIR, "tool.hair"),
    tool("cloth", icons::ani::T_CLOTH, "tool.cloth"),
    tool("accessories", icons::ani::T_ACCESSORIES, "tool.accessories"),
    tool("paint", icons::ani::T_PAINT, "tool.paint"),
];

/// Ferramentas do modo Objeto do workspace Modelagem.
pub const MODELAGEM_OBJECT: &[ToolDef] = &[
    tool("select_box", icons::ani::T_SELECT, "tool.select_box"),
    tool("move", icons::ani::T_MOVE, "tool.move"),
    tool("rotate", icons::ani::T_ROTATE, "tool.rotate"),
    tool("scale", icons::ani::T_SCALE, "tool.scale"),
    tool("add_cube", icons::ani::T_ADD_CUBE, "tool.add_primitive"),
];

pub const POSING: &[ToolDef] = &[
    tool("rig", icons::ani::T_RIG, "tool.rig"),
    tool("ik", icons::ani::T_IK, "tool.ik"),
    tool("poses_preset", icons::ani::T_POSE, "tool.poses_preset"),
];

pub const SHADING: &[ToolDef] = &[
    tool("cel_shader", icons::ani::T_CEL_SHADER, "tool.cel_shader"),
    tool("rim", icons::ani::T_RIM, "tool.rim"),
    tool("outline", icons::ani::T_OUTLINE, "tool.outline"),
    tool("palette", icons::ani::T_PALETTE, "tool.palette"),
    tool("shader_ball", icons::ani::T_SHADER_BALL, "tool.shader_ball"),
];

pub const ILUMINACAO: &[ToolDef] = &[
    tool("sun", icons::ani::T_SUN, "tool.sun"),
    tool("shadows", icons::ani::T_SHADOWS, "tool.shadows"),
    tool("ambient", icons::ani::T_AMBIENT, "tool.ambient"),
    tool("rigs", icons::ani::T_RIGS, "tool.rigs"),
];

/// Ferramentas do Cenário (esquema do Unreal): Q seleção, W mover, E girar, R escalar.
pub const CENARIO: &[ToolDef] = &[
    tool("select_box", icons::ani::T_SELECT, "tool.select_box"),
    tool("move", icons::ani::T_MOVE, "tool.move"),
    tool("rotate", icons::ani::T_ROTATE, "tool.rotate"),
    tool("scale", icons::ani::T_SCALE, "tool.scale"),
];

pub const ANIMACAO: &[ToolDef] = &[
    tool("timeline", icons::ani::T_TIMELINE, "tool.timeline"),
    tool("curves", icons::ani::T_CURVES, "tool.curves"),
    tool("clips", icons::ani::T_CLIPS, "tool.clips"),
];

pub const RENDER: &[ToolDef] = &[
    tool("camera", icons::ani::T_LENS, "tool.camera"),
    tool("passes", icons::ani::T_PASSES, "tool.passes"),
    tool("export", icons::ani::T_EXPORT, "tool.export"),
];

pub const BIBLIOTECA: &[ToolDef] = &[
    tool("browser", icons::ani::T_GRID, "tool.browser"),
    tool("search", icons::ani::T_SEARCH, "tool.search"),
    tool("filters", icons::ani::T_FILTER, "tool.filters"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nine_workspaces_in_canonical_order() {
        let keys: Vec<_> = WorkspaceId::ALL.iter().map(|w| w.key()).collect();
        assert_eq!(
            keys,
            [
                "personagem", "modelagem", "posing", "shading", "iluminacao", "cenario", "animacao",
                "render", "biblioteca"
            ]
        );
    }

    #[test]
    fn keys_roundtrip_and_are_unique() {
        for w in WorkspaceId::ALL {
            assert_eq!(WorkspaceId::from_key(w.key()), Some(w));
        }
        assert_eq!(WorkspaceId::from_key("nope"), None);
    }

    #[test]
    fn only_library_lacks_viewport() {
        for w in WorkspaceId::ALL {
            assert_eq!(w.has_viewport(), w != WorkspaceId::Biblioteca);
        }
    }

    #[test]
    fn every_workspace_has_tools_with_unique_ids() {
        for w in WorkspaceId::ALL {
            let tools = w.tools();
            assert!(!tools.is_empty(), "{:?}", w);
            let mut ids: Vec<_> = tools.iter().map(|t| t.id).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), tools.len(), "ids repetidos em {:?}", w);
        }
    }
}
