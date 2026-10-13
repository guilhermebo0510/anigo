# ANIGO - Pesquisa: C++ vs Rust para o workspace de MODELAGEM (out/2026)

Metodo: sparse index + API do crates.io, download e inspecao dos `.crate` em `crates_inspect/<nome>/` (NADA executado), paginas oficiais do Blender e dos projetos.
Versoes/datas abaixo = estado em 11/out/2026. "Nao testado" = nao rodei build.rs de codigo baixado.

## 0. Resumo executivo (veredito)

| Item | Veredito | Escolha |
|---|---|---|
| Subdivisao (Catmull-Clark + creases + UV face-varying) | **Rust puro agora** | CC proprio; `opensubdiv-rs` como oraculo de testes |
| Booleanas | **Rust puro agora** (atras de trait) / C++ depois | `boolmesh`; fallback `manifold-csg` (so no PC do usuario) |
| Voxel remesh / dyntopo | **Rust puro agora** | `fast-surface-nets` + SDF proprio; OpenVDB: nao |
| Retopo automatica (quads) | **C++ depois** | QuadriFlow (BSD-3) como ferramenta/CLI separada |
| UV unwrap manual (seams + LSCM/ABF) + pack | **Rust puro agora** | proprio + `crunch`/`rectangle-pack` |
| UV auto (Smart UV / atlas) | **C++ agora (barato)** | `xatlas-rs` 0.1.3 (so `cc`) |
| Tangentes MikkTSpace | **Rust puro agora** | `bevy_mikktspace` 1.0.0 |
| Decimate/LOD/otimizacao de indices | **C++ agora (barato)** | `meshopt` 0.6.2 (vendorizado) |
| Import FBX | **C agora (barato)** | `ufbx` 0.11.5 (vendorizado, so `cc`) |
| Import assimp / Draco / OpenUSD | nao / depois / depois | `russimp` nao; `draco-rs` so se precisar; `openusd` (Rust) avaliar |
| Estrutura de malha | **Rust puro (proprio)** | SoA estilo Blender `Mesh` + `EditMesh` estilo BMesh; BVH proprio |

Achado critico de ambiente: **o unico binding C++ "bom" de booleanas (`manifold-csg-sys`) faz `git clone github.com/elalish/manifold` no build.rs** -> impossivel no sandbox; funciona no PC do usuario. Crates com C/C++ realmente vendorizado e que compilam aqui so com `cc`: `meshopt`, `xatlas-rs`, `ufbx`.

## 1. Subdivisao

- **O que o Blender usa:** OpenSubdiv (Pixar) para o modificador Subdivision Surface (CPU e GPU no viewport). Licenca OpenSubdiv: Apache-2.0 (3.7 renomeou para "Tomorrow Open Source Technology License", mesmos termos). Ultima release: **3.7.0 (30/out/2025)** - [notas](https://graphics.pixar.com/opensubdiv/docs/release_37.html).
- **Crates:**

| Crate | Versao / data | Licenca | Natureza | Notas |
|---|---|---|---|---|
| [`opensubdiv-petite-sys`](https://crates.io/crates/opensubdiv-petite-sys) + [`opensubdiv-petite`](https://crates.io/crates/opensubdiv-petite) (virtualritz) | 0.3.1, mai/2026, ~4.7k dl | Apache-2.0 | C++ **vendorizado** (OpenSubdiv 3.7.0, 246 arquivos em `OpenSubdiv/opensubdiv`) via `cmake` crate | Exige **cmake + bindgen (libclang/LLVM)** sempre (build-dep nao opcional). Features cuda/metal/tbb/omp opcionais. README cita MSVC como suportado. **Nao testado** (sandbox tem cmake/clang/g++, entao deveria buildar). |
| [`opensubdiv-sys`](https://crates.io/crates/opensubdiv-sys) / `opensubdiv` | 0.0.0 / 0.1.0, 2020 | - | placeholders | Ignorar. |
| [`opensubdiv-rs`](https://crates.io/crates/opensubdiv-rs) (doubleailes) | 0.5.0, 02/out/2026, ~250 dl | MIT (!) | **Port Rust puro** "fiel" (Sdc/Vtr/Far: Catmark/Loop/Bilinear, creases semi-sharp, face-varying, stencils, patch tables), `#![forbid(unsafe_code)]`, zero deps | Muito novo, pouca adocao; licenca declarada MIT difere do original Apache - checar atribuicao antes de embutir. Bom como **oraculo de teste** ou dependencia opcional. |
| [`subdiv-kernels`](https://crates.io/crates/subdiv-kernels) (virtualritz) | 0.1.0, jun/2026, 33 dl | MIT/Apache/Zlib/BSD | Rust puro: Catmull-Clark, Loop, sqrt3, Doo-Sabin -> `StencilTable` + canais face-varying (UV) + limit stencils | Design bom para animacao (recompor stencils uma vez, reavaliar por frame). Imaturo. |

- **Rust puro (CC proprio):** Catmull-Clark em malha de n-gons com `crease` por aresta, corner/bordas e interpolacao de atributos de **corner** (UV, normais customizadas) e ~400 linhas. Para anime o cage e baixo-poly, niveis 1-3, CPU basta. Perda vs OpenSubdiv: superficie-limite exata (patches de Stam/gregory) e avaliacao GPU - necessarios so para paridade bit-a-bit com Blender/Maya.
- **Recomendacao: Rust puro agora.** Implementar CC proprio com testes comparando com `opensubdiv-rs` (dev-dependency). Deixar `opensubdiv-petite` (C++) como feature opcional "depois" se precisar de GPU Osd/limit patches. Risco C++: libclang no Windows + cmake; esforco ~1 dia para ligar, mas ganho baixo agora.

## 2. Booleanas robustas

- **Blender:** **Correcao importante:** no 4.5 o solver **Manifold foi ADICIONADO** (3o solver); Exact continua (e lida com malhas nao-manifold/overlap, com opcoes Self Intersection / Hole Tolerant). "Fast" foi renomeado para **"Float" no 5.0**; no 5.1 Exact ficou >35% mais rapido com muitos atributos e Manifold um pouco melhor. [4.5 manual](https://docs.blender.org/manual/de/4.5/modeling/modifiers/generate/booleans.html), [5.0 notes](https://developer.blender.org/docs/release_notes/5.0/modeling/), [5.1 notes](https://developer.blender.org/docs/release_notes/5.1/modeling/). Manifold exige entradas fechadas/manifold.
- **Upstream:** [elalish/manifold](https://github.com/elalish/manifold), **v3.5.4 (25/set/2026)**, Apache-2.0, TBB opcional.

| Opcao | Versao / data | Licenca | Natureza | Veredito |
|---|---|---|---|---|
| [`boolmesh`](https://crates.io/crates/boolmesh) (komietty) | 0.2.1, 11/out/2026, 19k dl, MSRV 1.88 | **MPL-2.0** | **Rust puro**, reimplementacao "do zero" inspirada no Manifold; deps: `glam` **0.32** + `rayon` opcional; f32/f64 | **Melhor candidato.** Diferencial: `Manifold::new_with_values` carrega **UV/cor/normal por canto de triangulo** atraves da boolean (costuras de UV preservadas). Exige entrada manifold fechada (igual ao solver Manifold do Blender). MPL = copyleft por arquivo (ok para linkar; modificacoes em arquivos dele devem ser publicadas). glam diferente do nosso 0.29 -> converter em `[f32;3]` na borda. |
| [`manifold-rust`](https://crates.io/crates/manifold-rust) (Lars Brubaker) | 0.16.0, 03/out/2026, 12k dl, criado jul/2026 | Apache-2.0 | **Port Rust puro** do Manifold 3.5.0 + 2o motor "robusto" (aritmetica racional exata, tolera nao-manifold/auto-intersecao, testado em Thingi10K) | Mais completo (hull, Minkowski, SDF, cross-section) e cobre o caso "Exact" do Blender, mas **muito novo** (3 meses) e grande (830 KB). Segundo fallback. |
| [`manifold-csg`](https://crates.io/crates/manifold-csg) / `manifold3d` / `manifold-csg-sys` (zmerlynn) | 0.4.1 / 3.5.105, set/2026, ~49k dl | Apache-2.0 OR MIT | **Binding C++** seguro (f64, Send) | **NAO vendoriza**: `build/fetch.rs` faz `git clone https://github.com/elalish/manifold` (tag v3.5.3) + cmake -> **falha no sandbox**; no PC do usuario ok (git+cmake+C++; feature `parallel` usa TBB). Env `MANIFOLD_CSG_LIB_DIR` permite lib pre-compilada/vcpkg. |
| [`manifold-rs`](https://crates.io/crates/manifold-rs) (WilstonOreo/microcad) | 0.7.0, fev/2026 | MIT OR Apache-2.0 | C++ vendorizado, mas **Manifold 3.0.0 antigo**, exige **Ninja**, `MANIFOLD_PAR=OFF`, exceptions off | Nao recomendado (versao velha). |
| [`csgrs`](https://crates.io/crates/csgrs) | 0.20.1, jul/2025 | MIT | Rust puro, BSP, nalgebra | Nao robusto (BSP classico); nao. |
| CGAL | - | GPL/LGPL(+comercial) | C++ pesado, Boost/GMP | **Descartar** (licenca + peso). |

- **Recomendacao: Rust puro agora (`boolmesh`) atras de um trait `BooleanBackend`**; `manifold-csg` como feature opcional "C++ depois" (PC do usuario) se os testes de robustez/perf falharem; `manifold-rust` robusto se precisar de entradas nao-manifold. Prioridade baixa no roadmap: boolean serve mais a props/hard-surface que a personagens anime.

## 3. Remesh / voxel / escultura / retopo

- **Blender (manual):** Voxel Remesh (malha uniforme, remove geometria interna) e **Quad Remesh = QuadriFlow** (melhor, mais lento, nao limpa auto-intersecao); Dyntopo causa overhead e o proprio manual recomenda voxel remesh antes de esculpir. [retopologia](https://docs.blender.org/manual/en/dev/modeling/meshes/retopology.html).

| Ferramenta | Licenca | Rust? | Avaliacao para editor anime |
|---|---|---|---|
| `fast-surface-nets` 0.2.1 (jan/2025) | MIT/Apache | Rust puro, **glam 0.29 (igual ao nosso)** | SDF -> malha. Base do voxel remesh (precisa de mesh->SDF proprio via BVH + winding number). **Usar.** |
| [`baby_shark`](https://crates.io/crates/baby_shark) 0.3.12 (set/2025, 34k dl) | MIT | Rust puro | Lista voxel remesh, remesh incremental, decimation, boolean, ARAP. Util como referencia/atalho; qualidade nao verificada por mim. |
| [`freestyle-sculpt`](https://crates.io/crates/freestyle-sculpt) 0.12.0 (13/set/2026, 3k dl) | MIT | Rust puro | Dyntopo ("Freestyle" Stanculescu et al.), mesmo algoritmo do Dyntopo do Blender; exemplo Bevy. Avaliar; acoplada a seu `MeshGraph`. |
| OpenVDB | MPL-2.0 | sem binding util (`openvdb` crate = placeholder 2020) | Pesado (Boost/TBB/Blosc); **nao vale** para anime. |
| QuadriFlow ([hjwdzh](https://github.com/hjwdzh/QuadriFlow)) | **BSD-3** (verificado) | nao existe crate | Retopo automatica que o Blender oferece. Binding C++ via `cmake`+`cxx` ou CLI externo. **Depois.** |
| Instant Meshes ([wjakob](https://github.com/wjakob/instant-meshes)) | **BSD-3** (verificado) | [`quadrs`](https://crates.io/crates/quadrs) 1.0.0 (mar/2026, MIT, 122 dl, "experimental") | Port Rust experimental; imaturo. |
| QuadWild | GPL-3 (nao verificado em fonte primaria) | nao | Evitar (copyleft forte). |
| libigl (MPL-2.0) / geometry-central (MIT) | - | sem bindings | Nao vale ligar; portar so algoritmos pontuais (LSCM, geodesica). |

- **Veredito:** voxel remesh + sculpt (brushes, symmetry, mask, smooth/relax, multires) = **Rust puro agora**. Dyntopo: avaliar `freestyle-sculpt`, nao bloqueante (anime = pouca escultura de detalhe; malha limpa por loops). Retopo automatica = **C++ depois** (QuadriFlow como ferramenta isolada: falha/crash nao derruba o editor). Artistas de anime normalmente fazem retopo manual (snap a superficie + BVH).

## 4. UV, tangentes, decimate, importadores

| Item | Crate / versao / data | Licenca | Natureza | Veredito |
|---|---|---|---|---|
| xatlas | [`xatlas-rs`](https://crates.io/crates/xatlas-rs) 0.1.3 (2019, 7.5k dl): `vendor/xatlas.cpp`, `cc`, bindings pre-gerados (bindgen so com feature) | MIT | C++ vendorizado, **compila no sandbox** | xatlas upstream: ultimo commit **jul/2022** (estavel, sem manutencao), MIT. Faz charts+pack automaticos (lightmap/Smart UV) - **ruim para personagem** (charts fragmentados, ignora intencao do artista). Usar so como "auto-unwrap/atlas". |
| | [`xatlas-rs-v2`](https://crates.io/crates/xatlas-rs-v2) 0.1.4 (nov/2025) | MIT | fork republicado; exige **bindgen/libclang** no build | Preferir 0.1.3 (sem libclang). |
| Unwrap Blender-like | - | - | Blender usa Angle Based e Conformal (LSCM) por ilha; pack separado | **Rust puro**: LSCM/ABF por ilha com solver esparso (CG) a partir de seams marcados + pack com [`crunch`](https://crates.io/crates/crunch) 0.5.3 / [`rectangle-pack`](https://crates.io/crates/rectangle-pack). `halfedge` 0.3.0 afirma LSCM/Tutte/MVC (nao verificado). |
| MikkTSpace | [`bevy_mikktspace`](https://crates.io/crates/bevy_mikktspace) **1.0.0 (17/fev/2026)**, 7.8M dl | Zlib AND (MIT OR Apache-2.0) | **Rust puro**, zero deps, trait `Geometry` generica | **Usar.** (`mikktspace` 0.3.0 de 2022 esta defasado, glam 0.15.) Alimentar com normais por canto (custom normals). |
| Decimate/LOD | [`meshopt`](https://crates.io/crates/meshopt) 0.6.2 (out/2025, 600k dl): vendoriza meshoptimizer **v0.25** em `vendor/src`, `cc` | MIT OR Apache-2.0 | C++ vendorizado, **compila no sandbox**, bindgen opcional | **C++ agora**: simplify com pesos de atributos/travas de borda, cache de vertices, quantizacao. Alternativa pura: [`meshoptimizer-rs`](https://crates.io/crates/meshoptimizer-rs) 0.2.1 (out/2026, port do 1.3, 924 dl - novo). Triangulos apenas; o "Decimate" de edit mode (QEM em n-gons) fica proprio. |
| FBX | [`ufbx`](https://crates.io/crates/ufbx) **0.11.5 (25/set/2026)**, 562k dl: `ufbx/ufbx.c` (1,2 MB) + `cc` | MIT OR Unlicense | **C vendorizado**, compila no sandbox | **Agora**. Cobre mesh, normais por canto, skin, blend shapes, animacao. Melhor opcao FBX. |
| assimp | `russimp` 3.2.1 / `russimp-sys` 2.0.2 (jan/2024) | MIT | `build.rs` baixa binario de `github.com/jkvargas/...releases` ou compila (`build-assimp`) | **Nao**: bloqueado no sandbox, pesado, ufbx+gltf bastam. |
| Draco | [`draco-rs`](https://crates.io/crates/draco-rs) 0.1.3 (abr/2025) | MIT | draco vendorizado (~470 arquivos) via cmake + autocxx (libclang) | So se exportar glTF comprimido. Depois/nunca. |
| OpenUSD | [`openusd`](https://crates.io/crates/openusd) 0.8.0 (11/out/2026, 27k dl) Rust puro, MIT; `rust-usd` 0.0.5 (binding pxr C++, imaturo) | MIT | - | Avaliar `openusd` (Rust) para troca com DCCs; **depois**. |

## 5. Estruturas de dados de malha

**Blender moderno (referencia):**
- `Mesh` (objeto/modo objeto) e SoA por **dominios** (point/edge/face/corner), tudo como **atributos genericos**: `position`, `.edge_verts`, `face_offsets` (OffsetIndices: n+1 inteiros em vez de 12 B/face), `.corner_vert`, `.corner_edge`; MPoly/MLoop/MEdge/MVert foram removidos entre 3.6 e 4.0 ([PR #106638](https://projects.blender.org/blender/blender/pulls/106638)).
- `BMesh` (so edit mode): loop-radial (BMVert/BMEdge/BMLoop/BMFace, n-gons, CustomData por elemento); entra/sai com conversao Mesh<->BMesh.
- **Normais (4.1+):** "Auto Smooth" removido; sharpness = atributos bool `sharp_edge`/`sharp_face`, normais customizadas opcionais por canto; Blender escolhe automaticamente vertex/face/**corner normals**; normais de canto sao **cacheadas**; novo "Set Sharpness by Angle" ([4.1](https://developer.blender.org/docs/release_notes/4.1/modeling/)). No Geometry Nodes, "Set Mesh Normal" tem modos Sharpness / Free (vetores no espaco local, nao deforma) / **Tangent Space** (normais num espaco dependente da deformacao - sobrevive a skin/shape keys) ([manual 5.0](https://docs.blender.org/api/5.0/bpy.types.GeometryNodeSetMeshNormal.html)). **5.1:** "Better Custom Normals" - Cycles passa a usar normais por canto em vez de dividir geometria ([release 5.1](https://www.blender.org/download/releases/5-1/)).
- **5.0:** tags Freestyle viram atributos (`freestyle_edge/face`; Mesh ainda nao), selecao de UV compartilhada entre UV maps (`.uv_select_vert/edge/face`), solver "Fast" -> "Float", 6 modificadores baseados em nos ([5.0](https://developer.blender.org/docs/release_notes/5.0/modeling/)). **5.2 LTS (14/jul/2026):** LoopTools embutido (To Circle, Space Edge Loops Evenly, Flatten), interpolacao de custom data ao fazer merge, selecao atraves de faces culled, delimitadores de select linked na UV ([5.2](https://developer.blender.org/docs/release_notes/5.2/modeling/)). 5.1: delimitadores em loop/ring select, polygon fill ~5x mais rapido.

**Crates Rust (veredito):**

| Crate | Estado | Uso |
|---|---|---|
| `hedge` 0.2.1 (2019), `plexus` 0.0.11 (2019) | abandonados | Nao. |
| [`halfedge`](https://crates.io/crates/halfedge) 0.3.0 (set/2026, 293 dl, MIT/Apache, ~24k linhas) | novo, escopo enorme (subdiv, QEM, LSCM, boolean, remesh, BVH, glTF), slotmap | Estudar; nao depender ainda. |
| `smesh` 0.2.7 | MPL-2.0, estilo Surface_mesh | Referencia. |
| `bevy_mesh` | acoplada ao Bevy | Nao. |
| `parry3d` 0.31.1 (set/2026, Apache-2.0) | maduro; TriMesh+BVH, raycast, closest point; usa `glamx`/**glam 0.33** | Opcional p/ queries; conflito de versao de glam. |
| `bvh` 0.12.0 (nalgebra) | MIT | Possivel, mas traz nalgebra. |
| `rstar` 0.13 / `spade` 2.15.1 | maduros, MIT/Apache | rstar: snap de vertices; spade: triangulacao 2D/CDT (fill de n-gon, UV). |
| `nalgebra` vs `glam` | - | Manter **glam**. |

**Recomendacao: estrutura propria** em dois niveis, estilo Blender:
1. `Mesh` SoA imutavel/Arc (copy-on-write, undo barato, upload direto p/ wgpu): `positions`, `face_offsets`, `corner_verts`, `corner_edges`, `edge_verts` + atributos nomeados por dominio (`sharp_edge`, `sharp_face`, `crease`, UV por canto, `corner_normals` opcional, pesos de skin, mascara de sculpt).
2. `EditMesh` (BMesh-like, loop-radial ou half-edge com n-gons, indices estaveis + free-list) construido ao entrar em edit mode; operadores (extrude, bevel, loop cut, bridge, knife) vivem aqui.
3. BVH proprio (~300 linhas, AABB, SAH binning, glam 0.29) para picking/raycast/closest/shrinkwrap; ver `parry3d`/`bvh` so se precisar de mais.
Para cel shading: normais por canto sao dado de primeira classe (cache invalidado por topologia/posicao), com ferramentas de transferencia de normais (de proxy mesh) e modo tangent-space para sobreviver a deformacao.

## 6. Matriz de risco / esforco

| Item | Escolha | Natureza | Risco | Esforco |
|---|---|---|---|---|
| Subdivisao | CC proprio (+ `opensubdiv-rs` oraculo) | Rust | Baixo (creases/fvar exigem cuidado) | 2-4 dias |
| Subdiv exata/GPU | `opensubdiv-petite` | C++ depois | Medio (libclang+cmake no Windows) | 1-2 dias |
| Booleanas | `boolmesh` (trait) | Rust agora | Medio (jovem, so manifold, MPL) | 1-2 dias |
| Booleanas robustas | `manifold-csg` / `manifold-rust` | C++ depois / Rust | Medio (git clone no build) | 1 dia |
| Voxel remesh | `fast-surface-nets` + SDF | Rust | Baixo | 3-5 dias |
| Dyntopo | `freestyle-sculpt` (avaliar) | Rust | Medio | 3+ dias |
| Retopo auto | QuadriFlow | C++ depois | Medio (Eigen, ferramenta isolada) | 3-5 dias |
| UV manual+pack | LSCM/ABF proprio + `crunch` | Rust | Medio (qualidade do solver) | 1-2 sem |
| UV auto | `xatlas-rs` 0.1.3 | C++ agora | Baixo (sem manutencao upstream) | 0,5 dia |
| MikkTSpace | `bevy_mikktspace` | Rust | Baixo | 0,5 dia |
| Decimate/LOD | `meshopt` | C++ agora | Baixo | 0,5-1 dia |
| FBX | `ufbx` | C agora | Baixo | 1-2 dias |
| Estruturas | proprias | Rust | Medio (nucleo do editor) | 2-3 sem |

## 7. O que builda neste sandbox (teoria, nao executado)
- Compila so com `cc`/vendorizado: `meshopt`, `xatlas-rs` 0.1.3, `ufbx`.
- Vendorizado mas exige cmake (+ libclang p/ bindgen): `opensubdiv-petite-sys` (cmake/clang/g++ existem aqui), `draco-rs`, `manifold-rs` (+Ninja).
- Bloqueado (rede github): `manifold-csg-sys` (git clone), `russimp-sys` (download de release), qualquer build de QuadriFlow/OpenSubdiv a partir do GitHub.
- Windows/MSVC: `manifold-csg` declara testado em Windows; `opensubdiv-petite` cita MSVC; `xatlas-rs` 0.1.3 so passa flags -std em linux/darwin (MSVC usa padrao, ok); `xatlas-rs-v2` pede `XATLAS_MSVC_CRT` opcional.
- Duplicidade de glam: `boolmesh` pede glam 0.32, `parry3d` (glamx) 0.33, `fast-surface-nets` 0.29. Fronteiras com `[f32;3]`/slices.
