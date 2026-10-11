//! Auto-unwrap (estilo "Smart UV Project") com o **xatlas** (C++ vendorizado).
//!
//! O xatlas recebe a malha **como polígonos** (`faceVertexCount`): n-gons chegam
//! inteiros, sem triangulação por nossa conta. Ele divide a superfície em charts,
//! parametriza cada um e empacota tudo num atlas único. Como charts criam costuras,
//! o xatlas devolve vértices duplicados nas costuras; aqui eles são **mapeados de
//! volta para os corners originais**: o resultado é um UV por corner
//! ([`AtlasResult::uvs_per_corner`]) e a topologia do [`EditMesh`] **não muda**
//! (mesmos vértices, mesmas faces, mesmos corners — só `Corner::uv` é reescrito).
//!
//! # Convenções
//! * UV normalizado em `[0, 1]` dividindo pelo tamanho do atlas; a origem e o
//!   sentido de V são os do xatlas (sem flip) — quem consome decide a convenção.
//! * Faces ocultas (`Face::hidden`) e degeneradas (área zero) não entram no atlas:
//!   ficam em [`AtlasResult::unmapped_faces`] e mantêm o UV que tinham.
//! * Um atlas só: se o `resolution` pedido não comportar `texels_per_unit`, o xatlas
//!   geraria vários sub-atlas e a operação falha com erro explicativo.
//! * Charts nunca misturam materiais (`Face::material`) quando
//!   [`AtlasOptions::split_by_material`] (padrão).
//!
//! `unsafe` fica isolado em [`ffi`].

mod ffi;

use crate::{guarded, NativeError, NativeResult};
use anigo_modeling::EditMesh;
use glam::Vec2;

const LIB: &str = "xatlas";

/// Teto de corners aceitos (proteção contra entradas absurdas).
pub const MAX_CORNERS: usize = 20_000_000;

/// Opções de [`compute_atlas`] / [`auto_unwrap`].
#[derive(Debug, Clone, PartialEq)]
pub struct AtlasOptions {
    // ---- geração de charts
    /// Área máxima de um chart, em unidades² da malha. `0` = sem limite.
    pub max_chart_area: f32,
    /// Perímetro máximo de um chart, em unidades da malha. `0` = sem limite.
    pub max_boundary_length: f32,
    /// Peso do desvio de normal (mais alto → charts mais planos, mais charts).
    pub normal_deviation_weight: f32,
    pub roundness_weight: f32,
    pub straightness_weight: f32,
    /// Peso das costuras de normal (`> 1000` = costuras de normal sempre respeitadas).
    pub normal_seam_weight: f32,
    pub texture_seam_weight: f32,
    /// Custo máximo para crescer um chart (menor → mais charts).
    pub max_cost: f32,
    /// Iterações de crescimento/semeadura de charts (mais alto → charts melhores, mais lento).
    pub max_iterations: u32,
    /// Usa os UVs atuais do `EditMesh` como dica para formar charts (ilhas existentes).
    pub use_input_uvs: bool,
    /// Força enrolamento de UV consistente entre charts.
    pub fix_winding: bool,
    /// Charts não misturam materiais.
    pub split_by_material: bool,

    // ---- empacotamento
    /// Lado do atlas em texels. Com `texels_per_unit > 0` o tamanho é exato (e a
    /// operação falha se não couber em um atlas); com `texels_per_unit = 0` a
    /// densidade é estimada e o atlas final fica **próximo** de `resolution`.
    /// `0` = estimado a partir de `texels_per_unit` (ou ~1024 se ambos forem 0).
    pub resolution: u32,
    /// Densidade: texels por unidade de malha. `0` = estimada para caber em `resolution`.
    pub texels_per_unit: f32,
    /// Preenchimento entre charts, em texels (evita sangramento de mipmap/filtro).
    pub padding: u32,
    /// Charts maiores que isto (texels) são reduzidos. `0` = sem limite.
    pub max_chart_size: u32,
    /// Reserva espaço para os texels amostrados por filtro bilinear.
    pub bilinear: bool,
    /// Alinha charts a blocos de 4×4 (mais rápido, melhor para compressão de bloco).
    pub block_align: bool,
    /// Posicionamento por força bruta (melhor empacotamento, mais lento).
    pub brute_force: bool,
    /// Gira charts para melhorar o empacotamento.
    pub rotate_charts: bool,
    /// Alinha charts ao eixo do casco convexo.
    pub rotate_charts_to_axis: bool,
}

impl Default for AtlasOptions {
    fn default() -> Self {
        Self {
            max_chart_area: 0.0,
            max_boundary_length: 0.0,
            normal_deviation_weight: 2.0,
            roundness_weight: 0.01,
            straightness_weight: 6.0,
            normal_seam_weight: 4.0,
            texture_seam_weight: 0.5,
            max_cost: 2.0,
            max_iterations: 1,
            use_input_uvs: false,
            fix_winding: true,
            split_by_material: true,
            resolution: 1024,
            texels_per_unit: 0.0,
            padding: 2,
            max_chart_size: 0,
            bilinear: true,
            block_align: false,
            brute_force: false,
            rotate_charts: true,
            rotate_charts_to_axis: true,
        }
    }
}

impl AtlasOptions {
    fn validate(&self) -> NativeResult<()> {
        let non_neg = [
            ("max_chart_area", self.max_chart_area),
            ("max_boundary_length", self.max_boundary_length),
            ("normal_deviation_weight", self.normal_deviation_weight),
            ("roundness_weight", self.roundness_weight),
            ("straightness_weight", self.straightness_weight),
            ("normal_seam_weight", self.normal_seam_weight),
            ("texture_seam_weight", self.texture_seam_weight),
            ("max_cost", self.max_cost),
            ("texels_per_unit", self.texels_per_unit),
        ];
        for (name, v) in non_neg {
            if !v.is_finite() || v < 0.0 {
                return Err(NativeError::InvalidInput(format!(
                    "AtlasOptions::{name} deve ser finito e >= 0 (veio {v})"
                )));
            }
        }
        if self.resolution > 16_384 {
            return Err(NativeError::LimitExceeded {
                what: "resolution",
                got: self.resolution as u64,
                max: 16_384,
            });
        }
        if self.padding > 256 {
            return Err(NativeError::LimitExceeded {
                what: "padding",
                got: self.padding as u64,
                max: 256,
            });
        }
        if self.max_iterations > 64 {
            return Err(NativeError::LimitExceeded {
                what: "max_iterations",
                got: self.max_iterations as u64,
                max: 64,
            });
        }
        Ok(())
    }

    fn to_ffi(&self) -> ffi::Params {
        ffi::Params {
            max_chart_area: self.max_chart_area,
            max_boundary_length: self.max_boundary_length,
            normal_deviation_weight: self.normal_deviation_weight,
            roundness_weight: self.roundness_weight,
            straightness_weight: self.straightness_weight,
            normal_seam_weight: self.normal_seam_weight,
            texture_seam_weight: self.texture_seam_weight,
            max_cost: self.max_cost,
            max_iterations: self.max_iterations,
            use_input_mesh_uvs: self.use_input_uvs,
            fix_winding: self.fix_winding,
            max_chart_size: self.max_chart_size,
            padding: self.padding,
            texels_per_unit: self.texels_per_unit,
            resolution: self.resolution,
            bilinear: self.bilinear,
            block_align: self.block_align,
            brute_force: self.brute_force,
            rotate_charts: self.rotate_charts,
            rotate_charts_to_axis: self.rotate_charts_to_axis,
        }
    }
}

/// UVs calculados para uma malha, ainda não aplicados.
#[derive(Debug, Clone, PartialEq)]
pub struct AtlasResult {
    /// `[face][corner]` → UV normalizado em `[0, 1]`. Faces não mapeadas têm UV `(0, 0)`
    /// aqui (e [`apply_atlas`] as ignora).
    pub uvs_per_corner: Vec<Vec<Vec2>>,
    /// Faces fora do atlas (ocultas ou degeneradas): mantêm o UV original.
    pub unmapped_faces: Vec<u32>,
    /// N-gons cujos triângulos o xatlas separou em charts diferentes: seus UVs por
    /// corner podem ficar distorcidos. Vazio no caso normal (n-gons planares).
    pub split_faces: Vec<u32>,
    /// Número de charts (ilhas de UV).
    pub chart_count: u32,
    /// Tamanho do atlas em texels `(largura, altura)`.
    pub atlas_size: (u32, u32),
    /// Fração do atlas coberta por charts, em `[0, 1]`.
    pub utilization: f32,
    /// Densidade efetiva (texels por unidade de malha).
    pub texels_per_unit: f32,
}

impl AtlasResult {
    /// `true` se todas as faces receberam UV.
    pub fn fully_mapped(&self) -> bool {
        self.unmapped_faces.is_empty()
    }
}

/// Calcula o atlas sem modificar a malha.
pub fn compute_atlas(mesh: &EditMesh, opts: &AtlasOptions) -> NativeResult<AtlasResult> {
    guarded(LIB, || compute_atlas_inner(mesh, opts))
}

fn compute_atlas_inner(mesh: &EditMesh, opts: &AtlasOptions) -> NativeResult<AtlasResult> {
    opts.validate()?;
    if mesh.verts.is_empty() || mesh.faces.is_empty() {
        return Err(NativeError::EmptyInput("EditMesh sem vértices ou sem faces"));
    }
    mesh.validate()?;

    let corner_total: usize = mesh.faces.iter().map(|f| f.corners.len()).sum();
    if corner_total > MAX_CORNERS {
        return Err(NativeError::LimitExceeded {
            what: "corners",
            got: corner_total as u64,
            max: MAX_CORNERS as u64,
        });
    }
    if let Some(f) = mesh.faces.iter().find(|f| f.corners.len() > u8::MAX as usize) {
        return Err(NativeError::LimitExceeded {
            what: "corners por face",
            got: f.corners.len() as u64,
            max: u8::MAX as u64,
        });
    }

    let positions: Vec<[f32; 3]> = mesh.verts.iter().map(|v| v.co.to_array()).collect();

    // O xatlas recebe TRIÂNGULOS: o modo polígono da versão vendorizada tem bugs
    // (índices de saída não inicializados). Triangulamos aqui, de forma determinística
    // (a mesma triangulação do `to_core_mesh`), e mapeamos de volta por corner.
    struct Tri {
        face: u32,
        local: [usize; 3],
    }
    let mut tris: Vec<Tri> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut ignore: Vec<u8> = Vec::new();
    let mut material: Vec<u32> = Vec::new();
    let mut face_ignored: Vec<bool> = Vec::with_capacity(mesh.faces.len());
    for (fi, f) in mesh.faces.iter().enumerate() {
        let pts = mesh.face_positions(fi as u32);
        let skip = f.hidden || mesh.face_area(fi as u32) <= 0.0;
        face_ignored.push(skip);
        for local in anigo_modeling::convert::triangulate_polygon(&pts) {
            indices.extend(local.iter().map(|&k| f.corners[k].vert));
            ignore.push(skip as u8);
            material.push(if opts.split_by_material { f.material as u32 } else { 0 });
            tris.push(Tri { face: fi as u32, local });
        }
    }
    if tris.is_empty() || face_ignored.iter().all(|&b| b) {
        return Err(NativeError::Degenerate(
            "nenhuma face visível com área > 0 para desembrulhar".into(),
        ));
    }

    let out = ffi::generate(&positions, &indices, Some(&ignore), Some(&material), &opts.to_ffi())
        .map_err(|message| NativeError::Backend { lib: LIB, message })?;

    if out.atlas_count != 1 {
        return Err(NativeError::Backend {
            lib: LIB,
            message: format!(
                "o resultado não coube em um único atlas ({} sub-atlas): reduza texels_per_unit \
                 ou aumente resolution",
                out.atlas_count
            ),
        });
    }
    if out.width == 0 || out.height == 0 {
        return Err(NativeError::Backend {
            lib: LIB,
            message: "atlas com tamanho zero".into(),
        });
    }
    let (w, h) = (out.width as f32, out.height as f32);

    // Por face: chart "primário" = o do primeiro triângulo mapeado. Cada corner recebe o
    // UV do primeiro triângulo desse chart que o contém; se o corner só existir em
    // triângulos de outros charts (n-gon dividido pelo xatlas), usa o primeiro que o
    // contém e a face é registrada em `split_faces`.
    let mut uvs_per_corner: Vec<Vec<Vec2>> = mesh
        .faces
        .iter()
        .map(|f| vec![Vec2::ZERO; f.corners.len()])
        .collect();
    let mut primary_chart: Vec<Option<i32>> = vec![None; mesh.faces.len()];
    let mut chart_of_corner: Vec<Vec<Option<i32>>> = mesh
        .faces
        .iter()
        .map(|f| vec![None; f.corners.len()])
        .collect();
    let mut mapped = vec![false; mesh.faces.len()];
    let mut split = vec![false; mesh.faces.len()];
    let uv_of = |ov: usize| -> Vec2 {
        let [u, v] = out.uv[ov];
        Vec2::new(u / w, v / h)
    };
    for (t, tri) in tris.iter().enumerate() {
        let f = tri.face as usize;
        if face_ignored[f] {
            continue;
        }
        let ovs = [
            out.indices[3 * t] as usize, // validado em ffi::generate (< vertex_count)
            out.indices[3 * t + 1] as usize,
            out.indices[3 * t + 2] as usize,
        ];
        if ovs.iter().any(|&ov| out.atlas_index[ov] < 0) {
            continue; // o xatlas descartou o triângulo (degenerado)
        }
        let chart = out.chart_index[ovs[0]];
        match primary_chart[f] {
            None => primary_chart[f] = Some(chart),
            Some(c) if c != chart => split[f] = true,
            _ => {}
        }
        for (k, &ov) in ovs.iter().enumerate() {
            let corner = tri.local[k];
            let take = match chart_of_corner[f][corner] {
                None => true,
                // prefere o chart primário se o corner já tinha UV de outro chart
                Some(c) => Some(c) != primary_chart[f] && Some(chart) == primary_chart[f],
            };
            if take {
                uvs_per_corner[f][corner] = uv_of(ov);
                chart_of_corner[f][corner] = Some(chart);
            }
        }
        mapped[f] = true;
    }
    let mut unmapped_faces = Vec::new();
    let mut split_faces = Vec::new();
    for fi in 0..mesh.faces.len() {
        // todo corner precisa ter recebido UV; senão a face inteira é "não mapeada"
        let complete = chart_of_corner[fi].iter().all(Option::is_some);
        if !mapped[fi] || !complete {
            unmapped_faces.push(fi as u32);
            uvs_per_corner[fi].iter_mut().for_each(|uv| *uv = Vec2::ZERO);
        } else if split[fi] {
            split_faces.push(fi as u32);
        }
    }
    if unmapped_faces.len() == mesh.faces.len() {
        return Err(NativeError::Backend {
            lib: LIB,
            message: "o xatlas não mapeou nenhuma face".into(),
        });
    }

    Ok(AtlasResult {
        uvs_per_corner,
        unmapped_faces,
        split_faces,
        chart_count: out.chart_count,
        atlas_size: (out.width, out.height),
        utilization: out.utilization,
        texels_per_unit: out.texels_per_unit,
    })
}

/// Escreve `result` em `Corner::uv` (faces não mapeadas ficam como estão).
/// Valida o formato **antes** de alterar: em `Err` a malha fica intacta.
pub fn apply_atlas(mesh: &mut EditMesh, result: &AtlasResult) -> NativeResult<()> {
    if result.uvs_per_corner.len() != mesh.faces.len() {
        return Err(NativeError::InvalidInput(format!(
            "atlas tem {} faces, a malha tem {}",
            result.uvs_per_corner.len(),
            mesh.faces.len()
        )));
    }
    for (fi, (uvs, f)) in result.uvs_per_corner.iter().zip(&mesh.faces).enumerate() {
        if uvs.len() != f.corners.len() {
            return Err(NativeError::InvalidInput(format!(
                "face {fi}: atlas tem {} corners, a malha tem {}",
                uvs.len(),
                f.corners.len()
            )));
        }
        if uvs.iter().any(|uv| !uv.is_finite()) {
            return Err(NativeError::InvalidInput(format!(
                "face {fi}: UV não finito no atlas"
            )));
        }
    }
    let mut skip = vec![false; mesh.faces.len()];
    for &f in &result.unmapped_faces {
        if let Some(s) = skip.get_mut(f as usize) {
            *s = true;
        }
    }
    for (fi, (face, uvs)) in mesh
        .faces
        .iter_mut()
        .zip(&result.uvs_per_corner)
        .enumerate()
    {
        if skip[fi] {
            continue;
        }
        for (corner, uv) in face.corners.iter_mut().zip(uvs) {
            corner.uv = *uv;
        }
    }
    Ok(())
}

/// Desembrulha automaticamente: devolve uma cópia da malha com `Corner::uv`
/// escrito. Vértices, faces, corners, normais e atributos de aresta ficam idênticos.
pub fn auto_unwrap(mesh: &EditMesh, opts: &AtlasOptions) -> NativeResult<EditMesh> {
    let result = compute_atlas(mesh, opts)?;
    let mut out = mesh.clone();
    apply_atlas(&mut out, &result)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_modeling::primitives;
    use glam::Vec3;

    /// Cobertura por face num grid `res × res`: devolve (pixels cobertos, pixels em ≥ 2 faces).
    fn overlap_stats(mesh: &EditMesh, res: usize) -> (usize, usize) {
        let mut owner: Vec<Option<u32>> = vec![None; res * res];
        let mut covered = 0usize;
        let mut overlapped = 0usize;
        for (fi, f) in mesh.faces.iter().enumerate() {
            let uvs: Vec<Vec2> = f.corners.iter().map(|c| c.uv).collect();
            // fan: serve para faces convexas (cubo/prisma)
            for k in 1..uvs.len() - 1 {
                let (a, b, c) = (uvs[0], uvs[k], uvs[k + 1]);
                let min = a.min(b).min(c);
                let max = a.max(b).max(c);
                let x0 = ((min.x * res as f32).floor() as isize).max(0) as usize;
                let x1 = ((max.x * res as f32).ceil() as isize).min(res as isize - 1).max(0) as usize;
                let y0 = ((min.y * res as f32).floor() as isize).max(0) as usize;
                let y1 = ((max.y * res as f32).ceil() as isize).min(res as isize - 1).max(0) as usize;
                let area = (b - a).perp_dot(c - a);
                if area.abs() < 1e-12 {
                    continue;
                }
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let p = Vec2::new((x as f32 + 0.5) / res as f32, (y as f32 + 0.5) / res as f32);
                        let w0 = (b - p).perp_dot(c - p) / area;
                        let w1 = (c - p).perp_dot(a - p) / area;
                        let w2 = (a - p).perp_dot(b - p) / area;
                        let eps = 1e-4;
                        if w0 > eps && w1 > eps && w2 > eps {
                            match owner[y * res + x] {
                                None => {
                                    owner[y * res + x] = Some(fi as u32);
                                    covered += 1;
                                }
                                Some(o) if o != fi as u32 => overlapped += 1,
                                _ => {}
                            }
                        }
                    }
                }
            }
        }
        (covered, overlapped)
    }

    fn assert_topology_unchanged(before: &EditMesh, after: &EditMesh) {
        assert_eq!(before.verts, after.verts);
        assert_eq!(before.edge_attrs, after.edge_attrs);
        assert_eq!(before.faces.len(), after.faces.len());
        for (a, b) in before.faces.iter().zip(&after.faces) {
            assert_eq!(a.material, b.material);
            assert_eq!(a.smooth, b.smooth);
            assert_eq!(a.corners.len(), b.corners.len());
            for (ca, cb) in a.corners.iter().zip(&b.corners) {
                assert_eq!(ca.vert, cb.vert);
                assert_eq!(ca.normal, cb.normal);
            }
        }
    }

    #[test]
    fn cube_unwraps_inside_unit_square_without_overlap() {
        let cube = primitives::cube(1.0);
        let r = compute_atlas(&cube, &AtlasOptions::default()).expect("atlas do cubo");
        assert!(r.fully_mapped(), "faces sem UV: {:?}", r.unmapped_faces);
        assert!(r.split_faces.is_empty(), "quads não deveriam ser divididos: {:?}", r.split_faces);
        assert_eq!(r.uvs_per_corner.len(), 6);
        assert!(r.chart_count >= 1 && r.chart_count <= 6, "charts: {}", r.chart_count);
        // `resolution` com texels_per_unit = 0 é aproximado: o xatlas estima a densidade
        // e o atlas final fica perto (não exatamente) do lado pedido.
        assert!(
            (512..=2048).contains(&r.atlas_size.0) && (512..=2048).contains(&r.atlas_size.1),
            "tamanho do atlas inesperado: {:?}",
            r.atlas_size
        );
        assert!(r.utilization > 0.0 && r.utilization <= 1.0);
        for uvs in &r.uvs_per_corner {
            assert_eq!(uvs.len(), 4);
            for uv in uvs {
                assert!(
                    uv.x >= -1e-4 && uv.x <= 1.0 + 1e-4 && uv.y >= -1e-4 && uv.y <= 1.0 + 1e-4,
                    "UV fora de [0,1]: {uv:?}"
                );
            }
        }

        let unwrapped = auto_unwrap(&cube, &AtlasOptions::default()).expect("auto_unwrap");
        assert_topology_unchanged(&cube, &unwrapped);
        unwrapped.validate().expect("malha válida");
        // todos os corners têm UV escrito (o cubo original tem tudo em (0,0))
        for f in &unwrapped.faces {
            let first = f.corners[0].uv;
            assert!(f.corners.iter().any(|c| c.uv != first), "face com UV colapsado");
        }
        let (covered, overlapped) = overlap_stats(&unwrapped, 1024);
        assert!(covered > 1000, "cobertura baixa: {covered}");
        assert!(
            (overlapped as f32) <= covered as f32 * 0.005,
            "sobreposição: {overlapped} de {covered} texels"
        );
    }

    #[test]
    fn auto_unwrap_is_deterministic() {
        let sphere = primitives::uv_sphere(1.0, 12, 16);
        let a = auto_unwrap(&sphere, &AtlasOptions::default()).expect("a");
        let b = auto_unwrap(&sphere, &AtlasOptions::default()).expect("b");
        assert_eq!(a, b);
    }

    #[test]
    fn ngon_prism_keeps_polygons_whole() {
        let prism = primitives::cylinder(0.5, 1.0, 6, 1, true);
        let max_corners = prism.faces.iter().map(|f| f.len()).max().unwrap_or(0);
        let r = compute_atlas(&prism, &AtlasOptions::default()).expect("atlas do prisma");
        assert!(r.fully_mapped());
        let unwrapped = auto_unwrap(&prism, &AtlasOptions::default()).expect("auto_unwrap");
        assert_topology_unchanged(&prism, &unwrapped);
        assert_eq!(
            unwrapped.faces.iter().map(|f| f.len()).max().unwrap_or(0),
            max_corners,
            "n-gons preservados"
        );
        for f in &unwrapped.faces {
            for c in &f.corners {
                assert!(c.uv.x >= -1e-4 && c.uv.x <= 1.0001 && c.uv.y >= -1e-4 && c.uv.y <= 1.0001);
            }
        }
    }

    #[test]
    fn texel_density_drives_atlas_size() {
        let cube = primitives::cube(1.0);
        let opts = AtlasOptions {
            resolution: 0,
            texels_per_unit: 128.0,
            padding: 1,
            ..AtlasOptions::default()
        };
        let r = compute_atlas(&cube, &opts).expect("atlas");
        assert!((r.texels_per_unit - 128.0).abs() < 1e-3);
        // 6 faces de 1×1 a 128 texels/unidade: o atlas precisa de pelo menos ~3 faces de lado
        assert!(r.atlas_size.0 >= 128 && r.atlas_size.1 >= 128);
        // não coube em um atlas fixo pequeno → erro claro
        let tiny = AtlasOptions {
            resolution: 64,
            texels_per_unit: 128.0,
            ..AtlasOptions::default()
        };
        assert!(matches!(compute_atlas(&cube, &tiny), Err(NativeError::Backend { .. })));
    }

    #[test]
    fn hidden_and_degenerate_faces_keep_their_uv() {
        let mut m = primitives::cube(1.0);
        let marker = Vec2::new(0.25, 0.75);
        m.faces[2].hidden = true;
        for c in &mut m.faces[2].corners {
            c.uv = marker;
        }
        let out = auto_unwrap(&m, &AtlasOptions::default()).expect("auto_unwrap");
        assert!(out.faces[2].corners.iter().all(|c| c.uv == marker));
        let r = compute_atlas(&m, &AtlasOptions::default()).expect("atlas");
        assert_eq!(r.unmapped_faces, vec![2]);
    }

    #[test]
    fn invalid_inputs_return_err() {
        let opts = AtlasOptions::default();
        assert!(matches!(
            compute_atlas(&EditMesh::new("vazia"), &opts),
            Err(NativeError::EmptyInput(_))
        ));
        // só vértices
        let mut only_verts = EditMesh::new("v");
        only_verts.add_vert(Vec3::ZERO);
        assert!(compute_atlas(&only_verts, &opts).is_err());
        // posição NaN
        let mut nan = primitives::cube(1.0);
        nan.verts[0].co.x = f32::NAN;
        assert!(compute_atlas(&nan, &opts).is_err());
        // todas ocultas
        let mut hidden = primitives::cube(1.0);
        hidden.faces.iter_mut().for_each(|f| f.hidden = true);
        assert!(matches!(compute_atlas(&hidden, &opts), Err(NativeError::Degenerate(_))));
        // triângulo de área zero (colinear)
        let mut flat = EditMesh::new("flat");
        let a = flat.add_vert(Vec3::new(0.0, 0.0, 0.0));
        let b = flat.add_vert(Vec3::new(1.0, 0.0, 0.0));
        let c = flat.add_vert(Vec3::new(2.0, 0.0, 0.0));
        flat.add_face(&[a, b, c]).expect("face");
        assert!(matches!(compute_atlas(&flat, &opts), Err(NativeError::Degenerate(_))));
        // opções inválidas
        let cube = primitives::cube(1.0);
        for bad in [
            AtlasOptions { texels_per_unit: -1.0, ..AtlasOptions::default() },
            AtlasOptions { max_chart_area: f32::NAN, ..AtlasOptions::default() },
            AtlasOptions { resolution: 1_000_000, ..AtlasOptions::default() },
            AtlasOptions { padding: 10_000, ..AtlasOptions::default() },
        ] {
            assert!(compute_atlas(&cube, &bad).is_err());
        }
    }

    #[test]
    fn apply_atlas_rejects_mismatched_shape_and_leaves_mesh_intact() {
        let cube = primitives::cube(1.0);
        let mut r = compute_atlas(&cube, &AtlasOptions::default()).expect("atlas");
        let mut target = cube.clone();
        r.uvs_per_corner[1].pop();
        assert!(apply_atlas(&mut target, &r).is_err());
        assert_eq!(target, cube, "falha não pode alterar a malha");
        r.uvs_per_corner.pop();
        assert!(apply_atlas(&mut target, &r).is_err());
        assert_eq!(target, cube);
    }

    #[test]
    fn sphere_with_many_faces_maps_every_corner() {
        let sphere = primitives::uv_sphere(1.0, 24, 32);
        let r = compute_atlas(&sphere, &AtlasOptions::default()).expect("atlas");
        assert!(r.fully_mapped());
        assert!(r.chart_count > 1, "esfera fechada precisa de costuras");
        let out = auto_unwrap(&sphere, &AtlasOptions::default()).expect("auto_unwrap");
        let (covered, overlapped) = overlap_stats(&out, 1024);
        assert!(covered > 10_000);
        assert!((overlapped as f32) <= covered as f32 * 0.01, "{overlapped}/{covered}");
    }
}
