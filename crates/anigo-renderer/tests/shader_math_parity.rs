//! Paridade WGSL ⇄ Rust da matemática do motor cel.
//!
//! As funções puras do shader (face SDF, largura/taper do contorno, rampa 2D)
//! são EXTRAÍDAS do texto dos arquivos de produção e executadas numa GPU (um
//! compute shader mínimo que as chama sobre uma grade de entradas); o resultado
//! é comparado com `anigo_core::npr`. Se alguém mudar uma fórmula num lado sem
//! o outro, este teste cai — a equivalência deixa de ser uma promessa.
//!
//! Sem adaptador GPU os testes se pulam com mensagem.

use anigo_core::npr;
use anigo_renderer::HeadlessRenderer;
use glam::Vec3;
use wgpu::util::DeviceExt;

fn renderer_or_skip(test: &str) -> Option<HeadlessRenderer> {
    match pollster::block_on(HeadlessRenderer::new()) {
        Ok(r) => Some(r),
        Err(error) => {
            eprintln!("[{test}] PULADO: sem adaptador GPU utilizável ({error})");
            None
        }
    }
}

fn shader_source(name: &str) -> String {
    std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders").join(name),
    )
    .unwrap()
}

/// Texto do bloco entre os marcadores (inclusive).
fn marked_block(source: &str, begin: &str, end: &str) -> String {
    let start = source.find(begin).unwrap_or_else(|| panic!("sem {begin}"));
    let stop = source.find(end).unwrap_or_else(|| panic!("sem {end}")) + end.len();
    source[start..stop].to_string()
}

/// Texto completo de `fn <name>(...) { ... }` (chaves balanceadas).
fn extract_fn(source: &str, name: &str) -> String {
    let start = source
        .find(&format!("fn {name}("))
        .unwrap_or_else(|| panic!("fn {name} não encontrada"));
    let open = start + source[start..].find('{').unwrap();
    let mut depth = 0;
    for (i, c) in source[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return source[start..open + i + 1].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("fn {name} sem fechamento");
}

/// Executa `body` (WGSL; recebe `a`, `b`, `c`: `vec4<f32>` — entradas 0..2 do caso —
/// e devolve um `vec4<f32>`) uma vez por caso, depois de `prelude`.
fn run_cases(renderer: &HeadlessRenderer, prelude: &str, body: &str, cases: &[[[f32; 4]; 3]]) -> Vec<[f32; 4]> {
    let device = &renderer.device;
    let queue = &renderer.queue;
    let source = format!(
        "{prelude}\n\
         @group(0) @binding(0) var<storage, read> inputs: array<vec4<f32>>;\n\
         @group(0) @binding(1) var<storage, read_write> outputs: array<vec4<f32>>;\n\
         @compute @workgroup_size(1)\n\
         fn main(@builtin(global_invocation_id) id: vec3<u32>) {{\n\
             let a = inputs[id.x * 3u];\n\
             let b = inputs[id.x * 3u + 1u];\n\
             let c = inputs[id.x * 3u + 2u];\n\
             outputs[id.x] = {body};\n\
         }}\n"
    );
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shader math parity"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let flat: Vec<f32> = cases.iter().flatten().flatten().copied().collect();
    let input = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("parity inputs"),
        contents: bytemuck::cast_slice(&flat),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let out_size = (cases.len() * 16) as u64;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("parity outputs"),
        size: out_size,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("parity staging"),
        size: out_size,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("parity pipeline"),
        layout: None,
        module: &module,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("parity bind group"),
        layout: &pipeline.get_bind_group_layout(0),
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: input.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: output.as_entire_binding() },
        ],
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let mut pass = encoder.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.dispatch_workgroups(cases.len() as u32, 1, 1);
    }
    encoder.copy_buffer_to_buffer(&output, 0, &staging, 0, out_size);
    queue.submit(Some(encoder.finish()));
    let slice = staging.slice(..);
    slice.map_async(wgpu::MapMode::Read, |r| r.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = slice.get_mapped_range().unwrap();
    let floats: &[f32] = bytemuck::cast_slice(&*data);
    floats.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
}

fn close(label: &str, gpu: f32, cpu: f32, tolerance: f32) {
    let scale = cpu.abs().max(1.0);
    assert!(
        (gpu - cpu).abs() <= tolerance * scale,
        "{label}: GPU {gpu} vs Rust {cpu} (tolerância {tolerance})"
    );
}

#[test]
fn face_sdf_block_matches_the_rust_math() {
    let Some(renderer) = renderer_or_skip("parity_face") else { return };
    // O bloco canônico (idêntico em face_sdf.wgsl e cel_shading.wgsl, conferido por check:wgsl)
    let block = marked_block(
        &shader_source("face_sdf.wgsl"),
        "// ANIGO-FACE-SDF-BEGIN",
        "// ANIGO-FACE-SDF-END",
    );
    // theta(local_x, local_z, light): a = local_x, b = local_z, c = light
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    let axes = [
        (Vec3::X, Vec3::Z),
        (Vec3::new(0.8, 0.0, -0.6), Vec3::new(0.6, 0.0, 0.8)), // cabeça girada
        (Vec3::X * 2.0, Vec3::Z * 2.0),                        // escala uniforme
    ];
    for (x, z) in axes {
        for deg in (-180..=180).step_by(15) {
            let r = (deg as f32).to_radians();
            for elevation in [0.0f32, 0.4, -0.3] {
                let light = Vec3::new(r.sin(), elevation, r.cos()).normalize();
                cases.push([[x.x, x.y, x.z, 0.0], [z.x, z.y, z.z, 0.0], [light.x, light.y, light.z, 0.0]]);
                expected.push(npr::face_sdf_theta(x, z, light));
            }
        }
    }
    let out = run_cases(
        &renderer,
        &block,
        "vec4<f32>(face_sdf_theta(a.xyz, b.xyz, c.xyz), 0.0, 0.0, 0.0)",
        &cases,
    );
    for (i, (gpu, cpu)) in out.iter().zip(&expected).enumerate() {
        close(&format!("theta[{i}]"), gpu[0], *cpu, 2e-5);
    }

    // threshold(theta, offset) / factor(sdf, thr, softness): a = (theta, offset, sdf, thr), b.x = softness
    let mut cases = Vec::new();
    let mut thr_expected = Vec::new();
    let mut factor_expected = Vec::new();
    for t in (-32..=32).map(|i| i as f32 * std::f32::consts::PI / 32.0) {
        for offset in [-0.1f32, 0.0, 0.07] {
            for sdf in [0.0f32, 0.3, 0.5, 0.62, 0.8, 1.0] {
                for softness in [0.0f32, 0.01, 0.05, 0.2] {
                    let thr = npr::face_sdf_threshold(t, offset);
                    cases.push([[t, offset, sdf, thr], [softness, 0.0, 0.0, 0.0], [0.0; 4]]);
                    thr_expected.push(thr);
                    factor_expected.push(npr::face_sdf_factor(sdf, thr, softness));
                }
            }
        }
    }
    let out = run_cases(
        &renderer,
        &block,
        "vec4<f32>(face_sdf_threshold(a.x, a.y), face_sdf_factor(a.z, a.w, b.x), 0.0, 0.0)",
        &cases,
    );
    for (i, gpu) in out.iter().enumerate() {
        close(&format!("threshold[{i}]"), gpu[0], thr_expected[i], 1e-5);
        close(&format!("factor[{i}]"), gpu[1], factor_expected[i], 1e-4);
    }

    // mirror: a = (u, v, theta, center)
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for u in [0.0f32, 0.1, 0.25, 0.4, 0.5, 0.9, 1.0] {
        for theta in [-3.0f32, -0.5, -1e-6, 0.0, 1e-6, 0.5, 3.0] {
            for center in [0.0f32, 0.25, 0.5] {
                cases.push([[u, 0.3, theta, center], [0.0; 4], [0.0; 4]]);
                expected.push(npr::face_sdf_mirror_u(u, theta, center));
            }
        }
    }
    let out = run_cases(
        &renderer,
        &block,
        "vec4<f32>(face_sdf_mirror_uv(vec2<f32>(a.x, a.y), a.z, a.w), 0.0, 0.0)",
        &cases,
    );
    for (i, (gpu, cpu)) in out.iter().zip(&expected).enumerate() {
        assert_eq!(gpu[0], *cpu, "mirror_u[{i}] u={}", cases[i][0][0]);
        assert_eq!(gpu[1], 0.3, "o v nunca muda");
    }
}

#[test]
fn outline_width_and_taper_match_the_rust_math() {
    let Some(renderer) = renderer_or_skip("parity_outline") else { return };
    let source = shader_source("inverted_hull.wgsl");
    let prelude = format!(
        "{}\n{}",
        extract_fn(&source, "outline_width_scale"),
        extract_fn(&source, "outline_taper_scale")
    );
    // width_scale(view_depth, ref_eff, falloff, min_scale): a; taper(n_dot_v, taper): b.xy
    let mut cases = Vec::new();
    let mut width_expected = Vec::new();
    let mut taper_expected = Vec::new();
    for depth in [0.0f32, 0.5, 1.0, 2.5, 5.0, 10.0, 40.0, 500.0] {
        for ref_eff in [1.0f32, 4.0, 9.39658] {
            for falloff in [0.0f32, 0.25, 0.5, 1.0] {
                for min_scale in [0.0f32, 0.2, 0.9] {
                    for (ndv, taper) in [(0.0f32, 0.0f32), (0.5, 0.5), (-0.7, 1.0), (1.0, 1.0), (2.0, 0.3)] {
                        cases.push([[depth, ref_eff, falloff, min_scale], [ndv, taper, 0.0, 0.0], [0.0; 4]]);
                        width_expected.push(npr::outline_width_scale(depth, ref_eff, falloff, min_scale));
                        taper_expected.push(npr::outline_taper_scale(ndv, taper));
                    }
                }
            }
        }
    }
    let out = run_cases(
        &renderer,
        &prelude,
        "vec4<f32>(outline_width_scale(a.x, a.y, a.z, a.w), outline_taper_scale(b.x, b.y), 0.0, 0.0)",
        &cases,
    );
    for (i, gpu) in out.iter().enumerate() {
        close(&format!("width_scale[{i}] {:?}", cases[i][0]), gpu[0], width_expected[i], 2e-5);
        close(&format!("taper_scale[{i}]"), gpu[1], taper_expected[i], 1e-6);
    }
    // padrões: exatamente 1.0 (o contorno anterior byte a byte)
    let out = run_cases(
        &renderer,
        &prelude,
        "vec4<f32>(outline_width_scale(a.x, a.y, 0.0, 0.7), outline_taper_scale(a.z, 0.0), 0.0, 0.0)",
        &[[[3.0, 5.0, 0.7, 0.0], [0.0; 4], [0.0; 4]]],
    );
    assert_eq!(out[0][0], 1.0);
    assert_eq!(out[0][1], 1.0);
}

#[test]
fn ramp_index_math_matches_the_rust_math() {
    let Some(renderer) = renderer_or_skip("parity_ramp") else { return };
    let source = shader_source("cel_shading.wgsl");
    let prelude = format!(
        "{}\n{}\n{}",
        extract_fn(&source, "ramp_v"),
        extract_fn(&source, "ramp_u"),
        extract_fn(&source, "blend_toon_factor")
    );
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for row in [-1.0f32, 0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 7.0] {
        for hl in [0.0f32, 0.2, 0.5, 0.73, 1.0] {
            for thr in [0.3f32, 0.5, 0.62] {
                for (an, rp, bl) in [(0.0f32, 1.0f32, 0.0f32), (0.3, 0.8, 0.5), (1.0, 0.0, 1.0)] {
                    cases.push([[row, hl, thr, 0.0], [an, rp, bl, 0.0], [0.0; 4]]);
                    expected.push([
                        npr::ramp_v(row),
                        npr::ramp_u(hl, thr),
                        npr::blend_toon_factor(an, rp, bl),
                    ]);
                }
            }
        }
    }
    let out = run_cases(
        &renderer,
        &prelude,
        "vec4<f32>(ramp_v(a.x), ramp_u(a.y, a.z), blend_toon_factor(b.x, b.y, b.z), 0.0)",
        &cases,
    );
    for (i, gpu) in out.iter().enumerate() {
        close(&format!("ramp_v[{i}]"), gpu[0], expected[i][0], 1e-7);
        close(&format!("ramp_u[{i}]"), gpu[1], expected[i][1], 1e-6);
        close(&format!("blend[{i}]"), gpu[2], expected[i][2], 1e-6);
    }
    // blend = 0 devolve o analítico EXATO
    let out = run_cases(
        &renderer,
        &prelude,
        "vec4<f32>(blend_toon_factor(b.x, b.y, 0.0), 0.0, 0.0, 0.0)",
        &[[[0.0; 4], [0.3, 0.9, 0.0, 0.0], [0.0; 4]]],
    );
    assert_eq!(out[0][0], 0.3);
}

#[test]
fn shadow_math_matches_the_rust_math() {
    let Some(renderer) = renderer_or_skip("parity_shadow") else { return };
    let source = shader_source("cel_shading.wgsl");
    // matriz REAL da luz (a que o renderer envia), embutida como literais WGSL exatos
    let pts: Vec<Vec3> = [Vec3::new(0.0, 1.0, 0.0), Vec3::new(3.0, 0.5, -2.0), Vec3::new(-2.0, 2.0, 1.5)]
        .into_iter()
        .flat_map(|c| {
            (0..8).map(move |i| {
                c + Vec3::new(
                    if i & 1 == 0 { -0.8 } else { 0.8 },
                    if i & 2 == 0 { -0.8 } else { 0.8 },
                    if i & 4 == 0 { -0.8 } else { 0.8 },
                )
            })
        })
        .collect();
    let fit = anigo_core::shadow::fit_directional([0.45, 0.75, 0.35], &pts, None, 2048).expect("ajuste");
    let m = fit.view_proj.to_cols_array();
    let literal: Vec<String> = m.iter().map(|v| format!("{v:e}")).collect();
    let prelude = format!(
        "const LIGHT_M: mat4x4<f32> = mat4x4<f32>({});\n{}\n{}\n{}",
        literal.join(", "),
        extract_fn(&source, "shadow_project"),
        extract_fn(&source, "shadow_receiver_pos"),
        extract_fn(&source, "shadow_step")
    );
    // project: c = ponto de mundo
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for p in pts.iter().copied().chain([Vec3::ZERO, Vec3::new(10.0, -3.0, 4.0), Vec3::new(-1.0, 0.2, 0.9)]) {
        cases.push([[0.0; 4], [0.0; 4], [p.x, p.y, p.z, 0.0]]);
        expected.push(anigo_core::shadow::project(&fit.view_proj, p));
    }
    let out = run_cases(&renderer, &prelude, "vec4<f32>(shadow_project(LIGHT_M, c.xyz), 0.0)", &cases);
    for (i, (gpu, cpu)) in out.iter().zip(&expected).enumerate() {
        close(&format!("project.u[{i}]"), gpu[0], cpu.x, 2e-5);
        close(&format!("project.v[{i}]"), gpu[1], cpu.y, 2e-5);
        close(&format!("project.z[{i}]"), gpu[2], cpu.z, 2e-5);
    }
    // receiver_pos: a = mundo, b = normal, c = (luz.xyz, normal_bias)
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for n in [Vec3::Y, Vec3::new(0.3, 0.9, -0.3).normalize(), Vec3::X, Vec3::new(-0.6, -0.2, 0.77).normalize()] {
        for l in [Vec3::Y, Vec3::new(0.45, 0.75, 0.35).normalize(), -Vec3::X, Vec3::new(0.0, -1.0, 0.0)] {
            for bias in [0.0f32, 0.03, 0.2] {
                let w = Vec3::new(1.5, 0.25, -0.5);
                cases.push([[w.x, w.y, w.z, 0.0], [n.x, n.y, n.z, 0.0], [l.x, l.y, l.z, bias]]);
                expected.push(anigo_core::shadow::receiver_position(w, n, l, bias));
            }
        }
    }
    let out = run_cases(&renderer, &prelude, "vec4<f32>(shadow_receiver_pos(a.xyz, b.xyz, c.xyz, c.w), 0.0)", &cases);
    for (i, (gpu, cpu)) in out.iter().zip(&expected).enumerate() {
        close(&format!("receiver.x[{i}]"), gpu[0], cpu.x, 1e-6);
        close(&format!("receiver.y[{i}]"), gpu[1], cpu.y, 1e-6);
        close(&format!("receiver.z[{i}]"), gpu[2], cpu.z, 1e-6);
    }
    // step: a = (oclusão, meia-largura)
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for occ in (0..=20).map(|i| i as f32 / 20.0) {
        for hw in [0.0f32, 0.05, 0.1, 0.25, 0.5] {
            cases.push([[occ, hw, 0.0, 0.0], [0.0; 4], [0.0; 4]]);
            expected.push(anigo_core::shadow::shadow_step(occ, hw));
        }
    }
    let out = run_cases(&renderer, &prelude, "vec4<f32>(shadow_step(a.x, a.y), 0.0, 0.0, 0.0)", &cases);
    for (i, (gpu, cpu)) in out.iter().zip(&expected).enumerate() {
        close(&format!("step[{i}] {:?}", cases[i][0]), gpu[0], *cpu, 1e-5);
    }
}
