//! "Impressão digital" do render: hash FNV-1a dos pixels RGBA de cenas
//! determinísticas, gerado com Vulkan por software (lavapipe).
//!
//! Serve de rede de segurança para migrações de dependência (wgpu 24 → 30) e
//! refactors do `HeadlessRenderer`: os hashes têm de continuar IGUAIS. Quando
//! não há adaptador GPU, os testes se pulam com mensagem (não falham).
//!
//! Para (re)gerar as constantes:
//! `GOLDEN_PRINT=1 cargo test -p anigo-renderer --test golden_frames -- --nocapture`
//! (só altere uma constante com justificativa por escrito no commit).

use anigo_core::mesh::{BaseGender, Mesh};
use anigo_core::scene::{Scene, SceneNode};
use anigo_renderer::{fnv1a64, HeadlessRenderer};

const SIZE: u32 = 256;

/// Base feminina canônica, câmera padrão, 256x256.
const GOLDEN_FEMALE_BASE: u64 = 0xbaa8_e1f7_4abe_3d5d;
/// Cubo com outline (plano padrão do contrato: outline + cel), 256x256.
const GOLDEN_CUBE_OUTLINE: u64 = 0xd244_06b5_945f_dd58;
/// Base feminina com DoF ligado, 256x256.
///
/// ATENÇÃO (defeito PRÉ-EXISTENTE, registrado no baseline wgpu 24): o caminho
/// de DoF do headless gera um `CommandBuffer` inválido — o resolve da
/// profundidade MSAA→1× usa `copy_texture_to_texture` com a textura de
/// profundidade sem `COPY_SRC` (e com contagens de amostras diferentes), então
/// o quadro inteiro sai zerado (RGBA = 0). Este hash é, portanto, o do quadro
/// 100% transparente (FNV-1a de 256·256·4 zeros). Ele congela o comportamento
/// atual para a migração; quando o DoF for consertado o hash MUDA de propósito.
const GOLDEN_DOF: u64 = 0x9c73_5bed_0a72_2325;

fn female_base_scene() -> Scene {
    let mesh = Mesh::create_canonical_base(BaseGender::Female);
    let mut scene = Scene::new_empty();
    scene.add_node(SceneNode::new("base", "CanonicalBase").with_mesh(mesh));
    scene
}

fn cube_scene() -> Scene {
    let mut scene = Scene::new_empty();
    scene.add_node(SceneNode::new("cube", "Cube").with_mesh(Mesh::create_cube(1.0)));
    scene
}

fn dof_scene() -> Scene {
    let mut scene = female_base_scene();
    scene.dof.enabled = true;
    scene
}

fn renderer_or_skip(test: &str) -> Option<HeadlessRenderer> {
    match pollster::block_on(HeadlessRenderer::new()) {
        Ok(renderer) => {
            // Os hashes são do rasterizador de referência (lavapipe, Vulkan por
            // software). Outras GPUs/backends (inclusive llvmpipe via GL)
            // divergem por precisão/ordem de rasterização legítima.
            let info = &renderer.adapter_info;
            if info.backend != wgpu::Backend::Vulkan || !info.name.contains("llvmpipe") {
                eprintln!(
                    "[{test}] PULADO: hashes são do lavapipe/Vulkan; adaptador atual: {} ({:?})",
                    info.name, info.backend
                );
                return None;
            }
            Some(renderer)
        }
        Err(error) => {
            eprintln!("[{test}] PULADO: sem adaptador GPU utilizável ({error})");
            None
        }
    }
}

fn frame_hash(renderer: &HeadlessRenderer, scene: &Scene) -> (u64, Vec<u8>) {
    let (image, _) = pollster::block_on(renderer.render_scene(scene, SIZE, SIZE))
        .expect("render_scene deve funcionar");
    assert_eq!((image.width(), image.height()), (SIZE, SIZE));
    let raw = image.into_raw();
    (fnv1a64(&raw), raw)
}

fn check(test: &str, scene: Scene, expected: u64, allow_blank: bool) {
    let Some(renderer) = renderer_or_skip(test) else {
        return;
    };
    let (hash, raw) = frame_hash(&renderer, &scene);
    // Determinismo: um segundo quadro da mesma cena tem de ser idêntico.
    let (again, _) = frame_hash(&renderer, &scene);
    assert_eq!(hash, again, "[{test}] render não determinístico entre quadros");
    if std::env::var_os("GOLDEN_PRINT").is_some() {
        println!("[{test}] diagnostics: {:?}", anigo_renderer::diagnostics_entries());
        println!("[{test}] device errors: {:?}", anigo_renderer::recent_errors());
    }
    assert!(allow_blank || raw.iter().any(|b| *b != 0), "[{test}] quadro vazio");
    if std::env::var_os("GOLDEN_PRINT").is_some() {
        println!(
            "[{test}] adapter={} backend={:?} hash=0x{hash:016x}",
            renderer.adapter_info.name, renderer.adapter_info.backend
        );
    }
    assert_eq!(
        hash, expected,
        "[{test}] hash do quadro divergiu: obtido 0x{hash:016x}, esperado 0x{expected:016x}"
    );
}

#[test]
fn golden_female_base_default_camera() {
    check("female_base", female_base_scene(), GOLDEN_FEMALE_BASE, false);
}

#[test]
fn golden_cube_with_outline() {
    check("cube_outline", cube_scene(), GOLDEN_CUBE_OUTLINE, false);
}

#[test]
fn golden_dof_enabled() {
    check("dof", dof_scene(), GOLDEN_DOF, true);
}
