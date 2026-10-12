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
            WorkspaceId::Personagem => icons::USER,
            WorkspaceId::Modelagem => icons::CUBE,
            WorkspaceId::Posing => icons::BONE,
            WorkspaceId::Shading => icons::PALETTE,
            WorkspaceId::Iluminacao => icons::SUN,
            WorkspaceId::Cenario => icons::MOUNTAINS,
            WorkspaceId::Animacao => icons::FILM_STRIP,
            WorkspaceId::Render => icons::CAMERA,
            WorkspaceId::Biblioteca => icons::SQUARES_FOUR,
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
    tool("body", icons::USER, "tool.body"),
    tool("face", icons::SMILEY, "tool.face"),
    tool("hair", icons::SCISSORS, "tool.hair"),
    tool("cloth", icons::T_SHIRT, "tool.cloth"),
    tool("accessories", icons::CUBE, "tool.accessories"),
    tool("paint", icons::PAINT_BRUSH, "tool.paint"),
];

/// Ferramentas do modo Objeto do workspace Modelagem.
pub const MODELAGEM_OBJECT: &[ToolDef] = &[
    tool("select_box", icons::SELECTION, "tool.select_box"),
    tool("move", icons::ARROWS_OUT_CARDINAL, "tool.move"),
    tool("rotate", icons::ARROWS_CLOCKWISE, "tool.rotate"),
    tool("scale", icons::ARROWS_OUT, "tool.scale"),
    tool("add_cube", icons::CUBE, "tool.add_primitive"),
];

pub const POSING: &[ToolDef] = &[
    tool("rig", icons::BONE, "tool.rig"),
    tool("ik", icons::TARGET, "tool.ik"),
    tool("poses_preset", icons::PERSON_SIMPLE_RUN, "tool.poses_preset"),
];

pub const SHADING: &[ToolDef] = &[
    tool("cel_shader", icons::CIRCLE, "tool.cel_shader"),
    tool("rim", icons::SPARKLE, "tool.rim"),
    tool("outline", icons::POLYGON, "tool.outline"),
    tool("palette", icons::PALETTE, "tool.palette"),
    tool("shader_ball", icons::SPHERE, "tool.shader_ball"),
];

pub const ILUMINACAO: &[ToolDef] = &[
    tool("sun", icons::SUN, "tool.sun"),
    tool("shadows", icons::CIRCLE, "tool.shadows"),
    tool("ambient", icons::CLOUD, "tool.ambient"),
    tool("rigs", icons::LIGHTBULB, "tool.rigs"),
];

/// Ferramentas do Cenário (esquema do Unreal): Q seleção, W mover, E girar, R escalar.
pub const CENARIO: &[ToolDef] = &[
    tool("select_box", icons::CURSOR, "tool.select_box"),
    tool("move", icons::ARROWS_OUT_CARDINAL, "tool.move"),
    tool("rotate", icons::ARROWS_CLOCKWISE, "tool.rotate"),
    tool("scale", icons::ARROWS_OUT, "tool.scale"),
];

pub const ANIMACAO: &[ToolDef] = &[
    tool("timeline", icons::CLOCK, "tool.timeline"),
    tool("curves", icons::BEZIER_CURVE, "tool.curves"),
    tool("lipsync", icons::SMILEY, "tool.lipsync"),
];

pub const RENDER: &[ToolDef] = &[
    tool("camera", icons::CAMERA, "tool.camera"),
    tool("passes", icons::STACK, "tool.passes"),
    tool("export", icons::FILM_REEL, "tool.export"),
];

pub const BIBLIOTECA: &[ToolDef] = &[
    tool("browser", icons::GRID_FOUR, "tool.browser"),
    tool("search", icons::MAGNIFYING_GLASS, "tool.search"),
    tool("filters", icons::SLIDERS, "tool.filters"),
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
