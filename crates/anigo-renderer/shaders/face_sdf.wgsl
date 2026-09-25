// ANIGO Face Shadow SDF (Fase 2 #17) — sombra facial estilo anime
// (Genshin Impact / Guilty Gear): a sombra do nariz, olhos e queixo segue o
// SDF do rosto projetado pelo azimut da luz, não as normais poligonais.
//
// Este arquivo é a **definição canônica** do bloco de face SDF. O passe de
// cel compila o `cel_shading.wgsl` como fonte única, então o bloco entre os
// marcadores ANIGO-FACE-SDF-BEGIN/END precisa ser **byte-idêntico** aqui e no
// cel shader (conferido por scripts/check_wgsl.mjs) — uma definição, dois usos.
//
// Convenção do SDF (canal R da textura face_sdf_tex):
//   0 = centro da região de sombra (nasal/olhos/queixo), 1 = fora da região.
// Sem textura, o renderer ancora o neutro 1x1 branco (R = 1) → fator 0 → a
// sombra facial não altera a imagem (paridade com o frame congelado).
//
// Projeção angular: o eixo Z local do nó 0 é a frente do rosto. A direção da
// luz mundial L é projetada nesse espaço local pelos colunas da matriz de
// modelo (sem inversa — colunas de R·S mantêm direção para escala positiva):
//   theta = atan2(dot(L, eixoX_local), dot(L, eixoZ_local))
// theta ≈ 0  → luz frontal (sombra mínima)
// |theta|→π  → luz traseira (sombra máxima)

// ANIGO-FACE-SDF-BEGIN — bloco compartilhado (byte a byte igual em
// face_sdf.wgsl e cel_shading.wgsl; conferido por scripts/check_wgsl.mjs)
fn face_sdf_theta(local_x: vec3<f32>, local_z: vec3<f32>, light_dir: vec3<f32>) -> f32 {
    let lx = dot(light_dir, local_x);
    let lz = dot(light_dir, local_z);
    return atan2(lx, lz);
}
fn face_sdf_threshold(theta: f32, offset: f32) -> f32 {
    // light_front: 1 = luz frontal (theta ≈ 0), 0 = luz traseira (|theta| ≈ π).
    // A banda de sombra cresce até 0.25 de threshold quando a luz vai para o
    // lado/costas; offset é o controle do usuário (face_shadow_offset).
    let light_front = cos(theta) * 0.5 + 0.5;
    return 0.5 + (1.0 - light_front) * 0.25 + offset;
}
fn face_sdf_factor(sdf: f32, threshold: f32, softness: f32) -> f32 {
    // 1 = totalmente na sombra, 0 = fora da região de sombra.
    let s = max(softness, 0.001);
    return 1.0 - smoothstep(threshold - s, threshold + s, sdf);
}
// ANIGO-FACE-SDF-END
