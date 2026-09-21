// ANIGO Anime Eye Shader (Fase 2 #43) — íris estilizada com parallax mapping
// e destaques desacoplados.
//
// Este arquivo é a **definição canônica** do bloco de olho anime. O passe de
// cel compila o `cel_shading.wgsl` como fonte única, então o bloco entre os
// marcadores ANIGO-ANIME-EYE-BEGIN/END precisa ser **byte-idêntico** aqui e no
// cel shader (conferido por scripts/check_wgsl.mjs) — uma definição, dois usos.
//
// Os olhos carregam >70% da expressividade em anime. As duas convenções
// estéticas inegociáveis que este bloco implementa:
//
// 1. PARALLAX DA ÍRIS — a pupila/íris parecem afundadas no globo ocular sem
//    modelar a cavidade geométrica: o UV é deslocado pela direção de visão na
//    base tangente da superfície, com a profundidade da "recalada":
//        UV_iris = UV + V_tangent.xy × depth_scale
//    Orbitar a câmera ao redor do rosto desloca V_tangent → a íris "anda"
//    dentro do olho → profundidade convexa 3D convincente.
//
// 2. HIGHLIGHTS DESACOPLADOS — o brilho branco característico é uma camada
//    desenhada à mão (mask procedural, sem textura) somada DEPOIS da
//    iluminação, nunca multiplicada por luz/sombra. A convenção de anime: o
//    olho brilha mesmo em penumbra total (olhos "vidrados" mortem a cena).
//
// O mask dos highlights é procedural (dois brilhos: principal elíptico no
// canto superior e secundário menor no inferior) ancorado ao UV do olho —
// quando o material tem uma textura de olho real (Fase 2 #26) no slot main,
// o parallax a desloca e o mask adiciona os brilhos sobre ela.

// ANIGO-ANIME-EYE-BEGIN — bloco compartilhado (byte a byte igual em
// anime_eye.wgsl e cel_shading.wgsl; conferido por scripts/check_wgsl.mjs)
fn eye_parallax_uv(uv: vec2<f32>, view_tangent_xy: vec2<f32>, depth_scale: f32) -> vec2<f32> {
    // V_tangent.xy: direção de visão projetada na base tangente (T, B) da
    // superfície. depth_scale é a "recalada" da íris (0 = plano, >0 = fundo).
    return clamp(uv + view_tangent_xy * depth_scale, vec2<f32>(0.0), vec2<f32>(1.0));
}
fn eye_highlight_mask(uv: vec2<f32>) -> f32 {
    // Dois brilhos desenhados à mão (convenção clássica de olho anime):
    // principal = elipse grande no canto superior-esquerco do olho,
    // secundário = ponto menor no canto inferior-direito (85% da intensidade).
    let d_main = length((uv - vec2<f32>(0.38, 0.62)) * vec2<f32>(1.0, 0.72));
    let main = 1.0 - smoothstep(0.075, 0.125, d_main);
    let d_second = length(uv - vec2<f32>(0.68, 0.34));
    let second = (1.0 - smoothstep(0.028, 0.055, d_second)) * 0.85;
    return max(main, second);
}
fn eye_highlight_rgb(mask: f32, intensity: f32) -> vec3<f32> {
    // Branco linear × mask × intensidade — somado após a iluminação (nunca
    // multiplicado por luz/sombra): visível mesmo em sombra total.
    return vec3<f32>(mask * intensity);
}
// ANIGO-ANIME-EYE-END
