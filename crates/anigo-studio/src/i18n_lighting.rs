//! Textos do workspace Iluminação (pt-BR / en). Chaves `light.*` (+ `tool.rigs`); o teste confere que toda chave
//! usada no código existe aqui, que cada rig tem rótulo e dica e que não há duplicata nem texto vazio.

/// (chave, pt-BR, en)
pub const LIGHT_TABLE: &[(&str, &str, &str)] = &[
    ("tool.rigs", "Rigs de luz", "Light rigs"),
    ("light.title", "Iluminação", "Lighting"),
    // seções
    ("light.section.rigs", "Rigs de luz", "Light rigs"),
    ("light.section.sun", "Sol (luz principal)", "Sun (main light)"),
    ("light.section.shadows", "Sombras", "Shadows"),
    ("light.section.ambient", "Ambiente e céu", "Ambient & sky"),
    ("light.section.rim", "Rim light (personagem)", "Rim light (character)"),
    ("light.section.extra", "Luzes extras", "Extra lights"),
    ("light.section.test", "Cena de prova", "Test scene"),
    // rigs
    ("light.rigs.hint", "Um rig é um passo de desfazer: sol, ambiente (fill) e rim do personagem de uma vez.", "A rig is one undo step: sun, ambient (fill) and the character rim at once."),
    ("light.rig.three_point", "Três pontos", "Three-point"),
    ("light.rig.three_point.tip", "Key a 40° à esquerda e 35° acima, fill = ambiente (contraste ~3:1 entre luz e sombra), rim do material.", "Key 40° left and 35° up, fill = ambient (about 3:1 light-to-shadow contrast), material rim."),
    ("light.rig.rim", "Contraluz / rim", "Backlight / rim"),
    ("light.rig.rim.tip", "Sol atrás do personagem: contorno brilhante forte e ambiente alto para a frente não apagar.", "Sun behind the character: strong bright outline and high ambient so the front does not black out."),
    ("light.rig.day", "Dia", "Day"),
    ("light.rig.day.tip", "Sol alto e quase branco, céu azul, sombra levemente fria.", "High, near-white sun, blue sky, slightly cool shadows."),
    ("light.rig.golden_hour", "Golden hour", "Golden hour"),
    ("light.rig.golden_hour.tip", "Sol baixo e quente (3200 K), céu alaranjado, sombra violeta.", "Low warm sun (3200 K), orange sky, violet shadows."),
    ("light.rig.night", "Noite", "Night"),
    ("light.rig.night.tip", "Luar frio e fraco, ambiente azul-escuro, rim azulado.", "Weak cool moonlight, dark blue ambient, bluish rim."),
    ("light.rig.interior", "Interior", "Interior"),
    ("light.rig.interior.tip", "Luz quente e macia de lâmpada, ambiente alto (contraste baixo).", "Warm, soft lamp light, high ambient (low contrast)."),
    ("light.rig.studio", "Estúdio", "Studio"),
    ("light.rig.studio.tip", "Key neutra a 35° com fill moderado e rim frio: o visual padrão de ficha de personagem.", "Neutral key at 35° with moderate fill and a cool rim: the standard character sheet look."),
    ("light.rig_applied", "Rig aplicado", "Rig applied"),
    // luz presa à câmera
    ("light.follow_camera", "Luz presa à câmera", "Light attached to camera"),
    ("light.follow_camera_tip", "Preview: a luz acompanha a câmera (studio light). NÃO altera o projeto até você aplicar.", "Preview: the light follows the camera (studio light). It does NOT change the project until you apply it."),
    ("light.follow_hint", "Os ângulos abaixo são relativos à câmera; o projeto continua com a luz anterior.", "The angles below are relative to the camera; the project keeps the previous light."),
    ("light.follow_apply", "Aplicar ao projeto", "Apply to project"),
    ("light.follow_apply_tip", "Grava a direção atual da luz (no mundo) no projeto: um passo de desfazer.", "Writes the current light direction (in world space) to the project: one undo step."),
    ("light.relative", "(câmera)", "(camera)"),
    ("light.relative_hint", "Ângulos relativos à câmera: 0° = luz na câmera, + = à direita.", "Camera-relative angles: 0° = light at the camera, + = to the right."),
    // sol
    ("light.azimuth", "Azimute", "Azimuth"),
    ("light.elevation", "Elevação", "Elevation"),
    ("light.az_short", "Az", "Az"),
    ("light.el_short", "El", "El"),
    ("light.az_tip", "0° = luz vinda da frente do personagem; + gira para a direita da tela. Arraste o disco do sol na viewport.", "0° = light from the character's front; + turns to the screen right. Drag the sun disk in the viewport."),
    ("light.color", "Cor da luz", "Light color"),
    ("light.temperature", "Temperatura", "Temperature"),
    ("light.intensity", "Intensidade", "Intensity"),
    ("light.reset_sun", "Restaurar sol", "Reset sun"),
    // sombras
    ("light.terminator", "Terminador (material do personagem)", "Terminator (character material)"),
    ("light.threshold", "Limiar", "Threshold"),
    ("light.penumbra", "Penumbra", "Penumbra"),
    ("light.terminator_tip", "Onde a luz vira sombra e quão dura é a borda. Vale para o personagem; os objetos do cenário usam o material padrão.", "Where light turns to shadow and how hard the edge is. Applies to the character; scene objects use the default material."),
    ("light.shadow_tint_title", "Cor da sombra", "Shadow color"),
    ("light.shadow_tint", "Tinta da sombra", "Shadow tint"),
    ("light.shadow_sat", "Saturação", "Saturation"),
    ("light.shadow_tint_tip", "Multiplica a cor de sombra do material (branco = a cor do material, sem tinta extra).", "Multiplies the material's shadow color (white = the material color, no extra tint)."),
    ("light.cast", "Sombra projetada", "Cast shadow"),
    ("light.cast_on", "Ligada", "Enabled"),
    ("light.cast_tip", "Shadow map de 1 luz (estilizado). Desligada por padrão: o quadro é idêntico ao do motor anterior.", "1-light shadow map (stylized). Off by default: the frame is identical to the previous engine."),
    ("light.cast_strength", "Força", "Strength"),
    ("light.cast_hardness", "Dureza", "Hardness"),
    ("light.cast_bias", "Bias", "Bias"),
    ("light.cast_normal_bias", "Bias de normal", "Normal bias"),
    ("light.cast_res", "Resolução", "Resolution"),
    ("light.cast_distance", "Cobertura", "Coverage"),
    ("light.auto", "automática", "automatic"),
    ("light.cast_note", "A sombra do cabelo/cabeça no rosto vem deste mesmo mapa (auto-sombra). Bias alto some com a acne mas descola a sombra.", "Hair/head shadow on the face comes from this same map (self-shadowing). A high bias hides acne but detaches the shadow."),
    ("light.cast_unsupported", "Este backend gráfico (GL) não suporta o passe de sombra.", "This graphics backend (GL) does not support the shadow pass."),
    // ambiente
    ("light.ambient", "Ambiente", "Ambient"),
    ("light.sky", "Cor do céu", "Sky color"),
    ("light.ground", "Cor do chão", "Ground color"),
    ("light.hemi_tip", "Hemisfério: faces voltadas para cima recebem a cor do céu, para baixo a do chão. É o \"fill\" do rig.", "Hemisphere: up-facing surfaces get the sky color, down-facing the ground color. It is the rig's \"fill\"."),
    // rim
    ("light.rim_intensity", "Intensidade", "Intensity"),
    ("light.rim_spread", "Abertura", "Spread"),
    ("light.rim_color", "Cor do rim", "Rim color"),
    ("light.rim_tip", "Rim Fresnel + contraluz do material do personagem: aparece mais quando o sol está atrás.", "Fresnel rim + backlight of the character material: strongest when the sun is behind."),
    ("light.no_material", "Sem material de personagem no projeto.", "No character material in the project."),
    // luzes extras
    ("light.extra.point", "Ponto", "Point"),
    ("light.extra.spot", "Spot", "Spot"),
    ("light.extra.rect", "Retangular", "Rect"),
    ("light.extra_hint", "Luzes de cena somam-se ao sol: ponto, holofote, painel e direcional. Edite cada uma em Cenário > Detalhes > Luz.", "Scene lights add to the sun: point, spot, rect and directional. Edit each in Scene > Details > Light."),
    ("light.extra_count", "{n} de 8 luzes em uso", "{n} of 8 lights in use"),
    ("light.extra_tip", "Luzes ponto/spot/retangular exigem um contrato de render com várias luzes (o atual tem uma direcional principal + ambiente). Desabilitado de propósito; o desenho está em docs/native/ILUMINACAO_UI.md.", "Point/spot/rect lights need a multi-light render contract (the current one has one main directional + ambient). Disabled on purpose; the design is in docs/native/ILUMINACAO_UI.md."),
    // cena de prova
    ("light.test_hint", "Para ver sombra projetada o projeto precisa de algo onde ela caia.", "To see cast shadows the project needs something for them to fall on."),
    ("light.add_floor", "Adicionar piso de prova", "Add test floor"),
    ("light.add_floor_tip", "Um plano sob o personagem (um passo de desfazer).", "A plane under the character (one undo step)."),
    ("light.add_hair", "Adicionar volume de cabelo (prova)", "Add hair volume (test)"),
    ("light.add_hair_tip", "Um elipsoide sobre a testa para ver a sombra do cabelo no rosto. É um proxy: o cabelo do Personagem ainda não é malha de cena.", "An ellipsoid over the forehead to see hair shadow on the face. It is a proxy: the Character hair is not a scene mesh yet."),
    ("light.no_geometry", "Personagem ainda sem geometria.", "Character has no geometry yet."),
    // viewport
    ("light.gizmo_tip", "Mostrar a esfera de direção da luz", "Show the light direction sphere"),
    ("light.frame", "Enquadrar personagem", "Frame character"),
    ("light.view_front", "Vista frontal", "Front view"),
    ("light.front", "frente", "front"),
    ("light.help", "Arraste o disco do sol (Shift = passos de 15°) · Ctrl+arrastar gira a luz · MMB/Alt+LMB órbita · Shift+MMB pan · roda zoom · F enquadra", "Drag the sun disk (Shift = 15° steps) · Ctrl+drag turns the light · MMB/Alt+LMB orbit · Shift+MMB pan · wheel zoom · F frames"),
    // histórico
    ("light.cmd.light", "Luz", "Light"),
    ("light.cmd.material", "Terminador / rim", "Terminator / rim"),
    ("light.cmd.rig", "Rig de luz", "Light rig"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn no_duplicate_keys_and_no_empty_strings() {
        let mut seen = HashSet::new();
        for (k, pt, en) in LIGHT_TABLE {
            assert!(seen.insert(*k), "chave duplicada {k}");
            assert!(!pt.is_empty() && !en.is_empty(), "tradução vazia em {k}");
        }
    }

    #[test]
    fn every_key_used_in_the_source_is_translated() {
        let sources = [
            include_str!("app/lighting_ui/mod.rs"),
            include_str!("app/lighting_ui/panels.rs"),
            include_str!("app/lighting_ui/viewport.rs"),
            include_str!("app/lighting_ui/script.rs"),
            include_str!("lighting/mod.rs"),
            include_str!("lighting/gizmo.rs"),
            include_str!("lighting/rigs.rs"),
        ];
        let known: HashSet<&str> = LIGHT_TABLE.iter().map(|(k, _, _)| *k).collect();
        let mut missing = Vec::new();
        for src in sources {
            let mut rest = src;
            while let Some(i) = rest.find("\"light.") {
                let tail = &rest[i + 1..];
                let Some(end) = tail.find('"') else { break };
                let key = &tail[..end];
                // `light.rig.` é só o prefixo do `concat!` do catálogo (conferido abaixo por rig)
                if !key.ends_with('.') && key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_') && !known.contains(key) {
                    missing.push(key.to_string());
                }
                rest = &tail[end..];
            }
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "chaves sem tradução: {missing:?}");
    }

    #[test]
    fn every_rig_has_a_label_and_a_tip_in_both_languages() {
        use crate::i18n::{tr_in, Lang};
        for rig in &crate::lighting::rigs::RIGS {
            for key in [rig.label_key, rig.tip_key] {
                for lang in [Lang::PtBr, Lang::En] {
                    assert_ne!(tr_in(lang, key), key, "{key} sem tradução");
                }
            }
        }
    }

    #[test]
    fn light_labels_resolve_in_both_languages() {
        use crate::i18n::{tr_in, Lang};
        for (k, pt, en) in LIGHT_TABLE {
            assert_eq!(tr_in(Lang::PtBr, k), *pt);
            assert_eq!(tr_in(Lang::En, k), *en);
        }
    }
}
