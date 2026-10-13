# Fronteira C/C++ do ANIGO (`anigo-native`)

Regra de ouro: **Rust puro é o padrão; C/C++ só entra quando é claramente melhor** (maduro,
caro de reescrever, ou sem equivalente Rust confiável) **e sempre atrás de uma feature
Cargo desligável**, de modo que o resto do app compile sem toolchain C++.

Toda a fronteira vive na crate `crates/anigo-native`. Nenhuma outra crate do workspace
depende de `meshopt`, `ufbx` ou do xatlas diretamente.

## 1. Estado atual

| feature (default) | biblioteca | origem | módulo | uso |
|---|---|---|---|---|
| `meshopt` | meshoptimizer 0.25 (C++) | crate `meshopt` 0.6.2, vendorizado, só `cc` | `simplify` | `simplify_mesh`, `optimize_mesh`, `generate_lods` |
| `ufbx` | ufbx 0.23.1 (C) | crate `ufbx` 0.11.5, vendorizado, só `cc` | `fbx` | `import_fbx` → `FbxScene` (`EditMesh` com n-gons, skin, blend shapes, clips) |
| `xatlas` | xatlas (C++, snapshot 2020) | **vendorizado em `vendor/xatlas`** + shim `extern "C"` em `native/`, `build.rs` com `cc` | `atlas` | `compute_atlas` / `apply_atlas` / `auto_unwrap` |

`anigo_native::capabilities()` devolve quais estão compilados (e as versões) para a UI
habilitar/desabilitar comandos. `cargo check -p anigo-native --no-default-features` compila só
com Rust (sem `cc`, sem compilador C++).

### Por que o xatlas não usa o crate `xatlas-rs`

* `xatlas-rs` **0.1.3** grava nomes de símbolo **MSVC x64** em `#[link_name]`
  (`?Create@xatlas@@YAPEAUAtlas@1@XZ`): compila em qualquer lugar, mas **não linka** em
  Linux/macOS/MinGW (`undefined symbol`, verificado). Funcionaria só no Windows/MSVC x64.
* `xatlas-rs-v2` 0.1.4 gera bindings com `bindgen` → exige **libclang/LLVM** instalado
  (incômodo no Windows).
* A API C++ do xatlas é mangled; a solução limpa é uma camada `extern "C"` própria.

Então: o `xatlas.cpp/.h` (MIT) foi copiado do crate publicado `xatlas-rs-v2` 0.1.4
(`vendor/source/xatlas/`, sha256 `xatlas.cpp` = `0ed0283a…52d5df4`, `xatlas.h` = `e7675335…218fbd`),
e `native/anigo_xatlas_shim.{h,cpp}` expõe duas funções C. Sem libclang, sem cmake, sem rede.

Quirk conhecido do snapshot vendorizado: o **modo polígono** (`MeshDecl::faceVertexCount`)
devolve índices de saída não inicializados (confirmado: leitura de lixo) — por isso o ANIGO
**triangula** o `EditMesh` (mesma triangulação do `to_core_mesh`) e mapeia os UVs de volta aos
corners. N-gons que o xatlas dividiria em charts diferentes são listados em
`AtlasResult::split_faces`.

## 2. Política de segurança de memória (vale para qualquer lib nativa)

1. **API pública = tipos do ANIGO.** `anigo_core::Mesh`, `anigo_modeling::EditMesh`, `Vec3`…
   **Nunca** ponteiros crus, `*mut`, `CString` ou tipos `-sys` na assinatura pública.
2. **Validar antes de cruzar.** O código nativo não valida nada: índices em faixa, `len % 3`,
   valores finitos, vetores de travas com `vertex_count` entradas, limites de tamanho
   (`MAX_TRIANGLES`, `max_input_bytes`, `MAX_CORNERS`…), atributos customizados consistentes.
   Entrada ruim vira `NativeError::{EmptyInput, Degenerate, InvalidInput, LimitExceeded}`.
3. **`unsafe` isolado.** Sem a feature `xatlas`, `#![forbid(unsafe_code)]`; com ela,
   `#![deny(unsafe_code)]` e **um único módulo** com `#![allow(unsafe_code)]`
   (`atlas/ffi.rs`), com cada bloco `unsafe` precedido de `// SAFETY:` listando invariantes.
   Crates de terceiros "seguros" (`meshopt`, `ufbx`) têm `unsafe` por dentro: tratamos suas
   funções como fronteira de confiança limitada — ver itens 2 e 4.
4. **Nunca deixar um pânico/unwind cruzar a fronteira.** Todo ponto de entrada passa por
   `guarded(lib, || …)` (`catch_unwind`) → `NativeError::Panic`. No C++, o shim captura
   `catch (...)` e devolve código de erro; exceções C++ jamais atravessam para Rust.
5. **Posse de memória clara.** Entradas: ponteiros de leitura válidos só durante a chamada.
   Saídas: copiadas para `Vec` Rust imediatamente; a memória do C++ é liberada por um guard
   (`Drop`) mesmo em erro. O wrapper de terceiros que devolve `Vec` já copia (meshopt).
6. **Defesa em profundidade na saída:** conferir que índices devolvidos estão em faixa antes
   de indexar vetores Rust (feito em `simplify` e `atlas/ffi.rs`).
7. **Limites de recursos em parsers:** ufbx recebe `memory_limit`, `allocation_limit`,
   `node_depth_limit`; `load_external_files = false` (não segue caminhos do arquivo).
8. **Determinismo:** mesma entrada → mesma saída (testado em simplify, atlas e FBX). Se a lib
   tiver threads, comparar duas execuções nos testes.
9. **Layout/ABI:** structs `#[repr(C)]` espelhadas no header C; teste que compara
   `size_of` Rust com `sizeof` C++ (`abi_sizes_match_cpp`).

## 3. Como adicionar uma biblioteca C/C++

Checklist (nesta ordem):

1. **Decidir se precisa mesmo** (seção 5): existe equivalente Rust puro maduro? O ganho
   justifica um toolchain C++ no CI e no Windows?
2. **Escolher o mecanismo de binding:**
   * C simples ou poucas funções → `extern "C"` à mão + `cc` (como o xatlas aqui); *ou* um
     crate `-sys` vendorizado do crates.io que compile só com `cc`.
   * C++ com classes/`std::string`/vetores → **`cxx`** (bridge tipado, sem `bindgen`/libclang).
   * Evitar `bindgen`/`autocxx` (exigem libclang) e builds que fazem `git clone`/download
     no `build.rs` (não funcionam offline nem em CI restrito).
   * Preferir fontes **vendorizadas dentro do repositório ou do crate** com versão e hash
     registrados (como a tabela do item 1).
3. **Feature Cargo própria** em `anigo-native/Cargo.toml` (`default` só se o custo for baixo):
   dependências `optional = true`, `foo = ["dep:foo_sys"]`; build-dependencies (`cc`,
   `cxx-build`, `cmake`) também opcionais, ativadas pela feature (`cfg(feature = "foo")` vale no
   `build.rs`).
4. **Módulo** `src/foo.rs` (ou `src/foo/{mod,ffi}.rs`) com `#[cfg(feature = "foo")]` em
   `lib.rs`, reexportando só a API segura. Se houver `unsafe`, ele fica em `ffi.rs`.
5. **Wrapper seguro:** funções que recebem/devolvem tipos do ANIGO; validação de entrada;
   `guarded(…)`; erros em `NativeError`; opções em struct com `Default`.
6. **Registrar em `capabilities()`** (flag + string de versão upstream e do crate).
7. **Testes** (obrigatórios): caso feliz com asserção de *propriedade* (não só "não falhou":
   fechamento/Euler, bbox, área, UV em [0,1]…); entradas vazias, degeneradas, NaN, truncadas
   e lixo → `Err` sem pânico; determinismo; limites; e `cargo check --no-default-features`.
8. **Documentar** aqui: versão, origem, hash, quirks, o que exige validação manual no Windows.
9. **CI:** job com `--no-default-features` (sem C++) e job com tudo ligado.

### Esqueleto `cxx` (para libs C++ futuras)

```rust
// src/foo/ffi.rs  — único módulo com unsafe/bridge
#[cxx::bridge(namespace = "anigo")]
mod bridge {
    struct FooParams { quality: f32, max_iterations: u32 }
    unsafe extern "C++" {
        include!("anigo-native/native/foo_shim.h");
        // Slices Rust ↔ rust::Slice<const T> no C++: tamanho viaja junto, sem ponteiro cru.
        fn foo_run(positions: &[f32], indices: &[u32], p: &FooParams, out_indices: &mut Vec<u32>) -> Result<()>;
    }
}
```

O shim C++ converte `std::exception` em `Result::Err` (o `cxx` já faz isso para funções que
retornam `Result`). O build usa `cxx_build::bridge("src/foo/ffi.rs").file("native/foo_shim.cc")`
com `-std=c++17` (`flag_if_supported`).

## 4. Validação local no Windows (MSVC)

O que foi compilado e testado aqui foi **Linux x86_64 (clang/g++ via `cc`)**. No Windows:

* **Toolchain:** Visual Studio Build Tools 2022 com "Desktop development with C++" e o Windows SDK
  (Rust `x86_64-pc-windows-msvc`). `cc` acha o MSVC sozinho; rodar a partir do "x64 Native Tools
  Command Prompt" evita falha de detecção. Não é preciso cmake nem LLVM para as 3 features atuais.
* **`xatlas`:** o `build.rs` usa `flag_if_supported("-std=c++11")` e `-pthread` (ignorados/rejeitados
  pelo MSVC sem erro) e define `NDEBUG`. `std::thread` do xatlas funciona com o CRT do MSVC.
  Atenção à **CRT estática × dinâmica** se o app usar `-C target-feature=+crt-static`: `cc` detecta `crt-static`
  via `CARGO_CFG_TARGET_FEATURE`; todo o binário deve usar o mesmo CRT.
* **`ufbx` / `meshopt`:** compilam com `cc` no MSVC (testados upstream); `ufbx.c` é um arquivo de 1,2 MB,
  então o primeiro build é mais lento (depois fica em cache).
* **Caminhos longos / antivírus:** o `target/` com vários `.obj` grandes pode ser lento sob antivírus;
  excluir a pasta do build do Defender ajuda.
* **Ao adicionar libs com cmake** (OpenSubdiv, Manifold, QuadriFlow): instalar CMake (≥ 3.20) e Ninja
  no PATH; o crate `cmake` escolhe o gerador do Visual Studio. Para `bindgen`/`autocxx` instalar
  LLVM e definir `LIBCLANG_PATH` (apontando a `...\LLVM\bin`) — **evitar** quando houver alternativa
  (`cxx`). Usar o mesmo perfil (Debug/Release) e a mesma CRT em todas as libs C++ linkadas.
* **Testes a rodar à mão:** `cargo test -p anigo-native` e `cargo check -p anigo-native
  --no-default-features`; abrir um FBX real (Blender/Maya/Mixamo) com `import_fbx` e conferir
  escala em metros, Y-up e sombreamento; rodar `auto_unwrap` num personagem e inspecionar o atlas.

## 5. "C++ depois": candidatos e critérios de migração

Estado: **Rust puro agora**; migrar para C++ só se o critério da coluna 3 for atingido.

| candidato | licença | migrar quando… | forma de integrar | risco de build (Windows/MSVC) |
|---|---|---|---|---|
| **OpenSubdiv** (GPU/limit patches) | Apache-2.0 (Tomorrow OSTL) | precisarmos de superfície-limite exata, avaliação em GPU, ou paridade bit-a-bit com Blender/Maya; o Catmull-Clark em Rust (CPU, níveis 1–3) ficar lento num cage denso | `opensubdiv-petite(-sys)` (vendorizado, `cmake` + **bindgen/libclang**) como feature `opensubdiv`; `opensubdiv-rs` (Rust) como oráculo de testes | **alto**: cmake + LLVM/libclang; README cita MSVC; features cuda/metal/tbb opcionais |
| **Manifold** (booleanas) | Apache-2.0 | `boolmesh` (Rust, MPL-2.0) falhar em robustez/performance nos testes (malhas grandes, quase-coplanares) | `manifold-csg` (binding seguro) — o `build.rs` faz **`git clone` do GitHub** (não builda offline) → usar `MANIFOLD_CSG_LIB_DIR` com lib pré-compilada/vcpkg; alternativa Rust `manifold-rust` | **médio/alto**: git + cmake + C++17; TBB opcional |
| **QuadriFlow** (retopo automática) | BSD-3 | usuários pedirem retopo automática de quads (hoje: retopo manual com snap) | **processo separado** (CLI) ou lib via `cxx`: um crash/OOM não pode derrubar o editor; entrada/saída por arquivo ou pipe | **alto**: Eigen, sem crate; compilar do GitHub |
| **OpenUSD** | Apache-2.0 (Modified) | integração de pipeline com DCCs/studios (troca de cenas) virar requisito | avaliar primeiro `openusd` (Rust puro, MIT); binding C++ (`rust-usd`) é imaturo e pesado | **muito alto** (centenas de MB, Boost/TBB) |
| **Draco** | Apache-2.0 | exportação glTF com `KHR_draco_mesh_compression` virar requisito de entrega | `draco-rs` (vendorizado, cmake + autocxx/libclang) ou ferramenta externa (`gltf-transform`) | **alto**: cmake + libclang |

Critérios gerais para migrar de Rust puro para C++:

1. **Corretude medida:** teste de regressão que o Rust falha e o C++ passa (robustez), não só "o C++ é mais rápido".
2. **Perf medida:** ≥ 3× no caso real do ANIGO (personagens anime, 10–100k tris) e o gargalo é percebido pelo usuário.
3. **Manutenção:** a lib tem releases ativos e licença compatível (evitar GPL/LGPL; MPL ok se não modificarmos).
4. **Isolamento:** pode ficar atrás de feature + trait (`BooleanBackend`, `SubdivBackend`) com o backend Rust como padrão;
   o app sem toolchain C++ continua funcional.
5. **Custo de build aceito** no CI e na máquina do artista (tabela acima); preferir `cxx` a `bindgen`.
6. **Operações perigosas/lentas** (QuadriFlow, Draco) vão para **processo separado** com timeout.

## 6. Limites conhecidos das integrações atuais

* `meshopt`: só triângulos; o "Decimate" de edit mode em n-gons (QEM) continua sendo do `anigo-modeling`.
  Erro do simplificador é relativo à extensão da malha; vértices nunca são movidos (colapsos de aresta).
* `ufbx`: sem cor de vértice, sem câmeras/luzes, sem NURBS/skin dual-quaternion/caches de geometria;
  materiais só com cor base e textura de cor base; animação via *bake* TRS a 30 fps (configurável).
  A rotação de eixos fica no TRS dos nós (use `FbxScene::mesh_in_world_space` para malha em Y-up).
* `xatlas`: snapshot de 2020 sem manutenção; um único atlas (erro se `texels_per_unit` × `resolution`
  não couber); charts automáticos fragmentam personagens — para unwrap autoral use o módulo `uv` de
  `anigo-modeling` e deixe o xatlas para lightmaps/atlas rápidos.
