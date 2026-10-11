//! Simplificação, LOD e otimização de índices de `anigo_core::Mesh` com o
//! **meshoptimizer** (C++ vendorizado pelo crate `meshopt`).
//!
//! # Layout e atributos
//!
//! `anigo_core::Vertex` tem 72 bytes (posição, normal, uv, cor NPR, joints, pesos).
//! O meshoptimizer lê a **posição** direto do buffer (stride 72, offset 0) e os
//! **atributos** (normal, uv, cor) de um buffer auxiliar de floats montado aqui,
//! ponderado por [`AttributeWeights`]: um colapso que distorce normal/uv custa mais
//! erro, então costuras de UV e arestas duras tendem a sobreviver.
//!
//! Vértices **não são movidos nem sintetizados**: cada vértice da saída é
//! bit-a-bit um vértice da entrada (normais, UV, cor, joints e pesos de skin
//! preservados exatamente). Atributos customizados por vértice
//! (`Mesh::custom_attributes`) são re-indexados junto.
//!
//! # Erro
//!
//! O erro do meshoptimizer é **relativo à extensão da malha** (`simplify_scale`):
//! `0.01` = 1 % do tamanho da malha. [`SimplifyResult::error`] devolve esse valor
//! relativo e [`SimplifyResult::error_absolute`] o equivalente em unidades de mundo.
//! `target_error` é um **teto**: o algoritmo para ao atingir o teto mesmo que a
//! razão pedida não tenha sido alcançada ([`SimplifyResult::reached_target`]).
//!
//! # Segurança
//!
//! O código C++ não valida índices. Por isso [`validate`] roda sempre antes de
//! qualquer chamada nativa: índices em faixa, múltiplo de 3, posições finitas,
//! limites de tamanho, atributos customizados consistentes.

use crate::{guarded, NativeError, NativeResult};
use anigo_core::mesh::CustomAttribute;
use anigo_core::{Mesh, Vertex};
use meshopt::{SimplifyOptions as Flags, VertexDataAdapter};
use std::collections::HashMap;
use std::mem::size_of;

const LIB: &str = "meshopt";

/// Teto de triângulos aceitos (proteção contra entradas absurdas).
pub const MAX_TRIANGLES: usize = 50_000_000;
/// Teto de vértices aceitos.
pub const MAX_VERTICES: usize = 100_000_000;

/// Peso de cada grupo de atributos no custo de colapso (relativo ao erro de posição).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AttributeWeights {
    /// Normal (3 floats). `0` = ignora a normal.
    pub normal: f32,
    /// UV (2 floats). `0` = ignora o UV.
    pub uv: f32,
    /// Cor NPR RGBA (4 floats: AO, shift de sombra, espessura de contorno, máscara).
    pub color: f32,
}

impl Default for AttributeWeights {
    fn default() -> Self {
        Self {
            normal: 1.0,
            uv: 1.0,
            color: 0.25,
        }
    }
}

impl AttributeWeights {
    /// Só posição conta (mais agressivo, ignora sombreamento e textura).
    pub const POSITION_ONLY: AttributeWeights = AttributeWeights {
        normal: 0.0,
        uv: 0.0,
        color: 0.0,
    };

    fn validate(&self) -> NativeResult<()> {
        for (name, w) in [
            ("normal", self.normal),
            ("uv", self.uv),
            ("color", self.color),
        ] {
            if !w.is_finite() || w < 0.0 {
                return Err(NativeError::InvalidInput(format!(
                    "peso de atributo '{name}' deve ser finito e >= 0 (veio {w})"
                )));
            }
        }
        Ok(())
    }
}

/// Opções de [`simplify_mesh`].
#[derive(Debug, Clone, PartialEq)]
pub struct SimplifyOptions {
    /// Congela os vértices da borda aberta da malha (buracos, bordas de cartões).
    pub lock_border: bool,
    /// Congela todo vértice sobre uma costura de UV (mesma posição, UVs diferentes):
    /// as ilhas de UV mantêm o contorno exato. Sem isto o meshoptimizer ainda
    /// preserva a *topologia* das costuras, mas pode deslizá-las.
    pub preserve_uv_seams: bool,
    /// Pesos de normal/uv/cor no custo de colapso.
    pub attribute_weights: AttributeWeights,
    /// Favorece triângulos mais regulares (melhor para skinning/deformação).
    pub regularize: bool,
    /// Remove componentes pequenos e desconectados (muda a topologia/fechamento!).
    pub prune: bool,
    /// Vértices extras a congelar (índices de `mesh.vertices`). Vazio = nenhum;
    /// senão deve ter exatamente `mesh.vertices.len()` entradas.
    pub locked_vertices: Vec<bool>,
}

impl Default for SimplifyOptions {
    fn default() -> Self {
        Self {
            lock_border: false,
            preserve_uv_seams: true,
            attribute_weights: AttributeWeights::default(),
            regularize: false,
            prune: false,
            locked_vertices: Vec::new(),
        }
    }
}

/// Resultado de [`simplify_mesh`].
#[derive(Debug, Clone, PartialEq)]
pub struct SimplifyResult {
    /// Malha simplificada (vértices compactados, ordem original preservada).
    pub mesh: Mesh,
    /// Erro máximo introduzido, **relativo** à extensão da malha de entrada.
    pub error: f32,
    /// Mesmo erro em unidades de mundo (`error * extensão`).
    pub error_absolute: f32,
    /// Razão de triângulos alcançada (`resultado / entrada`, em (0, 1]).
    pub ratio: f32,
    pub source_triangles: usize,
    pub result_triangles: usize,
    /// `false` se o `target_error` impediu de chegar à razão pedida.
    pub reached_target: bool,
}

// ------------------------------------------------------------------ validação

/// Valida uma malha antes de qualquer chamada nativa. Devolve o número de
/// triângulos **utilizáveis** (índices distintos e área > 0).
pub fn validate(mesh: &Mesh) -> NativeResult<usize> {
    if mesh.vertices.is_empty() {
        return Err(NativeError::EmptyInput("malha sem vértices"));
    }
    if mesh.indices.is_empty() {
        return Err(NativeError::EmptyInput("malha sem índices"));
    }
    if mesh.indices.len() % 3 != 0 {
        return Err(NativeError::InvalidInput(format!(
            "número de índices ({}) não é múltiplo de 3",
            mesh.indices.len()
        )));
    }
    if mesh.vertices.len() > MAX_VERTICES {
        return Err(NativeError::LimitExceeded {
            what: "vértices",
            got: mesh.vertices.len() as u64,
            max: MAX_VERTICES as u64,
        });
    }
    if mesh.indices.len() / 3 > MAX_TRIANGLES {
        return Err(NativeError::LimitExceeded {
            what: "triângulos",
            got: (mesh.indices.len() / 3) as u64,
            max: MAX_TRIANGLES as u64,
        });
    }
    let n = mesh.vertices.len();
    if let Some(&bad) = mesh.indices.iter().find(|&&i| i as usize >= n) {
        return Err(NativeError::InvalidInput(format!(
            "índice {bad} fora do intervalo (malha tem {n} vértices)"
        )));
    }
    if let Some(i) = mesh
        .vertices
        .iter()
        .position(|v| v.position.iter().any(|c| !c.is_finite()))
    {
        return Err(NativeError::InvalidInput(format!(
            "vértice {i} com posição não finita"
        )));
    }
    for (name, attr) in &mesh.custom_attributes {
        custom_stride(name, attr, n)?;
    }
    let usable = mesh
        .indices
        .chunks_exact(3)
        .filter(|t| triangle_is_usable(mesh, t))
        .count();
    if usable == 0 {
        return Err(NativeError::Degenerate(
            "todos os triângulos são degenerados (índices repetidos ou área zero)".into(),
        ));
    }
    Ok(usable)
}

fn triangle_is_usable(mesh: &Mesh, t: &[u32]) -> bool {
    if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] {
        return false;
    }
    let p = |i: u32| {
        let v = mesh.vertices[i as usize].position;
        [v[0] as f64, v[1] as f64, v[2] as f64]
    };
    let (a, b, c) = (p(t[0]), p(t[1]), p(t[2]));
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        u[1] * w[2] - u[2] * w[1],
        u[2] * w[0] - u[0] * w[2],
        u[0] * w[1] - u[1] * w[0],
    ];
    let len2 = cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2];
    len2 > 0.0 && len2.is_finite()
}

/// Bytes por vértice de um atributo customizado (exige `count == vertex_count`).
fn custom_stride(name: &str, attr: &CustomAttribute, vertex_count: usize) -> NativeResult<usize> {
    if attr.count != vertex_count || attr.count == 0 || attr.data.len() % attr.count != 0 {
        return Err(NativeError::InvalidInput(format!(
            "atributo customizado '{name}' não é por-vértice (count {}, {} bytes, {} vértices)",
            attr.count,
            attr.data.len(),
            vertex_count
        )));
    }
    Ok(attr.data.len() / attr.count)
}

// ----------------------------------------------------------- reindexação

/// Constrói `Mesh` novo com os vértices `order[k]` (índices antigos) e índices já
/// no espaço novo. Re-indexa os atributos customizados.
fn rebuild(src: &Mesh, order: &[u32], indices: Vec<u32>) -> NativeResult<Mesh> {
    let vertices: Vec<Vertex> = order.iter().map(|&o| src.vertices[o as usize]).collect();
    let mut out = Mesh::new(src.name.clone(), vertices, indices);
    for (name, attr) in &src.custom_attributes {
        let stride = custom_stride(name, attr, src.vertices.len())?;
        let mut data = Vec::with_capacity(order.len() * stride);
        for &o in order {
            let s = o as usize * stride;
            data.extend_from_slice(&attr.data[s..s + stride]);
        }
        out.custom_attributes.insert(
            name.clone(),
            CustomAttribute {
                name: attr.name.clone(),
                component_type: attr.component_type,
                attribute_type: attr.attribute_type.clone(),
                count: order.len(),
                data,
            },
        );
    }
    Ok(out)
}

/// Compacta: mantém só os vértices usados por `indices`, na ordem original.
fn compact(src: &Mesh, old_indices: &[u32]) -> NativeResult<Mesh> {
    let mut used = vec![false; src.vertices.len()];
    for &i in old_indices {
        used[i as usize] = true;
    }
    let mut new_id = vec![u32::MAX; src.vertices.len()];
    let mut order = Vec::new();
    for (old, u) in used.iter().enumerate() {
        if *u {
            new_id[old] = order.len() as u32;
            order.push(old as u32);
        }
    }
    let indices = old_indices.iter().map(|&i| new_id[i as usize]).collect();
    rebuild(src, &order, indices)
}

/// Reordena vértices pela primeira aparição no buffer de índices (é exatamente o
/// que o "vertex fetch optimizer" faz) e descarta os não usados.
fn reorder_by_first_use(src: &Mesh, old_indices: &[u32]) -> NativeResult<Mesh> {
    let mut new_id = vec![u32::MAX; src.vertices.len()];
    let mut order = Vec::new();
    let mut indices = Vec::with_capacity(old_indices.len());
    for &i in old_indices {
        let slot = &mut new_id[i as usize];
        if *slot == u32::MAX {
            *slot = order.len() as u32;
            order.push(i);
        }
        indices.push(*slot);
    }
    rebuild(src, &order, indices)
}

// ------------------------------------------------------------- simplify

fn uv_seam_locks(mesh: &Mesh) -> Vec<bool> {
    struct Group {
        uv: [u32; 2],
        members: Vec<u32>,
        seam: bool,
    }
    let norm = |f: f32| (f + 0.0).to_bits(); // -0.0 -> +0.0
    let mut groups: HashMap<[u32; 3], Group> = HashMap::new();
    for (i, v) in mesh.vertices.iter().enumerate() {
        let key = [
            norm(v.position[0]),
            norm(v.position[1]),
            norm(v.position[2]),
        ];
        let uv = [norm(v.uv[0]), norm(v.uv[1])];
        let g = groups.entry(key).or_insert_with(|| Group {
            uv,
            members: Vec::new(),
            seam: false,
        });
        if g.uv != uv {
            g.seam = true;
        }
        g.members.push(i as u32);
    }
    let mut locks = vec![false; mesh.vertices.len()];
    for g in groups.values().filter(|g| g.seam) {
        for &m in &g.members {
            locks[m as usize] = true;
        }
    }
    locks
}

fn simplify_core(
    mesh: &Mesh,
    target_triangles: usize,
    target_error: f32,
    opts: &SimplifyOptions,
) -> NativeResult<SimplifyResult> {
    let usable = validate(mesh)?;
    opts.attribute_weights.validate()?;
    if !target_error.is_finite() || !(0.0..=1.0).contains(&target_error) {
        return Err(NativeError::InvalidInput(format!(
            "target_error deve estar em [0, 1] (relativo à malha); veio {target_error}"
        )));
    }
    if !opts.locked_vertices.is_empty() && opts.locked_vertices.len() != mesh.vertices.len() {
        return Err(NativeError::InvalidInput(format!(
            "locked_vertices tem {} entradas, a malha tem {} vértices",
            opts.locked_vertices.len(),
            mesh.vertices.len()
        )));
    }

    // Só triângulos utilizáveis seguem para o nativo.
    let clean: Vec<u32> = mesh
        .indices
        .chunks_exact(3)
        .filter(|t| triangle_is_usable(mesh, t))
        .flatten()
        .copied()
        .collect();
    let source_triangles = usable;
    let target_triangles = target_triangles.clamp(1, source_triangles);

    let bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
    let adapter = VertexDataAdapter::new(bytes, size_of::<Vertex>(), 0).map_err(|e| {
        NativeError::Backend {
            lib: LIB,
            message: e.to_string(),
        }
    })?;
    let scale = meshopt::simplify_scale(&adapter);

    // Nada a fazer: devolve a malha compactada sem chamar o nativo.
    if target_triangles >= source_triangles {
        let mesh = compact(mesh, &clean)?;
        return Ok(SimplifyResult {
            mesh,
            error: 0.0,
            error_absolute: 0.0,
            ratio: 1.0,
            source_triangles,
            result_triangles: source_triangles,
            reached_target: true,
        });
    }

    // Travas: costuras de UV + travas do chamador. SEMPRE um vetor completo — o
    // nativo lê `vertex_count` bytes a partir do ponteiro.
    let mut locks = vec![false; mesh.vertices.len()];
    if opts.preserve_uv_seams {
        locks = uv_seam_locks(mesh);
    }
    for (l, extra) in locks.iter_mut().zip(opts.locked_vertices.iter()) {
        *l |= *extra;
    }

    // Atributos: só os grupos com peso > 0.
    let w = opts.attribute_weights;
    let mut weights: Vec<f32> = Vec::new();
    if w.normal > 0.0 {
        weights.extend([w.normal; 3]);
    }
    if w.uv > 0.0 {
        weights.extend([w.uv; 2]);
    }
    if w.color > 0.0 {
        weights.extend([w.color; 4]);
    }
    let attr_floats = weights.len();
    let mut attrs: Vec<f32> = Vec::with_capacity(mesh.vertices.len() * attr_floats);
    if attr_floats > 0 {
        for v in &mesh.vertices {
            if w.normal > 0.0 {
                attrs.extend_from_slice(&v.normal);
            }
            if w.uv > 0.0 {
                attrs.extend_from_slice(&v.uv);
            }
            if w.color > 0.0 {
                attrs.extend_from_slice(&v.color);
            }
        }
    }

    let mut flags = Flags::None;
    if opts.lock_border {
        flags |= Flags::LockBorder;
    }
    if opts.regularize {
        flags |= Flags::Regularize;
    }
    if opts.prune {
        flags |= Flags::Prune;
    }

    let mut rel_error = 0.0f32;
    let target_indices = target_triangles * 3;
    let result = if attr_floats > 0 {
        meshopt::simplify_with_attributes_and_locks(
            &clean,
            &adapter,
            &attrs,
            &weights,
            attr_floats * size_of::<f32>(),
            &locks,
            target_indices,
            target_error,
            flags,
            Some(&mut rel_error),
        )
    } else {
        meshopt::simplify_with_locks(
            &clean,
            &adapter,
            &locks,
            target_indices,
            target_error,
            flags,
            Some(&mut rel_error),
        )
    };

    if result.is_empty() || result.len() % 3 != 0 {
        return Err(NativeError::Degenerate(
            "a simplificação eliminou toda a geometria (tente prune=false ou erro maior)".into(),
        ));
    }
    // Defesa em profundidade: o nativo só devolve índices da entrada, mas a
    // compactação indexa vetores — confere antes.
    if result.iter().any(|&i| i as usize >= mesh.vertices.len()) {
        return Err(NativeError::Backend {
            lib: LIB,
            message: "índice fora de faixa devolvido pelo simplificador".into(),
        });
    }

    let out = compact(mesh, &result)?;
    let result_triangles = result.len() / 3;
    Ok(SimplifyResult {
        mesh: out,
        error: rel_error,
        error_absolute: rel_error * scale,
        ratio: result_triangles as f32 / source_triangles as f32,
        source_triangles,
        result_triangles,
        reached_target: result_triangles <= target_triangles,
    })
}

/// Simplifica `mesh` para `target_ratio` dos triângulos (0 < razão ≤ 1) sem
/// ultrapassar `target_error` (relativo à extensão da malha, em [0, 1]).
///
/// Nunca entra em pânico: entrada vazia, degenerada, com índices inválidos ou
/// NaN devolve `Err`.
pub fn simplify_mesh(
    mesh: &Mesh,
    target_ratio: f32,
    target_error: f32,
    opts: &SimplifyOptions,
) -> NativeResult<SimplifyResult> {
    guarded(LIB, || {
        if !target_ratio.is_finite() || target_ratio <= 0.0 || target_ratio > 1.0 {
            return Err(NativeError::InvalidInput(format!(
                "target_ratio deve estar em (0, 1]; veio {target_ratio}"
            )));
        }
        let usable = validate(mesh)?;
        let target = ((usable as f64 * target_ratio as f64).round() as usize).max(1);
        simplify_core(mesh, target, target_error, opts)
    })
}

// -------------------------------------------------------------- optimize

/// Opções de [`optimize_mesh_with`].
#[derive(Debug, Clone, PartialEq)]
pub struct OptimizeOptions {
    /// Funde vértices idênticos bit-a-bit (inclui atributos customizados).
    pub dedupe_vertices: bool,
    /// Quanto o otimizador de overdraw pode piorar o cache de vértices
    /// (`1.05` = até 5 %). `<= 1.0` desliga o passo de overdraw.
    pub overdraw_threshold: f32,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self {
            dedupe_vertices: true,
            overdraw_threshold: 1.05,
        }
    }
}

/// Otimiza a malha para GPU: dedupe → cache de vértices → overdraw → vertex fetch.
/// **Preserva a semântica**: o conjunto de triângulos (com seus atributos e
/// enrolamento) é idêntico; só mudam a ordem dos triângulos e dos vértices.
pub fn optimize_mesh(mesh: &Mesh) -> NativeResult<Mesh> {
    optimize_mesh_with(mesh, &OptimizeOptions::default())
}

/// Versão com opções de [`optimize_mesh`].
pub fn optimize_mesh_with(mesh: &Mesh, opts: &OptimizeOptions) -> NativeResult<Mesh> {
    guarded(LIB, || {
        validate(mesh)?;
        if !opts.overdraw_threshold.is_finite() {
            return Err(NativeError::InvalidInput(
                "overdraw_threshold deve ser finito".into(),
            ));
        }

        // 1) dedupe (em Rust: chave = bytes do vértice + bytes dos atributos customizados).
        let mut working = if opts.dedupe_vertices {
            dedupe(mesh)?
        } else {
            reorder_by_first_use(mesh, &mesh.indices)?
        };

        // 2) cache de vértices (nativo). Índices já validados e re-mapeados.
        let vertex_count = working.vertices.len();
        let mut indices = meshopt::optimize_vertex_cache(&working.indices, vertex_count);

        // 3) overdraw (nativo): precisa das posições no espaço de índices atual.
        if opts.overdraw_threshold > 1.0 {
            let bytes: &[u8] = bytemuck::cast_slice(&working.vertices);
            let adapter = VertexDataAdapter::new(bytes, size_of::<Vertex>(), 0).map_err(|e| {
                NativeError::Backend {
                    lib: LIB,
                    message: e.to_string(),
                }
            })?;
            meshopt::optimize_overdraw_in_place(&mut indices, &adapter, opts.overdraw_threshold);
        }
        if indices.len() != working.indices.len()
            || indices.iter().any(|&i| i as usize >= vertex_count)
        {
            return Err(NativeError::Backend {
                lib: LIB,
                message: "otimizador devolveu índices inconsistentes".into(),
            });
        }

        // 4) vertex fetch: reordena vértices pela ordem de uso.
        working.indices = indices;
        reorder_by_first_use(&working, &working.indices)
    })
}

fn dedupe(mesh: &Mesh) -> NativeResult<Mesh> {
    let mut names: Vec<(&String, usize)> = Vec::new();
    for (name, attr) in &mesh.custom_attributes {
        names.push((name, custom_stride(name, attr, mesh.vertices.len())?));
    }
    names.sort(); // ordem estável de chaves
    let key_of = |i: usize| -> Vec<u8> {
        let mut key = bytemuck::bytes_of(&mesh.vertices[i]).to_vec();
        for (name, stride) in &names {
            if let Some(a) = mesh.custom_attributes.get(*name) {
                key.extend_from_slice(&a.data[i * stride..(i + 1) * stride]);
            }
        }
        key
    };
    let mut seen: HashMap<Vec<u8>, u32> = HashMap::new();
    let mut canonical = vec![u32::MAX; mesh.vertices.len()];
    for &i in &mesh.indices {
        let i = i as usize;
        if canonical[i] == u32::MAX {
            canonical[i] = *seen.entry(key_of(i)).or_insert(i as u32);
        }
    }
    let indices: Vec<u32> = mesh
        .indices
        .iter()
        .map(|&i| canonical[i as usize])
        .collect();
    reorder_by_first_use(mesh, &indices)
}

// ------------------------------------------------------------------- LOD

/// Um nível de detalhe gerado por [`generate_lods`].
#[derive(Debug, Clone, PartialEq)]
pub struct LodLevel {
    /// Posição na lista (0 = mais detalhado).
    pub level: usize,
    /// Razão pedida (relativa à malha original).
    pub requested_ratio: f32,
    /// Razão de triângulos efetivamente alcançada (relativa à original).
    pub ratio: f32,
    pub triangle_count: usize,
    pub mesh: Mesh,
    /// Limite superior do erro acumulado desde a malha original, relativo à
    /// extensão da original (soma dos erros de cada passo).
    pub error: f32,
    /// Idem em unidades de mundo.
    pub error_absolute: f32,
}

/// Opções de [`generate_lods_with`].
#[derive(Debug, Clone, PartialEq)]
pub struct LodOptions {
    pub simplify: SimplifyOptions,
    /// Teto de erro **por passo** (relativo, em [0, 1]).
    pub target_error: f32,
    /// Passa cada nível por [`optimize_mesh`].
    pub optimize: bool,
}

impl Default for LodOptions {
    fn default() -> Self {
        Self {
            simplify: SimplifyOptions::default(),
            target_error: 0.1,
            optimize: true,
        }
    }
}

/// Gera LODs para as `ratios` (cada uma em (0, 1], relativa à malha original).
/// A saída vem em ordem decrescente de razão (nível 0 = mais detalhado); razões
/// repetidas são fundidas. Cada nível é simplificado a partir do anterior.
pub fn generate_lods(mesh: &Mesh, ratios: &[f32]) -> NativeResult<Vec<LodLevel>> {
    generate_lods_with(mesh, ratios, &LodOptions::default())
}

/// Versão com opções de [`generate_lods`].
pub fn generate_lods_with(
    mesh: &Mesh,
    ratios: &[f32],
    opts: &LodOptions,
) -> NativeResult<Vec<LodLevel>> {
    guarded(LIB, || {
        if ratios.is_empty() {
            return Err(NativeError::EmptyInput("lista de razões de LOD vazia"));
        }
        if ratios.len() > 16 {
            return Err(NativeError::LimitExceeded {
                what: "níveis de LOD",
                got: ratios.len() as u64,
                max: 16,
            });
        }
        if let Some(r) = ratios
            .iter()
            .find(|r| !r.is_finite() || **r <= 0.0 || **r > 1.0)
        {
            return Err(NativeError::InvalidInput(format!(
                "razão de LOD deve estar em (0, 1]; veio {r}"
            )));
        }
        let usable = validate(mesh)?;
        let mut sorted: Vec<f32> = ratios.to_vec();
        sorted.sort_by(|a, b| b.total_cmp(a));
        sorted.dedup();

        let source_scale = {
            let bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
            let adapter = VertexDataAdapter::new(bytes, size_of::<Vertex>(), 0).map_err(|e| {
                NativeError::Backend {
                    lib: LIB,
                    message: e.to_string(),
                }
            })?;
            meshopt::simplify_scale(&adapter)
        };

        let mut levels: Vec<LodLevel> = Vec::with_capacity(sorted.len());
        let mut current = mesh.clone();
        let mut acc_error = 0.0f32; // relativo à original
        for (level, &ratio) in sorted.iter().enumerate() {
            let target = ((usable as f64 * ratio as f64).round() as usize).max(1);
            let current_tris = validate(&current)?;
            let step = if target >= current_tris {
                // nada a reduzir neste passo (ex.: 1.0): só limpa
                simplify_core(&current, current_tris, 0.0, &opts.simplify)?
            } else {
                simplify_core(&current, target, opts.target_error, &opts.simplify)?
            };
            // erro do passo é relativo à malha *corrente*; converte p/ a original
            let step_abs = step.error_absolute;
            if source_scale > 0.0 {
                acc_error += step_abs / source_scale;
            }
            current = step.mesh;
            let emitted = if opts.optimize {
                optimize_mesh(&current)?
            } else {
                current.clone()
            };
            let triangle_count = emitted.indices.len() / 3;
            levels.push(LodLevel {
                level,
                requested_ratio: ratio,
                ratio: triangle_count as f32 / usable as f32,
                triangle_count,
                mesh: emitted,
                error: acc_error,
                error_absolute: acc_error * source_scale,
            });
        }
        Ok(levels)
    })
}

// ----------------------------------------------------------------- testes

#[cfg(test)]
mod tests {
    use super::*;
    use anigo_modeling::convert::{to_core_mesh, ToCoreOptions};
    use anigo_modeling::primitives;
    use std::collections::{BTreeMap, BTreeSet};

    /// Esfera UV de ~20k triângulos (142 setores × 71 anéis → 19 880 tris).
    fn sphere_20k() -> Mesh {
        let em = primitives::uv_sphere(1.0, 71, 142);
        let m = to_core_mesh(&em, &ToCoreOptions::default()).expect("converte esfera");
        assert!(
            (19_000..=21_000).contains(&(m.indices.len() / 3)),
            "esfera de teste fora do tamanho esperado: {}",
            m.indices.len() / 3
        );
        m
    }

    fn bounds(m: &Mesh) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for v in &m.vertices {
            for k in 0..3 {
                lo[k] = lo[k].min(v.position[k]);
                hi[k] = hi[k].max(v.position[k]);
            }
        }
        (lo, hi)
    }

    /// (V − E + F, arestas com ≠ 2 triângulos) após soldar posições idênticas.
    fn euler_by_edges(m: &Mesh) -> (i64, usize) {
        let mut weld: HashMap<[u32; 3], u32> = HashMap::new();
        let ids: Vec<u32> = m
            .vertices
            .iter()
            .map(|v| {
                let key = [
                    (v.position[0] + 0.0).to_bits(),
                    (v.position[1] + 0.0).to_bits(),
                    (v.position[2] + 0.0).to_bits(),
                ];
                let next = weld.len() as u32;
                *weld.entry(key).or_insert(next)
            })
            .collect();
        let mut edges: BTreeMap<(u32, u32), u32> = BTreeMap::new();
        let mut used = BTreeSet::new();
        let mut faces = 0i64;
        for t in m.indices.chunks_exact(3) {
            let w = [ids[t[0] as usize], ids[t[1] as usize], ids[t[2] as usize]];
            if w[0] == w[1] || w[1] == w[2] || w[0] == w[2] {
                continue; // colapsou na soldagem (polo duplicado)
            }
            faces += 1;
            for k in 0..3 {
                let (a, b) = (w[k], w[(k + 1) % 3]);
                used.insert(a);
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        let bad = edges.values().filter(|&&c| c != 2).count();
        (used.len() as i64 - edges.len() as i64 + faces, bad)
    }

    /// Multiconjunto de triângulos como tuplas de bytes de vértice (enrolamento preservado
    /// via rotação canônica).
    fn triangle_multiset(m: &Mesh) -> BTreeMap<Vec<u8>, usize> {
        let mut out: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
        for t in m.indices.chunks_exact(3) {
            let vb: Vec<Vec<u8>> = t
                .iter()
                .map(|&i| bytemuck::bytes_of(&m.vertices[i as usize]).to_vec())
                .collect();
            // rotação canônica: começa pelo menor
            let start = (0..3).min_by_key(|&k| vb[k].clone()).unwrap_or(0);
            let mut key = Vec::new();
            for k in 0..3 {
                key.extend_from_slice(&vb[(start + k) % 3]);
            }
            *out.entry(key).or_default() += 1;
        }
        out
    }

    #[test]
    fn sphere_25_percent_keeps_shape_closure_and_attributes() {
        let src = sphere_20k();
        let (src_lo, src_hi) = bounds(&src);
        let (src_chi, src_bad) = euler_by_edges(&src);
        assert_eq!(src_chi, 2, "esfera de entrada deve ter χ = 2");
        assert_eq!(src_bad, 0);

        let tolerance = 0.02;
        let r =
            simplify_mesh(&src, 0.25, tolerance, &SimplifyOptions::default()).expect("simplifica");
        assert!(
            r.error <= tolerance,
            "erro {} > tolerância {}",
            r.error,
            tolerance
        );
        assert!(r.error_absolute >= 0.0 && r.error_absolute <= tolerance * 2.01 + 1e-6);
        assert!(
            r.reached_target,
            "esfera lisa deve alcançar 25 % com 2 % de erro"
        );
        assert!(
            (0.20..=0.30).contains(&r.ratio),
            "razão alcançada fora de 25 % ± 5: {}",
            r.ratio
        );
        assert_eq!(r.result_triangles, r.mesh.indices.len() / 3);

        // bounding box mantida (±2 % do raio)
        let (lo, hi) = bounds(&r.mesh);
        for k in 0..3 {
            assert!(
                (lo[k] - src_lo[k]).abs() < 0.02,
                "bbox min eixo {k}: {} vs {}",
                lo[k],
                src_lo[k]
            );
            assert!(
                (hi[k] - src_hi[k]).abs() < 0.02,
                "bbox max eixo {k}: {} vs {}",
                hi[k],
                src_hi[k]
            );
        }

        // fechamento: Euler = 2 e toda aresta com exatamente 2 triângulos
        let (chi, bad) = euler_by_edges(&r.mesh);
        assert_eq!(bad, 0, "arestas abertas/não-manifold após simplificar");
        assert_eq!(chi, 2, "característica de Euler mudou");

        // nenhum vértice sintetizado: cada um é bit-a-bit um vértice da entrada
        let originals: BTreeSet<Vec<u8>> = src
            .vertices
            .iter()
            .map(|v| bytemuck::bytes_of(v).to_vec())
            .collect();
        for v in &r.mesh.vertices {
            assert!(
                originals.contains(bytemuck::bytes_of(v)),
                "vértice sintetizado"
            );
        }

        // normais continuam radiais e UVs continuam em [0,1]
        for v in &r.mesh.vertices {
            let p = glam::Vec3::from_array(v.position);
            let n = glam::Vec3::from_array(v.normal);
            assert!(
                p.normalize().dot(n) > 0.99,
                "normal fugiu da radial: {}",
                p.normalize().dot(n)
            );
            assert!((0.0..=1.0).contains(&v.uv[0]) && (0.0..=1.0).contains(&v.uv[1]));
        }

        // costura de UV intacta: nenhum triângulo atravessa a volta inteira de U
        for t in r.mesh.indices.chunks_exact(3) {
            let us: Vec<f32> = t
                .iter()
                .map(|&i| r.mesh.vertices[i as usize].uv[0])
                .collect();
            let span = us.iter().cloned().fold(f32::MIN, f32::max)
                - us.iter().cloned().fold(f32::MAX, f32::min);
            assert!(span < 0.5, "triângulo cruza a costura de UV (span {span})");
        }
    }

    #[test]
    fn simplify_is_deterministic() {
        let src = sphere_20k();
        let a = simplify_mesh(&src, 0.25, 0.02, &SimplifyOptions::default()).expect("a");
        let b = simplify_mesh(&src, 0.25, 0.02, &SimplifyOptions::default()).expect("b");
        assert_eq!(a, b);
        let la = generate_lods(&src, &[0.5, 0.25, 0.1]).expect("lods a");
        let lb = generate_lods(&src, &[0.5, 0.25, 0.1]).expect("lods b");
        assert_eq!(la, lb);
    }

    #[test]
    fn tight_error_stops_before_ratio() {
        let src = sphere_20k();
        let r = simplify_mesh(&src, 0.01, 1e-5, &SimplifyOptions::default()).expect("simplifica");
        assert!(r.error <= 1e-5);
        assert!(!r.reached_target);
        assert!(
            r.ratio > 0.5,
            "com erro quase zero a malha mal reduz (ratio {})",
            r.ratio
        );
    }

    #[test]
    fn ratio_one_is_identity_up_to_compaction() {
        let src = sphere_20k();
        let r = simplify_mesh(&src, 1.0, 0.1, &SimplifyOptions::default()).expect("simplifica");
        assert_eq!(r.ratio, 1.0);
        assert_eq!(r.error, 0.0);
        assert_eq!(triangle_multiset(&r.mesh), triangle_multiset(&src));
    }

    #[test]
    fn locked_vertices_survive() {
        let src = sphere_20k();
        let mut locked = vec![false; src.vertices.len()];
        for (i, v) in src.vertices.iter().enumerate() {
            if v.position[1] > 0.5 {
                locked[i] = true; // calota superior intacta
            }
        }
        let opts = SimplifyOptions {
            locked_vertices: locked.clone(),
            ..SimplifyOptions::default()
        };
        let r = simplify_mesh(&src, 0.25, 0.05, &opts).expect("simplifica");
        let want: BTreeSet<Vec<u8>> = src
            .vertices
            .iter()
            .zip(&locked)
            .filter(|(_, l)| **l)
            .map(|(v, _)| bytemuck::bytes_of(v).to_vec())
            .collect();
        let got: BTreeSet<Vec<u8>> = r
            .mesh
            .vertices
            .iter()
            .map(|v| bytemuck::bytes_of(v).to_vec())
            .collect();
        assert!(want.is_subset(&got), "vértices travados foram removidos");
    }

    #[test]
    fn lods_are_monotonic() {
        let src = sphere_20k();
        let lods = generate_lods(&src, &[0.1, 1.0, 0.5, 0.25, 0.5]).expect("lods");
        assert_eq!(lods.len(), 4, "razões repetidas são fundidas");
        assert_eq!(lods[0].requested_ratio, 1.0);
        assert!((lods[0].ratio - 1.0).abs() < 1e-6);
        for w in lods.windows(2) {
            assert!(w[1].triangle_count < w[0].triangle_count);
            assert!(w[1].error >= w[0].error);
        }
        for l in &lods {
            let (chi, bad) = euler_by_edges(&l.mesh);
            assert_eq!((chi, bad), (2, 0), "LOD {} abriu a malha", l.level);
        }
        assert!(lods[3].ratio < 0.15);
        assert!(generate_lods(&src, &[]).is_err());
        assert!(generate_lods(&src, &[0.0]).is_err());
        assert!(generate_lods(&src, &[1.5]).is_err());
        assert!(generate_lods(&src, &[f32::NAN]).is_err());
    }

    #[test]
    fn optimize_preserves_triangles_and_improves_nothing_semantic() {
        let src = sphere_20k();
        // injeta um vértice duplicado idêntico e um não usado
        let mut dup = src.clone();
        let extra = dup.vertices[10];
        dup.vertices.push(extra);
        let new_idx = (dup.vertices.len() - 1) as u32;
        for i in dup.indices.iter_mut() {
            if *i == 10 {
                *i = new_idx;
                break;
            }
        }
        dup.vertices.push(extra); // órfão
        let opt = optimize_mesh(&dup).expect("otimiza");
        assert!(
            opt.vertices.len() <= src.vertices.len(),
            "dedupe + descarte de órfãos"
        );
        assert_eq!(opt.indices.len(), dup.indices.len());
        assert_eq!(triangle_multiset(&opt), triangle_multiset(&src));
        assert_eq!(euler_by_edges(&opt), (2, 0));
        // idempotente em conteúdo
        let again = optimize_mesh(&opt).expect("otimiza de novo");
        assert_eq!(triangle_multiset(&again), triangle_multiset(&src));
    }

    #[test]
    fn optimize_remaps_custom_attributes() {
        let src = sphere_20k();
        let n = src.vertices.len();
        let data: Vec<u8> = (0..n).flat_map(|i| (i as f32).to_le_bytes()).collect();
        let mesh = Mesh::new(src.name.clone(), src.vertices.clone(), src.indices.clone())
            .with_custom_attribute(
                "_ANIGO_AO",
                CustomAttribute {
                    name: "_ANIGO_AO".into(),
                    component_type: 5126,
                    attribute_type: "SCALAR".into(),
                    count: n,
                    data,
                },
            );
        let opt = optimize_mesh(&mesh).expect("otimiza");
        let attr = &opt.custom_attributes["_ANIGO_AO"];
        assert_eq!(attr.count, opt.vertices.len());
        assert_eq!(attr.data.len(), opt.vertices.len() * 4);
        // cada vértice conserva o valor do seu original (identificado pela posição)
        let by_pos: HashMap<[u32; 3], usize> = src
            .vertices
            .iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    [
                        v.position[0].to_bits(),
                        v.position[1].to_bits(),
                        v.position[2].to_bits(),
                    ],
                    i,
                )
            })
            .collect();
        for (i, v) in opt.vertices.iter().enumerate() {
            let key = [
                v.position[0].to_bits(),
                v.position[1].to_bits(),
                v.position[2].to_bits(),
            ];
            let orig = by_pos[&key];
            let got = f32::from_le_bytes([
                attr.data[i * 4],
                attr.data[i * 4 + 1],
                attr.data[i * 4 + 2],
                attr.data[i * 4 + 3],
            ]);
            // vértices duplicados na mesma posição (costura) compartilham posição mas
            // não valor: aceita qualquer vértice original naquela posição
            let ok = src
                .vertices
                .iter()
                .enumerate()
                .filter(|(_, sv)| {
                    sv.position == v.position && sv.uv == v.uv && sv.normal == v.normal
                })
                .any(|(si, _)| si as f32 == got);
            assert!(
                ok,
                "atributo customizado desalinhado no vértice {i} (orig~{orig})"
            );
        }
    }

    #[test]
    fn empty_and_degenerate_inputs_return_err_not_panic() {
        let opts = SimplifyOptions::default();
        let empty = Mesh::new("vazia", vec![], vec![]);
        assert!(matches!(
            simplify_mesh(&empty, 0.5, 0.01, &opts),
            Err(NativeError::EmptyInput(_))
        ));
        assert!(matches!(
            optimize_mesh(&empty),
            Err(NativeError::EmptyInput(_))
        ));
        assert!(generate_lods(&empty, &[0.5]).is_err());

        let v = |x: f32, y: f32, z: f32| Vertex::new([x, y, z], [0.0, 0.0, 1.0], [0.0, 0.0]);
        // vértices sem índices
        let no_idx = Mesh::new("sem índices", vec![v(0.0, 0.0, 0.0)], vec![]);
        assert!(simplify_mesh(&no_idx, 0.5, 0.01, &opts).is_err());
        // índices não múltiplos de 3
        let odd = Mesh::new(
            "ímpar",
            vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
            vec![0, 1],
        );
        assert!(matches!(
            simplify_mesh(&odd, 0.5, 0.01, &opts),
            Err(NativeError::InvalidInput(_))
        ));
        // índice fora de faixa
        let oob = Mesh::new(
            "oob",
            vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
            vec![0, 1, 7],
        );
        assert!(matches!(
            simplify_mesh(&oob, 0.5, 0.01, &opts),
            Err(NativeError::InvalidInput(_))
        ));
        assert!(optimize_mesh(&oob).is_err());
        // triângulo de índices repetidos e triângulo de área zero
        let dup_idx = Mesh::new(
            "dup",
            vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0)],
            vec![0, 0, 1],
        );
        assert!(matches!(
            simplify_mesh(&dup_idx, 0.5, 0.01, &opts),
            Err(NativeError::Degenerate(_))
        ));
        let collinear = Mesh::new(
            "colinear",
            vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(2.0, 0.0, 0.0)],
            vec![0, 1, 2],
        );
        assert!(matches!(
            simplify_mesh(&collinear, 0.5, 0.01, &opts),
            Err(NativeError::Degenerate(_))
        ));
        // posição NaN
        let nan = Mesh::new(
            "nan",
            vec![v(0.0, 0.0, 0.0), v(f32::NAN, 0.0, 0.0), v(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        assert!(matches!(
            simplify_mesh(&nan, 0.5, 0.01, &opts),
            Err(NativeError::InvalidInput(_))
        ));
        // parâmetros ruins
        let tri = Mesh::new(
            "tri",
            vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(0.0, 1.0, 0.0)],
            vec![0, 1, 2],
        );
        for ratio in [0.0, -1.0, 1.5, f32::NAN, f32::INFINITY] {
            assert!(
                simplify_mesh(&tri, ratio, 0.01, &opts).is_err(),
                "ratio {ratio}"
            );
        }
        for err in [-0.1, 2.0, f32::NAN] {
            assert!(simplify_mesh(&tri, 0.5, err, &opts).is_err(), "erro {err}");
        }
        let bad_lock = SimplifyOptions {
            locked_vertices: vec![true],
            ..SimplifyOptions::default()
        };
        assert!(simplify_mesh(&tri, 0.5, 0.01, &bad_lock).is_err());
        let bad_w = SimplifyOptions {
            attribute_weights: AttributeWeights {
                normal: -1.0,
                ..AttributeWeights::default()
            },
            ..SimplifyOptions::default()
        };
        assert!(simplify_mesh(&tri, 0.5, 0.01, &bad_w).is_err());
        // atributo customizado que não é por-vértice
        let bad_attr = tri.clone().with_custom_attribute(
            "x",
            CustomAttribute {
                name: "x".into(),
                component_type: 5126,
                attribute_type: "SCALAR".into(),
                count: 2,
                data: vec![0; 8],
            },
        );
        assert!(simplify_mesh(&bad_attr, 0.5, 0.01, &opts).is_err());
    }

    #[test]
    fn single_triangle_is_returned_intact() {
        let v = |x: f32, y: f32| Vertex::new([x, y, 0.0], [0.0, 0.0, 1.0], [x, y]);
        let tri = Mesh::new(
            "tri",
            vec![v(0.0, 0.0), v(1.0, 0.0), v(0.0, 1.0)],
            vec![0, 1, 2],
        );
        let r = simplify_mesh(&tri, 0.1, 0.5, &SimplifyOptions::default()).expect("simplifica");
        assert_eq!(r.result_triangles, 1);
        assert_eq!(triangle_multiset(&r.mesh), triangle_multiset(&tri));
    }
}
