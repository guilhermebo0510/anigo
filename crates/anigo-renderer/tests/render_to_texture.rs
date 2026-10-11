//! `render_to_texture` / `encode_scene` / `from_device` / `RenderTargetCache`:
//! o caminho do viewport interativo tem de produzir os MESMOS pixels do
//! `render_scene` (readback) e reaproveitar os alvos entre quadros.
//!
//! Os testes pulam com mensagem quando não há adaptador GPU.

use std::sync::Arc;

use anigo_core::mesh::{BaseGender, Mesh};
use anigo_core::scene::{Scene, SceneNode};
use anigo_renderer::{offscreen_color_format, HeadlessRenderer, RenderTarget};

fn renderer_or_skip(test: &str) -> Option<HeadlessRenderer> {
    match pollster::block_on(HeadlessRenderer::new()) {
        Ok(renderer) => Some(renderer),
        Err(error) => {
            eprintln!("[{test}] PULADO: sem adaptador GPU utilizável ({error})");
            None
        }
    }
}

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

/// Textura como o viewport egui a criaria (+ COPY_SRC só para o teste ler).
fn viewport_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("test viewport texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: offscreen_color_format(),
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Lê RGBA8 de volta, sem o padding de 256 B por linha.
fn read_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let padded = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: (padded * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("poll");
    rx.recv().unwrap().expect("map");
    let data = slice.get_mapped_range().expect("range");
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        let start = (row * padded) as usize;
        out.extend_from_slice(&data[start..start + (width * 4) as usize]);
    }
    drop(data);
    buffer.unmap();
    out
}

fn assert_same_pixels(label: &str, left: &[u8], right: &[u8]) {
    assert_eq!(left.len(), right.len(), "[{label}] tamanhos diferentes");
    let diffs = left.iter().zip(right).filter(|(a, b)| a != b).count();
    assert_eq!(diffs, 0, "[{label}] {diffs} bytes diferentes");
}

#[test]
fn render_to_texture_matches_render_scene() {
    let Some(renderer) = renderer_or_skip("render_to_texture") else {
        return;
    };
    // 256x256 (linha alinhada) e 100x60 (linha NÃO alinhada a 256 B).
    for (name, scene, w, h) in [
        ("female_base_256", female_base_scene(), 256, 256),
        ("cube_100x60", cube_scene(), 100, 60),
    ] {
        let (image, _) = pollster::block_on(renderer.render_scene(&scene, w, h)).unwrap();
        assert!(image.as_raw().iter().any(|b| *b != 0), "[{name}] quadro vazio");

        let texture = viewport_texture(&renderer.device, w, h);
        let metrics = renderer.render_to_texture(&scene, &texture).unwrap();
        assert!(metrics.draw_calls >= 1);
        let pixels = read_texture(&renderer.device, &renderer.queue, &texture);
        assert_same_pixels(name, image.as_raw(), &pixels);

        // Reuso da MESMA textura: o segundo quadro sobre conteúdo anterior é
        // idêntico (clear completo + cache reaproveitado).
        renderer.render_to_texture(&scene, &texture).unwrap();
        let again = read_texture(&renderer.device, &renderer.queue, &texture);
        assert_same_pixels(&format!("{name}/reuse"), image.as_raw(), &again);
    }
}

#[test]
fn encode_scene_uses_external_encoder_and_matches() {
    let Some(renderer) = renderer_or_skip("encode_scene") else {
        return;
    };
    let scene = female_base_scene();
    let (image, _) = pollster::block_on(renderer.render_scene(&scene, 128, 128)).unwrap();

    let texture = viewport_texture(&renderer.device, 128, 128);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mut encoder = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("external encoder"),
        });
    let metrics = renderer
        .encode_scene(
            &mut encoder,
            &scene,
            &RenderTarget {
                color_view: &view,
                width: 128,
                height: 128,
            },
        )
        .unwrap();
    assert_eq!(metrics.executed_passes.first().map(String::as_str), Some("outline"));
    renderer.queue.submit(Some(encoder.finish()));
    let pixels = read_texture(&renderer.device, &renderer.queue, &texture);
    assert_same_pixels("encode_scene", image.as_raw(), &pixels);
}

#[test]
fn render_target_cache_reallocates_only_when_size_changes() {
    let Some(renderer) = renderer_or_skip("target_cache") else {
        return;
    };
    let scene = cube_scene();
    assert_eq!(renderer.target_cache_allocations(), 0);

    pollster::block_on(renderer.render_scene(&scene, 64, 64)).unwrap();
    let first = renderer.target_cache_allocations();
    assert!(first >= 1);

    // Mesmo tamanho, por qualquer caminho: nenhuma alocação nova.
    pollster::block_on(renderer.render_scene(&scene, 64, 64)).unwrap();
    let texture = viewport_texture(&renderer.device, 64, 64);
    renderer.render_to_texture(&scene, &texture).unwrap();
    assert_eq!(renderer.target_cache_allocations(), first);

    // Tamanho novo: realoca uma vez e estabiliza.
    pollster::block_on(renderer.render_scene(&scene, 96, 48)).unwrap();
    let resized = renderer.target_cache_allocations();
    assert!(resized > first);
    pollster::block_on(renderer.render_scene(&scene, 96, 48)).unwrap();
    assert_eq!(renderer.target_cache_allocations(), resized);
}

#[test]
fn render_to_texture_rejects_incompatible_textures() {
    let Some(renderer) = renderer_or_skip("rejects") else {
        return;
    };
    let scene = cube_scene();
    let wrong_format = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("wrong format"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    assert!(renderer.render_to_texture(&scene, &wrong_format).is_err());

    let no_attachment = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("no render attachment"),
        size: wgpu::Extent3d {
            width: 16,
            height: 16,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: offscreen_color_format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    assert!(renderer.render_to_texture(&scene, &no_attachment).is_err());
}

#[test]
fn from_device_shares_an_external_device() {
    // Device criado "por fora", como o eframe/egui faria.
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) {
        Ok(adapter) => adapter,
        Err(error) => {
            eprintln!("[from_device] PULADO: sem adaptador GPU utilizável ({error})");
            return;
        }
    };
    let info = adapter.get_info();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("device externo");
    let (device, queue) = (Arc::new(device), Arc::new(queue));

    let shared = HeadlessRenderer::from_device(device.clone(), queue.clone(), info).unwrap();
    assert!(Arc::ptr_eq(&shared.device, &device));

    let scene = female_base_scene();
    let texture = viewport_texture(&device, 128, 128);
    shared.render_to_texture(&scene, &texture).unwrap();
    let pixels = read_texture(&device, &queue, &texture);
    assert!(pixels.iter().any(|b| *b != 0));

    let (image, _) = pollster::block_on(shared.render_scene(&scene, 128, 128)).unwrap();
    assert_same_pixels("from_device", image.as_raw(), &pixels);
}
