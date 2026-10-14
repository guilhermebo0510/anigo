//! Gerado por `brand/gen_icons.py` — NÃO editar à mão. Ícones próprios do ANIGO (fonte `AnigoIcons.ttf`, pontos de código
//! F100–F1FF). Cada ícone tem um trecho sólido do lado de baixo/direita (o "lado de sombra" do cel shading).

pub const WS_PERSONAGEM: &str = "\u{F100}";
pub const WS_MODELAGEM: &str = "\u{F101}";
pub const WS_POSING: &str = "\u{F102}";
pub const WS_SHADING: &str = "\u{F103}";
pub const WS_ILUMINACAO: &str = "\u{F104}";
pub const WS_CENARIO: &str = "\u{F105}";
pub const WS_ANIMACAO: &str = "\u{F106}";
pub const WS_RENDER: &str = "\u{F107}";
pub const WS_BIBLIOTECA: &str = "\u{F108}";
pub const T_BODY: &str = "\u{F10A}";
pub const T_FACE: &str = "\u{F10B}";
pub const T_HAIR: &str = "\u{F10C}";
pub const T_CLOTH: &str = "\u{F10D}";
pub const T_ACCESSORIES: &str = "\u{F10E}";
pub const T_PAINT: &str = "\u{F10F}";
pub const T_RIG: &str = "\u{F114}";
pub const T_IK: &str = "\u{F115}";
pub const T_POSE: &str = "\u{F116}";
pub const T_CEL_SHADER: &str = "\u{F11E}";
pub const T_RIM: &str = "\u{F11F}";
pub const T_OUTLINE: &str = "\u{F120}";
pub const T_PALETTE: &str = "\u{F121}";
pub const T_SHADER_BALL: &str = "\u{F122}";
pub const T_SUN: &str = "\u{F128}";
pub const T_SHADOWS: &str = "\u{F129}";
pub const T_AMBIENT: &str = "\u{F12A}";
pub const T_RIGS: &str = "\u{F12B}";
pub const T_SELECT: &str = "\u{F132}";
pub const T_MOVE: &str = "\u{F133}";
pub const T_ROTATE: &str = "\u{F134}";
pub const T_SCALE: &str = "\u{F135}";
pub const T_ADD_CUBE: &str = "\u{F136}";
pub const T_TIMELINE: &str = "\u{F13C}";
pub const T_CURVES: &str = "\u{F13D}";
pub const T_CLIPS: &str = "\u{F13E}";
pub const T_LENS: &str = "\u{F146}";
pub const T_PASSES: &str = "\u{F147}";
pub const T_EXPORT: &str = "\u{F148}";
pub const T_GRID: &str = "\u{F150}";
pub const T_SEARCH: &str = "\u{F151}";
pub const T_FILTER: &str = "\u{F152}";

/// Todos os ícones (nome, glifo) — usado pelos testes e pela folha de contato.
pub const ALL: &[(&str, &str)] = &[
    ("ws_personagem", WS_PERSONAGEM),
    ("ws_modelagem", WS_MODELAGEM),
    ("ws_posing", WS_POSING),
    ("ws_shading", WS_SHADING),
    ("ws_iluminacao", WS_ILUMINACAO),
    ("ws_cenario", WS_CENARIO),
    ("ws_animacao", WS_ANIMACAO),
    ("ws_render", WS_RENDER),
    ("ws_biblioteca", WS_BIBLIOTECA),
    ("t_body", T_BODY),
    ("t_face", T_FACE),
    ("t_hair", T_HAIR),
    ("t_cloth", T_CLOTH),
    ("t_accessories", T_ACCESSORIES),
    ("t_paint", T_PAINT),
    ("t_rig", T_RIG),
    ("t_ik", T_IK),
    ("t_pose", T_POSE),
    ("t_cel_shader", T_CEL_SHADER),
    ("t_rim", T_RIM),
    ("t_outline", T_OUTLINE),
    ("t_palette", T_PALETTE),
    ("t_shader_ball", T_SHADER_BALL),
    ("t_sun", T_SUN),
    ("t_shadows", T_SHADOWS),
    ("t_ambient", T_AMBIENT),
    ("t_rigs", T_RIGS),
    ("t_select", T_SELECT),
    ("t_move", T_MOVE),
    ("t_rotate", T_ROTATE),
    ("t_scale", T_SCALE),
    ("t_add_cube", T_ADD_CUBE),
    ("t_timeline", T_TIMELINE),
    ("t_curves", T_CURVES),
    ("t_clips", T_CLIPS),
    ("t_lens", T_LENS),
    ("t_passes", T_PASSES),
    ("t_export", T_EXPORT),
    ("t_grid", T_GRID),
    ("t_search", T_SEARCH),
    ("t_filter", T_FILTER),
];
