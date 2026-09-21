//! ANIGO — contrato do renderer v1 (P0 "Consolidar o renderer").
//!
//! Este módulo é o lado Rust da **única** definição de shaders, buffers,
//! uniforms, câmera, passes e toon ramp do projeto:
//! `contracts/fixtures/render_contract_v1.json`. O viewport (TypeScript) lê o
//! mesmo documento através de `src/contracts/render_contract.v1.ts`, então uma
//! divergência entre headless e viewport vira erro de teste em vez de diferença
//! visual silenciosa.
//!
//! Os shaders vivem em `crates/anigo-renderer/shaders/` e são carregados aqui
//! com `include_str!` — os tipos `wgpu` (formato, cull, blend, depth, MSAA)
//! saem do contrato, nunca de literais espalhados pelo renderer.
//!
//! Testes (`cargo test -p anigo-renderer render_contract`) conferem:
//!   * o FNV-1a-64 de cada shader contra o contrato (e contra os bytes que o
//!     viewport empacota com `?raw`);
//!   * tamanho/offset de cada uniform do contrato contra `uniforms.rs`;
//!   * os bytes do toon ramp contra a impressão digital congelada.

use std::sync::OnceLock;

use serde_json::Value;

/// O documento congelado do contrato (fonte única, também lida pelo viewport).
pub const CONTRACT_JSON: &str = include_str!("../../../contracts/fixtures/render_contract_v1.json");

/// Versão do contrato que este módulo sabe interpretar.
pub const CONTRACT_VERSION: u64 = 1;

// ---------------------------------------------------------------------------
// Shaders (uma cópia só: os bytes que o Rust compila são os que o viewport usa)
// ---------------------------------------------------------------------------

pub const CEL_SHADING_WGSL: &str = include_str!("../shaders/cel_shading.wgsl");
pub const INVERTED_HULL_WGSL: &str = include_str!("../shaders/inverted_hull.wgsl");
pub const MORPH_SPARSE_COMPUTE_WGSL: &str = include_str!("../shaders/morph_sparse_compute.wgsl");

/// (nome no contrato, bytes) dos shaders de produção usados pelo headless.
pub const PRODUCTION_SHADERS: [(&str, &str); 3] = [
    ("cel_shading", CEL_SHADING_WGSL),
    ("inverted_hull", INVERTED_HULL_WGSL),
    ("morph_sparse_compute", MORPH_SPARSE_COMPUTE_WGSL),
];

/// FNV-1a de 64 bits — mesma função do gerador de fixtures e do viewport.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

// ---------------------------------------------------------------------------
// Acesso cru ao documento
// ---------------------------------------------------------------------------

/// Documento mínimo usado quando o contrato não carrega.
///
/// P1-01: o caminho crítico do renderer **não** aborta o processo nem falha em
/// silêncio — um contrato ilegível vira diagnóstico (consultável em
/// [`diagnostics`]) e o renderer cai nos defaults declarados aqui.
const FALLBACK_CONTRACT_JSON: &str = r#"{
  "version": 1,
  "shaders": [],
  "passes": [],
  "bind_groups": [],
  "uniforms": {},
  "vertex_layout": { "stride": 72, "attributes": [] },
  "targets": {},
  "toon_ramp": {}
}"#;

/// Diagnósticos acumulados (problemas do contrato, deduplicados).
fn feedback() -> &'static std::sync::Mutex<Vec<String>> {
    static FEEDBACK: OnceLock<std::sync::Mutex<Vec<String>>> = OnceLock::new();
    FEEDBACK.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Registra um problema do contrato sem `panic!`.
fn diag(message: impl Into<String>) {
    let message = message.into();
    let mut log = match feedback().lock() {
        Ok(log) => log,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !log.iter().any(|existing| existing == &message) {
        tracing::error!(target: "anigo::render_contract", "{message}");
        log.push(message);
    }
}

/// Registra um problema do renderer/contrato sem `panic!`.
///
/// Usado pelo headless quando encontra um passe sem pipeline correspondente:
/// um contrato em drift precisa virar diagnóstico observável, não um
/// `debug_assert!` que só aparece em build de debug.
pub fn report(message: impl Into<String>) {
    diag(message);
}

/// Problemas encontrados ao ler o contrato (vazio quando está tudo certo).
pub fn diagnostics() -> Vec<String> {
    match feedback().lock() {
        Ok(log) => log.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// `true` quando o contrato carregou e nenhum problema foi registrado.
pub fn contract_is_valid() -> bool {
    diagnostics().is_empty()
}

/// Valor nulo estático para accessors que não encontram o que procuram.
fn empty() -> &'static Value {
    static EMPTY: Value = Value::Null;
    &EMPTY
}

/// Contrato parseado uma única vez por processo (nunca aborta: ver `diag`).
pub fn contract() -> &'static Value {
    static PARSED: OnceLock<Value> = OnceLock::new();
    PARSED.get_or_init(|| {
        match serde_json::from_str::<Value>(CONTRACT_JSON) {
            Ok(parsed) => {
                let version = parsed["version"].as_u64().unwrap_or(0);
                if version != CONTRACT_VERSION {
                    diag(format!(
                        "render contract v{} não é suportado por este build (esperado v{})",
                        version, CONTRACT_VERSION
                    ));
                    return fallback_contract();
                }
                parsed
            }
            Err(error) => {
                diag(format!("render contract v{CONTRACT_VERSION} é JSON inválido: {error}"));
                fallback_contract()
            }
        }
    })
}

fn fallback_contract() -> Value {
    serde_json::from_str(FALLBACK_CONTRACT_JSON)
        .unwrap_or_else(|_| Value::Object(serde_json::Map::new()))
}

fn shader_value(name: &str) -> &'static Value {
    let shaders = match contract()["shaders"].as_array() {
        Some(shaders) => shaders,
        None => {
            diag("render contract sem `shaders`");
            return empty();
        }
    };
    match shaders
        .iter()
        .find(|shader| shader["name"].as_str() == Some(name))
    {
        Some(shader) => shader,
        None => {
            diag(format!("shader '{name}' não está no render contract"));
            empty()
        }
    }
}

/// Caminho do shader relativo à raiz do repositório.
pub fn shader_path(name: &str) -> &'static str {
    match shader_value(name)["path"].as_str() {
        Some(path) => path,
        None => {
            diag(format!("shader '{name}' sem `path`"));
            ""
        }
    }
}

/// FNV-1a-64 congelado do shader (hex de 16 dígitos, como no contrato).
pub fn shader_hash(name: &str) -> &'static str {
    match shader_value(name)["fnv1a64"].as_str() {
        Some(hash) => hash,
        None => {
            diag(format!("shader '{name}' sem `fnv1a64`"));
            ""
        }
    }
}

/// Nome do entry point de um estágio do shader (`vertex`/`fragment`/`compute`).
pub fn shader_entry_point(name: &str, stage: &str) -> &'static str {
    match shader_value(name)["entry_points"][stage].as_str() {
        Some(entry) => entry,
        None => {
            diag(format!("shader '{name}' não declara entry point '{stage}'"));
            ""
        }
    }
}

/// Todos os shaders de um papel (`production`, `fallback_webgl2`, `library`).
pub fn shaders_with_role(role: &str) -> Vec<(&'static str, &'static str)> {
    let shaders = match contract()["shaders"].as_array() {
        Some(shaders) => shaders,
        None => {
            diag("render contract sem `shaders`");
            return Vec::new();
        }
    };
    shaders
        .iter()
        .filter(|shader| shader["role"].as_str() == Some(role))
        .map(|shader| {
            (
                shader["name"].as_str().unwrap_or(""),
                shader["path"].as_str().unwrap_or(""),
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Uniforms / vertex layout
// ---------------------------------------------------------------------------

fn uniform_value(name: &str) -> &'static Value {
    let block = &contract()["uniforms"][name];
    if block.is_null() {
        diag(format!("uniform '{name}' não está no render contract"));
    }
    block
}

/// Array estático vazio (default de accessors quando o contrato está quebrado).
fn empty_array() -> &'static Vec<Value> {
    static EMPTY: OnceLock<Vec<Value>> = OnceLock::new();
    EMPTY.get_or_init(Vec::new)
}

pub fn uniform_size(name: &str) -> u32 {
    uniform_value(name)["size"].as_u64().unwrap_or(0) as u32
}

pub fn uniform_address_space(name: &str) -> &'static str {
    uniform_value(name)["address_space"].as_str().unwrap_or("uniform")
}

pub fn uniform_offset(name: &str, field: &str) -> u32 {
    let fields = match uniform_value(name)["fields"].as_array() {
        Some(fields) => fields,
        None => {
            diag(format!("uniform '{name}' sem `fields`"));
            empty_array()
        }
    };
    match fields
        .iter()
        .find(|entry| entry["name"].as_str() == Some(field))
    {
        Some(entry) => entry["offset"].as_u64().unwrap_or(0) as u32,
        None => {
            diag(format!("campo '{name}.{field}' não está no render contract"));
            0
        }
    }
}

/// Bytes do buffer de vértice (72 no layout NPR canônico).
pub fn vertex_stride() -> u64 {
    contract()["vertex_layout"]["stride"].as_u64().unwrap_or(0)
}

/// `(shader_location, offset, formato)` de cada atributo, na ordem declarada.
pub fn vertex_attributes() -> Vec<(u32, u64, wgpu::VertexFormat)> {
    let attributes = match contract()["vertex_layout"]["attributes"].as_array() {
        Some(attributes) => attributes,
        None => {
            diag("render contract sem `vertex_layout.attributes`");
            empty_array()
        }
    };
    attributes
        .iter()
        .map(|attribute| {
            (
                attribute["shader_location"].as_u64().unwrap_or(0) as u32,
                attribute["offset"].as_u64().unwrap_or(0),
                vertex_format(attribute["format"].as_str().unwrap_or("")),
            )
        })
        .collect()
}

/// Formato de atributo do contrato → enum do wgpu.
pub fn vertex_format(format: &str) -> wgpu::VertexFormat {
    match format {
        "float32x2" => wgpu::VertexFormat::Float32x2,
        "float32x3" => wgpu::VertexFormat::Float32x3,
        "float32x4" => wgpu::VertexFormat::Float32x4,
        "uint16x4" => wgpu::VertexFormat::Uint16x4,
        other => {
            diag(format!("formato de vértice '{other}' não é suportado"));
            wgpu::VertexFormat::Float32x3
        }
    }
}

// ---------------------------------------------------------------------------
// Passes / pipeline state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum PassKind {
    Render,
    Compute,
}

/// Estado de pipeline que headless e viewport precisam reproduzir igual.
#[derive(Debug, Clone, PartialEq)]
pub struct PassSpec {
    pub order: i64,
    pub name: &'static str,
    pub kind: PassKind,
    pub shader: &'static str,
    pub vertex_entry: Option<&'static str>,
    pub fragment_entry: Option<&'static str>,
    pub compute_entry: Option<&'static str>,
    pub bind_group: &'static str,
    pub cull_mode: Option<wgpu::Face>,
    pub front_face: wgpu::FrontFace,
    pub depth_write: bool,
    pub depth_compare: wgpu::CompareFunction,
    pub depth_bias: wgpu::DepthBiasState,
    pub blend: Option<wgpu::BlendState>,
    pub workgroup_size: Option<u32>,
    pub only_when: Option<&'static str>,
}

fn pass_value(name: &str) -> &'static Value {
    let passes = match contract()["passes"].as_array() {
        Some(passes) => passes,
        None => {
            diag("render contract sem `passes`");
            return empty();
        }
    };
    match passes.iter().find(|pass| pass["name"].as_str() == Some(name)) {
        Some(pass) => pass,
        None => {
            diag(format!("passe '{name}' não está no render contract"));
            empty()
        }
    }
}

/// Todos os passes declarados, ordenados por `order` (compute vem antes).
pub fn passes() -> Vec<PassSpec> {
    let declared = match contract()["passes"].as_array() {
        Some(declared) => declared,
        None => {
            diag("render contract sem `passes`");
            empty_array()
        }
    };
    let mut specs: Vec<PassSpec> = declared
        .iter()
        .map(pass_spec)
        .collect();
    specs.sort_by_key(|spec| spec.order);
    specs
}

/// O contrato é um JSON `'static` (uma única `OnceLock`), então as especificações
/// podem guardar `&'static str` e o renderer não guarda cópia própria dos nomes.
fn pass_spec(pass: &'static Value) -> PassSpec {
    let kind = match pass["kind"].as_str() {
        Some("compute") => PassKind::Compute,
        _ => PassKind::Render,
    };
    PassSpec {
        order: pass["order"].as_i64().unwrap_or(0),
        name: pass["name"].as_str().unwrap_or(""),
        kind,
        shader: pass["shader"].as_str().unwrap_or(""),
        vertex_entry: pass["vertex_entry"].as_str(),
        fragment_entry: pass["fragment_entry"].as_str(),
        compute_entry: pass["compute_entry"].as_str(),
        bind_group: pass["bind_group"].as_str().unwrap_or(""),
        cull_mode: pass["cull_mode"].as_str().map(cull_mode),
        front_face: wgpu::FrontFace::Ccw,
        depth_write: pass["depth_write"].as_bool().unwrap_or(false),
        depth_compare: compare_function(pass["depth_compare"].as_str().unwrap_or("less_equal")),
        depth_bias: depth_bias(&pass["depth_bias"]),
        blend: pass["blend"].as_str().and_then(blend_state),
        workgroup_size: pass["workgroup_size"].as_u64().map(|size| size as u32),
        only_when: pass["only_when"].as_str(),
    }
}

/// Passes de render na ordem declarada (headless e viewport desenham nessa ordem).
/// Ordem de execução dos passes de render (issue #14).
///
/// A **fonte é o `render_graph`**: os nós habilitados que têm passe no contrato,
/// na ordem topológica do DAG. Não existe uma segunda lista de ordem — headless
/// e viewport derivam daqui, então não há como divergirem. Sem o bloco no
/// contrato (contrato antigo), cai na ordem declarada da biblioteca de passes.
pub fn render_pass_order() -> Vec<&'static str> {
    let spec = render_graph_spec();
    let Some(nodes) = spec["nodes"].as_array() else {
        return passes()
            .into_iter()
            .filter(|pass| pass.kind == PassKind::Render)
            .map(|pass| pass.name)
            .collect();
    };
    let graph = match crate::render_graph::graph_from_contract() {
        Ok(graph) => graph,
        Err(error) => {
            diag(format!("render_graph inválido: {error}"));
            return Vec::new();
        }
    };
    let order = match graph.topological_order() {
        Ok(order) => order,
        Err(error) => {
            diag(format!("render_graph sem ordem topológica: {error}"));
            return Vec::new();
        }
    };
    order
        .iter()
        .filter_map(|name| {
            let node = nodes
                .iter()
                .find(|candidate| candidate["name"].as_str() == Some(name.as_str()))?;
            if node["kind"].as_str() != Some("render") || !node["enabled"].as_bool().unwrap_or(false)
            {
                return None;
            }
            node["contract_pass"].as_str()
        })
        .collect()
}

/// Issue #14: nó do render graph pelo nome (bloco `render_graph.nodes`).
///
/// A referência vem do JSON estático do contrato, sem cópia: um consultor por
/// frame não pode alocar (nem vazar) memória.
pub fn render_graph_node(name: &str) -> Option<&'static Value> {
    render_graph_spec()["nodes"]
        .as_array()?
        .iter()
        .find(|node| node["name"].as_str() == Some(name))
}

/// Issue #14: bloco `render_graph` do contrato (a autoridade sobre ordem,
/// ativação, recursos transitórios e aliasing).
pub fn render_graph_spec() -> &'static Value {
    &contract()["render_graph"]
}

/// Especificação de um passe de render (sem `panic!`: um passe trocado no
/// contrato vira diagnóstico, não derruba o processo no meio de um frame).
pub fn render_pass(name: &str) -> PassSpec {
    let parsed = pass_spec(pass_value(name));
    if parsed.kind != PassKind::Render {
        diag(format!("passe '{name}' deveria ser de render"));
    }
    parsed
}

/// Especificação de um passe de compute (idem: diagnóstico em vez de `panic!`).
pub fn compute_pass(name: &str) -> PassSpec {
    let parsed = pass_spec(pass_value(name));
    if parsed.kind != PassKind::Compute {
        diag(format!("passe '{name}' deveria ser compute"));
    }
    parsed
}

/// Grupo de bind groups declarados por passe (bindings do WGSL).
pub fn bind_group_entries(name: &str) -> Vec<(u32, &'static str)> {
    let groups = match contract()["bind_groups"].as_array() {
        Some(groups) => groups,
        None => {
            diag("render contract sem `bind_groups`");
            return Vec::new();
        }
    };
    let group = match groups.iter().find(|group| group["name"].as_str() == Some(name)) {
        Some(group) => group,
        None => {
            diag(format!("bind group '{name}' não está no render contract"));
            return Vec::new();
        }
    };
    let entries = match group["entries"].as_array() {
        Some(entries) => entries,
        None => {
            diag(format!("bind group '{name}' sem `entries`"));
            return Vec::new();
        }
    };
    entries
        .iter()
        .map(|entry| (entry["binding"].as_u64().unwrap_or(0) as u32, entry["kind"].as_str().unwrap_or("")))
        .collect()
}

pub fn cull_mode(mode: &str) -> wgpu::Face {
    match mode {
        "front" => wgpu::Face::Front,
        "back" => wgpu::Face::Back,
        other => {
            diag(format!("cull_mode '{other}' não é suportado"));
            wgpu::Face::Back
        }
    }
}

pub fn compare_function(value: &str) -> wgpu::CompareFunction {
    match value {
        "less" => wgpu::CompareFunction::Less,
        "less-equal" | "less_equal" => wgpu::CompareFunction::LessEqual,
        "greater" => wgpu::CompareFunction::Greater,
        "greater-equal" | "greater_equal" => wgpu::CompareFunction::GreaterEqual,
        "always" => wgpu::CompareFunction::Always,
        other => {
            diag(format!("depth_compare '{other}' não é suportado"));
            wgpu::CompareFunction::LessEqual
        }
    }
}

pub fn blend_state(value: &str) -> Option<wgpu::BlendState> {
    match value {
        "none" => None,
        "src_alpha_one_minus_src_alpha" => Some(wgpu::BlendState::ALPHA_BLENDING),
        other => {
            diag(format!("blend '{other}' não é suportado"));
            None
        }
    }
}

pub fn depth_bias(value: &Value) -> wgpu::DepthBiasState {
    wgpu::DepthBiasState {
        constant: value["constant"].as_i64().unwrap_or(0) as i32,
        slope_scale: value["slope_scale"].as_f64().unwrap_or(0.0) as f32,
        clamp: value["clamp"].as_f64().unwrap_or(0.0) as f32,
    }
}

// ---------------------------------------------------------------------------
// Catálogo de diagnósticos (P1-02: um só vocabulário para Rust ⇄ TypeScript)
// ---------------------------------------------------------------------------

/// `(code, severity)` de cada diagnóstico declarado no contrato.
pub fn diagnostic_codes() -> Vec<(&'static str, &'static str)> {
    let codes = match contract()["diagnostics"]["codes"].as_array() {
        Some(codes) => codes,
        None => {
            diag("render contract sem `diagnostics.codes`");
            return Vec::new();
        }
    };
    codes
        .iter()
        .map(|entry| {
            (
                entry["code"].as_str().unwrap_or(""),
                entry["severity"].as_str().unwrap_or(""),
            )
        })
        .collect()
}

/// Severidade de um código de diagnóstico (`None` quando não existe no contrato).
pub fn diagnostic_severity(code: &str) -> Option<crate::diagnostics::Severity> {
    let codes = contract()["diagnostics"]["codes"].as_array()?;
    codes
        .iter()
        .find(|entry| entry["code"].as_str() == Some(code))
        .and_then(|entry| entry["severity"].as_str())
        .and_then(crate::diagnostics::Severity::from_str)
}

// ---------------------------------------------------------------------------
// Alvos, MSAA e toon ramp
// ---------------------------------------------------------------------------

/// Formato do alvo offscreen do headless (`rgba8unorm`, sem view sRGB).
pub fn offscreen_color_format() -> wgpu::TextureFormat {
    texture_format(
        contract()["targets"]["offscreen_color_format"]
            .as_str()
            .unwrap_or("rgba8unorm"),
    )
}

pub fn depth_format() -> wgpu::TextureFormat {
    texture_format(
        contract()["targets"]["depth_format"]
            .as_str()
            .unwrap_or("depth24plus"),
    )
}

/// Amostras de MSAA que o alvo (e os pipelines) precisam usar.
pub fn msaa_sample_count() -> u32 {
    contract()["targets"]["msaa_samples"].as_u64().unwrap_or(1) as u32
}

/// `true` quando o alvo offscreen resolve a textura de MSAA antes da leitura.
pub fn resolve_to_swapchain() -> bool {
    contract()["targets"]["resolve_to_swapchain"].as_bool().unwrap_or(false)
}

/// Origem da cor de limpeza (`scene.background_color`).
pub fn clear_color_source() -> &'static str {
    contract()["targets"]["clear"]["source"]
        .as_str()
        .unwrap_or("scene.background_color")
}

pub fn texture_format(format: &str) -> wgpu::TextureFormat {
    match format {
        "rgba8unorm" => wgpu::TextureFormat::Rgba8Unorm,
        "rgba8unorm-srgb" => wgpu::TextureFormat::Rgba8UnormSrgb,
        "bgra8unorm" => wgpu::TextureFormat::Bgra8Unorm,
        "bgra8unorm-srgb" => wgpu::TextureFormat::Bgra8UnormSrgb,
        "depth24plus" => wgpu::TextureFormat::Depth24Plus,
        "depth32float" => wgpu::TextureFormat::Depth32Float,
        other => {
            diag(format!("formato de textura '{other}' não é suportado"));
            wgpu::TextureFormat::Rgba8Unorm
        }
    }
}

// ---------------------------------------------------------------------------
// Skinning (P1-04): paleta de ossos compartilhada por viewport e headless
// ---------------------------------------------------------------------------

/// Ossos da paleta declarados no contrato (24 no esqueleto canônico).
pub fn skinning_joint_count() -> u32 {
    skinning()["joint_count"].as_u64().unwrap_or(24) as u32
}

/// Nome da variável da paleta no WGSL (`bones`).
pub fn skinning_palette_uniform() -> &'static str {
    skinning()["palette_uniform"].as_str().unwrap_or("bones")
}

/// Tamanho da paleta em bytes (24 × mat4 = 1536).
pub fn skinning_palette_bytes() -> usize {
    skinning()["palette_bytes"].as_u64().unwrap_or(1536) as usize
}

/// Influências por vértice (o layout de vértice guarda quatro).
pub fn skinning_max_influences() -> u32 {
    skinning()["max_influences"].as_u64().unwrap_or(4) as u32
}

/// Marcadores do bloco de skinning que precisa ser idêntico entre os shaders.
pub fn skinning_block_markers() -> (&'static str, &'static str) {
    let markers = skinning()["block_markers"].as_array();
    let open = markers
        .and_then(|values| values.first())
        .and_then(|value| value.as_str())
        .unwrap_or("// ANIGO-SKINNING-BEGIN");
    let close = markers
        .and_then(|values| values.get(1))
        .and_then(|value| value.as_str())
        .unwrap_or("// ANIGO-SKINNING-END");
    (open, close)
}

/// Bindings (em ordem crescente) que um bind group declara no contrato.
pub fn bind_group_bindings(group: &str) -> Vec<u32> {
    let mut bindings: Vec<u32> = contract()["bind_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["name"].as_str() == Some(group))
        .flat_map(|entry| {
            entry["entries"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|value| value["binding"].as_u64())
                .map(|binding| binding as u32)
                .collect::<Vec<u32>>()
        })
        .collect();
    bindings.sort_unstable();
    bindings.dedup();
    bindings
}

/// Binding do uniform de skinning dentro de um bind group (`None` se ausente).
pub fn skinning_binding(group: &str) -> Option<u32> {
    let uniform = skinning_palette_uniform();
    contract()["bind_groups"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["name"].as_str() == Some(group))
        .and_then(|entry| entry["entries"].as_array())
        .into_iter()
        .flatten()
        .find(|value| value["name"].as_str() == Some(uniform))
        .and_then(|value| value["binding"].as_u64())
        .map(|binding| binding as u32)
}

fn skinning() -> &'static Value {
    &contract()["skinning"]
}

pub fn toon_ramp_spec() -> &'static Value {
    &contract()["toon_ramp"]
}

pub fn filter_mode(value: &str) -> wgpu::FilterMode {
    match value {
        "linear" => wgpu::FilterMode::Linear,
        "nearest" => wgpu::FilterMode::Nearest,
        other => {
            diag(format!("filtro '{other}' não é suportado"));
            wgpu::FilterMode::Linear
        }
    }
}

pub fn address_mode(value: &str) -> wgpu::AddressMode {
    match value {
        "clamp_to_edge" | "clamp-to-edge" => wgpu::AddressMode::ClampToEdge,
        "repeat" => wgpu::AddressMode::Repeat,
        "mirror_repeat" | "mirror-repeat" => wgpu::AddressMode::MirrorRepeat,
        other => {
            diag(format!("address mode '{other}' não é suportado"));
            wgpu::AddressMode::ClampToEdge
        }
    }
}

/// Gera a textura 256×4 do toon ramp a partir das linhas descritas no contrato.
///
/// A conta é feita em `f64` de propósito: é a mesma aritmética do gerador de
/// fixtures e do viewport (em `f32` o limiar de 0.7 renderizaria 178 em vez de
/// 179 e a impressão digital mudaria).
pub fn toon_ramp_bytes() -> Vec<u8> {
    let spec = toon_ramp_spec();
    let width = spec["width"].as_u64().unwrap_or(256) as usize;
    let height = spec["height"].as_u64().unwrap_or(4) as usize;
    let rows = spec["rows"].as_array().cloned().unwrap_or_default();
    if width == 0 || height == 0 {
        diag("toon_ramp sem width/height utilizáveis");
        return Vec::new();
    }
    let mut data = vec![0u8; width * height * 4];
    let empty_row = Value::Object(serde_json::Map::new());

    for y in 0..height {
        let row = rows.get(y).unwrap_or(&empty_row);
        let identity = row["kind"].as_str() == Some("identity");
        let steps = row["steps"].as_array().cloned().unwrap_or_default();
        for x in 0..width {
            let u = if width > 1 {
                x as f64 / (width as f64 - 1.0)
            } else {
                0.0
            };
            let mut factor = if identity {
                u
            } else {
                row["base"].as_f64().unwrap_or(0.0)
            };
            if !identity {
                for step in &steps {
                    if u >= step["threshold"].as_f64().unwrap_or(0.0) {
                        factor = step["value"].as_f64().unwrap_or(1.0);
                    }
                }
            }
            let value = (factor * 255.0).round().clamp(0.0, 255.0) as u8;
            let index = (y * width + x) * 4;
            data[index] = value;
            data[index + 1] = value;
            data[index + 2] = value;
            data[index + 3] = 255;
        }
    }
    data
}

/// Impressão digital dos bytes do toon ramp gerados aqui.
pub fn toon_ramp_fingerprint() -> u64 {
    fnv1a64(&toon_ramp_bytes())
}

/// Impressão digital congelada no contrato (hex).
pub fn expected_toon_ramp_fingerprint() -> &'static str {
    match contract()["toon_ramp"]["bytes_fnv1a64"].as_str() {
        Some(hash) => hash,
        None => {
            diag("toon_ramp sem `bytes_fnv1a64`");
            ""
        }
    }
}

// ---------------------------------------------------------------------------
// Câmera
// ---------------------------------------------------------------------------

/// `scene.nodes[*].world_matrix` — a fonte canônica do model matrix.
///
/// A matriz mundial já vem resolvida pelo núcleo (`W = W_pai × T_local`), o que
/// torna a hierarquia do snapshot a única autoridade sobre a pose do quadro
/// (issue #12). O contrato antigo dizia `scene.nodes[0].transform`, o que não
/// expressa nem o nó corrente nem a cadeia de pais.
pub fn model_matrix_source() -> &'static str {
    contract()["camera"]["model_from"]
        .as_str()
        .unwrap_or("scene.nodes[*].world_matrix")
}

/// Bloco de uniform que carrega a câmera (`camera`).
pub fn camera_uniform_block() -> &'static str {
    contract()["camera"]["uniform"].as_str().unwrap_or("camera")
}

/// Issue #13: modos de projeção declarados pelo contrato (`perspective` e
/// `orthographic`), com a matriz de cada um. Os dois escrevem a mesma
/// `view_proj`, então o shader não precisa saber qual está ativo.
pub fn projection_modes() -> [(&'static str, &'static str); 2] {
    let modes = &contract()["camera"]["projection_modes"];
    [
        (
            "perspective",
            modes["perspective"]["matrix"]
                .as_str()
                .unwrap_or("perspective_rh"),
        ),
        (
            "orthographic",
            modes["orthographic"]["matrix"]
                .as_str()
                .unwrap_or("orthographic_rh"),
        ),
    ]
}

/// Issue #13: descrição do frustum culling declarada pelo contrato.
///
/// O culling roda em espaço de mundo, com os 6 planos extraídos da `view_proj`;
/// a telemetria (`RenderMetrics.culled_draw_calls`) fecha o ciclo.
pub fn culling_report() -> CullingContract {
    let culling = &contract()["camera"]["culling"];
    CullingContract {
        volume: culling["volume"].as_str().unwrap_or("aabb+sphere").to_string(),
        planes_from: culling["planes_from"]
            .as_str()
            .unwrap_or("view_proj")
            .to_string(),
        plane_count: culling["plane_count"].as_u64().unwrap_or(6) as usize,
        test: culling["test"]
            .as_str()
            .unwrap_or("sphere_then_aabb")
            .to_string(),
        space: culling["space"].as_str().unwrap_or("world").to_string(),
        metrics: culling["metrics"]
            .as_str()
            .unwrap_or("RenderMetrics.culled_draw_calls")
            .to_string(),
    }
}

/// Issue #13: o culling declarado no contrato, já decodificado.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CullingContract {
    pub volume: String,
    pub planes_from: String,
    pub plane_count: usize,
    pub test: String,
    pub space: String,
    pub metrics: String,
}

/// Convenção de profundidade do clip space (`zero_to_one`).
pub fn clip_depth() -> &'static str {
    contract()["camera"]["clip_depth"]
        .as_str()
        .unwrap_or("zero_to_one")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CameraUniform, OutlineUniform};

    fn contract_hash_as_u64(hash: &str) -> u64 {
        match u64::from_str_radix(hash, 16) {
            Ok(value) => value,
            Err(_) => {
                diag(format!("hash do contrato não é hex: '{hash}'"));
                0
            }
        }
    }

    #[test]
    fn contract_parses_and_matches_version() {
        assert_eq!(contract()["version"].as_u64(), Some(CONTRACT_VERSION));
        assert_eq!(clip_depth(), "zero_to_one");
        assert_eq!(model_matrix_source(), "scene.nodes[*].world_matrix");

        // Issue #13: os dois modos de projeção e o culling declarados no contrato.
        let modes = projection_modes();
        assert_eq!(modes[0], ("perspective", "perspective_rh"));
        assert_eq!(modes[1], ("orthographic", "orthographic_rh"));
        let culling = culling_report();
        assert_eq!(culling.plane_count, 6);
        assert_eq!(culling.planes_from, "view_proj");
        assert_eq!(culling.space, "world");
        assert_eq!(culling.metrics, "RenderMetrics.culled_draw_calls");
        assert_eq!(culling.test, "sphere_then_aabb");
    }

    #[test]
    fn every_production_shader_matches_its_frozen_hash() {
        assert_eq!(shaders_with_role("production").len(), 3);
        for (name, source) in PRODUCTION_SHADERS {
            let normalized = source.replace("\r\n", "\n");
            let actual = fnv1a64(normalized.as_bytes());
            assert_eq!(
                actual,
                contract_hash_as_u64(shader_hash(name)),
                "shader '{}' divergiu do render contract ({} bytes de fonte)",
                name,
                normalized.len()
            );
            assert!(
                shader_path(name).starts_with("crates/anigo-renderer/shaders/"),
                "shader '{}' deve morar em crates/anigo-renderer/shaders/",
                name
            );
        }
    }

    #[test]
    fn uniform_blocks_match_the_frozen_layout() {
        use crate::uniforms::{CameraUniform, LightUniform, MaterialUniform, OutlineUniform};

        assert_eq!(uniform_size("camera") as usize, std::mem::size_of::<CameraUniform>());
        assert_eq!(uniform_size("light") as usize, std::mem::size_of::<LightUniform>());
        assert_eq!(uniform_size("material") as usize, std::mem::size_of::<MaterialUniform>());
        assert_eq!(uniform_size("outline") as usize, std::mem::size_of::<OutlineUniform>());
        assert_eq!(uniform_address_space("camera"), "uniform");

        // camera: view_proj @0, camera_pos @64, model @80, normal_mat @144
        assert_eq!(uniform_offset("camera", "view_proj"), 0);
        assert_eq!(uniform_offset("camera", "camera_pos"), 64);
        assert_eq!(uniform_offset("camera", "model"), 80);
        assert_eq!(uniform_offset("camera", "normal_mat"), 144);
        // outline: color @0, params @16, params2 @32
        assert_eq!(uniform_offset("outline", "color"), 0);
        assert_eq!(uniform_offset("outline", "params"), 16);
        assert_eq!(uniform_offset("outline", "params2"), 32);
    }

    #[test]
    fn uniform_field_bytes_land_on_the_contract_offsets() {
        let camera = CameraUniform {
            view_proj: [
                1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0,
            ],
            camera_pos: [17.0, 18.0, 19.0, 20.0],
            model: [21.0; 16],
            normal_mat: [22.0; 16],
        };
        let bytes = bytemuck::bytes_of(&camera);
        let read = |offset: usize| {
            f32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ])
        };
        assert_eq!(read(uniform_offset("camera", "view_proj") as usize), 1.0);
        assert_eq!(read(uniform_offset("camera", "view_proj") as usize + 60), 16.0);
        assert_eq!(read(uniform_offset("camera", "camera_pos") as usize), 17.0);
        assert_eq!(read(uniform_offset("camera", "camera_pos") as usize + 12), 20.0);
        assert_eq!(read(uniform_offset("camera", "model") as usize), 21.0);
        assert_eq!(read(uniform_offset("camera", "normal_mat") as usize + 60), 22.0);

        let outline = OutlineUniform {
            color: [1.0, 2.0, 3.0, 4.0],
            params: [5.0, 6.0, 7.0, 8.0],
            params2: [9.0, 10.0, 11.0, 12.0],
        };
        let bytes = bytemuck::bytes_of(&outline);
        let read = |offset: usize| {
            f32::from_le_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ])
        };
        assert_eq!(read(uniform_offset("outline", "color") as usize + 12), 4.0);
        assert_eq!(read(uniform_offset("outline", "params") as usize), 5.0);
        assert_eq!(read(uniform_offset("outline", "params2") as usize + 12), 12.0);
    }

    #[test]
    fn vertex_layout_is_the_frozen_72_byte_npr_vertex() {
        assert_eq!(vertex_stride(), 72);
        let attributes = vertex_attributes();
        assert_eq!(attributes.len(), 6);
        assert_eq!(attributes[0], (0, 0, wgpu::VertexFormat::Float32x3));
        assert_eq!(attributes[1], (1, 12, wgpu::VertexFormat::Float32x3));
        assert_eq!(attributes[2], (2, 24, wgpu::VertexFormat::Float32x2));
        assert_eq!(attributes[3], (3, 32, wgpu::VertexFormat::Float32x4));
        assert_eq!(attributes[4], (4, 48, wgpu::VertexFormat::Uint16x4));
        assert_eq!(attributes[5], (5, 56, wgpu::VertexFormat::Float32x4));
        for (_, offset, format) in &attributes {
            let size = match format {
                wgpu::VertexFormat::Float32x2 => 8,
                wgpu::VertexFormat::Float32x3 => 12,
                wgpu::VertexFormat::Float32x4 => 16,
                wgpu::VertexFormat::Uint16x4 => 8,
                other => panic!("formato {:?} sem tamanho declarado no teste", other),
            };
            assert!(*offset + size <= vertex_stride());
        }
        assert_eq!(uniform_offset("vertex_raw", "weights"), 56);
        assert_eq!(uniform_size("vertex_raw"), 72);
    }

    #[test]
    fn pass_graph_is_shared_with_the_viewport() {
        // Issue #14: a ordem vem do render graph (cel antes do outline: o
        // outline lê a profundidade já resolvida e o z-buffer garante o mesmo
        // resultado do desenho anterior).
        assert_eq!(
            render_pass_order(),
            vec!["depth_prepass", "cel", "outline"]
        );
        let graph = render_graph_spec();
        assert_eq!(graph["scheduling"].as_str(), Some("kahn_topological"));
        assert_eq!(graph["nodes"].as_array().map(Vec::len), Some(7));

        let outline = render_pass("outline");
        assert_eq!(outline.shader, "inverted_hull");
        assert_eq!(outline.cull_mode, Some(wgpu::Face::Front));
        assert!(!outline.depth_write);
        assert_eq!(outline.depth_compare, wgpu::CompareFunction::LessEqual);
        assert_eq!(outline.depth_bias.constant, 1);
        assert_eq!(outline.depth_bias.slope_scale, 1.0);
        assert_eq!(outline.blend, Some(wgpu::BlendState::ALPHA_BLENDING));

        let cel = render_pass("cel");
        assert_eq!(cel.shader, "cel_shading");
        assert_eq!(cel.cull_mode, Some(wgpu::Face::Back));
        assert!(cel.depth_write);
        assert_eq!(cel.blend, None);

        let morph = compute_pass("sparse_morph");
        assert_eq!(morph.workgroup_size, Some(64));
        assert_eq!(morph.only_when, Some("gpu_morph_active"));
        assert_eq!(morph.compute_entry, Some("cs_accumulate_morphs"));

        // P1-04 acrescentou o palette de ossos ao grupo 0: `bones` é o binding 5
        // do cel e o 2 do contorno (é o que o WGSL declara e o contrato congela).
        assert_eq!(
            bind_group_entries("cel"),
            vec![
                (0, "uniform"),
                (1, "uniform"),
                (2, "uniform"),
                (3, "texture_2d<f32>"),
                (4, "sampler"),
                (5, "uniform"),
            ]
        );
        assert_eq!(
            bind_group_entries("outline"),
            vec![(0, "uniform"), (1, "uniform"), (2, "uniform")]
        );
        assert_eq!(bind_group_entries("sparse_morph").len(), 5);
    }

    #[test]
    fn diagnostic_codes_are_unique_and_typed() {
        let codes = diagnostic_codes();
        assert!(codes.len() >= 20);
        let mut seen: Vec<&str> = Vec::new();
        for (code, severity) in &codes {
            assert!(!code.is_empty(), "código de diagnóstico vazio");
            assert!(!seen.contains(code), "código '{code}' duplicado");
            seen.push(code);
            assert!(
                diagnostic_severity(code).is_some(),
                "'{code}' com severidade '{severity}' não reconhecida"
            );
        }
    }

    #[test]
    fn targets_come_from_the_contract() {
        assert_eq!(offscreen_color_format(), wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(depth_format(), wgpu::TextureFormat::Depth24Plus);
        assert_eq!(msaa_sample_count(), 4);
        assert!(resolve_to_swapchain());
        assert_eq!(clear_color_source(), "scene.background_color");
    }

    #[test]
    fn toon_ramp_bytes_match_the_frozen_fingerprint() {
        let bytes = toon_ramp_bytes();
        assert_eq!(bytes.len(), 4096);
        assert_eq!(
            bytes.len() as u64,
            toon_ramp_spec()["bytes_len"].as_u64().unwrap_or(0)
        );
        assert_eq!(
            toon_ramp_fingerprint(),
            contract_hash_as_u64(expected_toon_ramp_fingerprint()),
            "toon ramp do Rust divergiu do contrato"
        );
        // linhas: 0 contínua, 1 degrau em 0.5, 2 degraus em 0.35/0.65, 3 em 0.25/0.5/0.75
        assert_eq!(bytes[0], 0);
        assert_eq!(bytes[256 * 4 * 1 + 128 * 4], 255);
        assert_eq!(bytes[256 * 4 * 3 + 64 * 4], 89);
        assert_eq!(bytes[256 * 4 * 3 + 128 * 4], 179);
        assert_eq!(bytes[3], 255);
        for pixel in bytes.chunks_exact(4) {
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
            assert_eq!(pixel[3], 255);
        }
    }

    #[test]
    fn toon_ramp_sampler_is_linear_clamped() {
        let spec = toon_ramp_spec();
        assert_eq!(spec["width"].as_u64(), Some(256));
        assert_eq!(spec["height"].as_u64(), Some(4));
        assert_eq!(
            filter_mode(spec["mag_filter"].as_str().unwrap_or("linear")),
            wgpu::FilterMode::Linear
        );
        assert_eq!(
            address_mode(spec["address_mode"].as_str().unwrap_or("clamp_to_edge")),
            wgpu::AddressMode::ClampToEdge
        );
    }

    /// O viewport declara os mesmos caminhos de shader que o contrato — o lado
    /// Rust só usa `include_str!` sobre esses arquivos, então os bytes batem por
    /// construção; o teste confirma que o caminho não mudou de lugar.
    #[test]
    fn shader_paths_are_the_canonical_ones() {
        assert_eq!(shader_path("cel_shading"), "crates/anigo-renderer/shaders/cel_shading.wgsl");
        assert_eq!(
            shader_path("inverted_hull"),
            "crates/anigo-renderer/shaders/inverted_hull.wgsl"
        );
        assert_eq!(
            shader_path("morph_sparse_compute"),
            "crates/anigo-renderer/shaders/morph_sparse_compute.wgsl"
        );
        assert_eq!(shader_entry_point("cel_shading", "vertex"), "vs_main");
        assert_eq!(shader_entry_point("cel_shading", "fragment"), "fs_main");
        assert_eq!(shaders_with_role("fallback_webgl2").len(), 4);
    }
}
