# Workspace de Modelagem — mapa de ferramentas (Blender como referência nº 1)

Escopo: **modelagem orgânica de personagens anime e assets de cenário**, não paramétrica.
Fora de escopo (por foco em anime): simulação de fluidos/fumaça, Geometry Nodes completo,
compositor, video sequencer, Grease Pencil 3D completo, escultura multi-resolução de
alta densidade (>1M polígonos).

Motor: crate `anigo-modeling` (Rust puro, sem GPU, 100 % testável em CI). A UI nativa
(`anigo-studio`) só chama o motor; viewport desenha o resultado com o renderer canônico.

Convenções (valem para todos os módulos):
* Y para cima; enrolamento anti-horário visto de fora = normal para fora (igual glTF).
* Nada de `unwrap()/expect()` em dado de entrada; erros via `ModelError`.
* Validar antes de mutar: em `Err`, a malha fica intacta.
* Determinismo: mesma entrada → mesmo `EditMesh::content_hash()`. Nada de `HashMap` iterado
  sem ordenar quando a ordem afeta o resultado.
* Cada operador tem testes de: caso feliz, entrada inválida, invariantes topológicos
  (`Topology::is_closed_manifold`, Euler, `EditMesh::validate`) e um caso não-manifold/fronteira.
* Atributos que viajam: UV por corner, normal customizada por corner, cor/skin/máscara por
  vértice, material/suave/face-set por face, sharp/seam/crease/bevel por aresta.

Legenda de validação: **A** = teste automático em CI · **B** = render em software (lavapipe)
com imagem de referência · **H** = precisa de olho/mão humana (listado em `VALIDACAO_LOCAL.md`).

## 1. Estado de dados e infra (✅ feito nesta sessão)

| Item | Módulo | Val. |
|---|---|---|
| Malha n-gon editável, atributos por corner/aresta/vértice/face | `mesh.rs` | A |
| Adjacência derivada, componentes, Euler, fronteira, não-manifold | `topology.rs` | A |
| Seleção v/e/f, grow/shrink, linked, invert, flush | `select.rs` | A |
| Primitivas: plano, grid, cubo (subdiv), quad-sphere, UV-sphere, cilindro, cone, cápsula, toro, círculo | `primitives.rs` | A |
| Normais por corner (sharp, ângulo, peso, custom) | `normals.rs` | A |
| Ponte `anigo_core::Mesh` ⇄ `EditMesh` (solda, triangulação, quadify) | `convert.rs` | A |
| Undo/redo por snapshot e por delta de posição | `history.rs` | A |

## 2. Operadores de Edit Mode (`ops/`)

| Ferramenta Blender | Arquivo | Prioridade | Val. |
|---|---|---|---|
| Extrude (região/individual/vértices/arestas) | `extrude.rs` | P0 | A |
| Inset (região/individual, espessura, profundidade, outset) | `inset.rs` | P0 | A |
| Bevel (arestas/vértices; largura, segmentos, perfil, bevel weight) | `bevel.rs` | P0 | A |
| Loop Cut & Slide, seleção de edge loop / edge ring / boundary loop | `loops.rs` | P0 | A |
| Bridge Edge Loops (com twist e interpolação) | `bridge.rs` | P0 | A |
| Knife/Bisect (corte por plano; clear inner/outer) | `bisect.rs` | P1 | A |
| Spin / Screw (torno) | `spin.rs` | P1 | A |
| Merge (centro/primeiro/último/colapso) e Merge by Distance | `merge.rs` | P0 | A |
| Dissolve (vértices/arestas/faces) + Limited Dissolve | `dissolve.rs` | P0 | A |
| Delete (vértices/arestas/faces/só arestas-e-faces/loose) | `delete.rs` | P0 | A |
| Fill (n-gon), Triangle Fill, Grid Fill | `fill.rs` | P0 | A |
| Triangulate / Tris to Quads | `triangulate.rs` | P0 | A |
| Flip Normals / Recalculate Outside / Make Consistent | `flip.rs` | P0 | A |
| Smooth/Relax vértices (Laplaciano, HC), Shrink/Fatten, To Sphere, Flatten | `smooth.rs` | P0 | A |
| Translate/Rotate/Scale com pivô; **Proportional Editing** (smooth, sphere, root, sharp, linear, const, random; connected-only) | `transform.rs` | P0 | A |
| Subdivide arestas/faces, Poke, Rip, Separate/Split, Mark sharp/seam/crease/bevel-weight, Set smooth/flat | `misc.rs` | P1 | A |

## 3. Subdivisão e modificadores (`subdiv.rs`, `modifiers/`)

| Ferramenta Blender | Arquivo | Prioridade | Val. |
|---|---|---|---|
| Subdivision Surface (Catmull-Clark + creases + UV/atributos por corner) | `subdiv.rs` | P0 | A |
| Mirror (eixos, merge, clipping, mirror UV) | `modifiers/mirror.rs` | P0 | A |
| Solidify (espessura, offset, rim) — roupas, cartões | `modifiers/solidify.rs` | P0 | A |
| Array / Weld / Edge Split / Displace | `modifiers/{array,weld,edge_split,displace}.rs` | P1 | A |
| Decimate (colapso por erro quádrico) | `modifiers/decimate.rs` | P1 | A |
| Pilha de modificadores avaliável e serializável | `modifiers/stack.rs` | P0 | A |
| Bevel / Boolean / Shrinkwrap / Lattice / Armature | (fase 2: boolean via `boolmesh` atrás de trait) | P2 | A |

## 4. Escultura (`sculpt/`, `bvh.rs`)

Pincéis P0: Draw, Clay Strips, Clay, Smooth, Flatten, Fill, Scrape, Pinch, Inflate, Grab,
Snake Hook, Nudge, Crease, Layer, Mask (paint/smooth/sharpen/invert/clear), Elastic Grab.
Simetria X/Y/Z + radial; falloffs (smooth, sphere, root, sharp, linear, const, custom curve);
espaçamento/estabilização de traço; auto-mask (máscara, normal, face set, cavidade, topologia);
Dyntopo (subdividir/colapsar por detalhe); Voxel Remesh (SDF → surface nets). Val.: A (+H para sensação).

## 5. Normais, UV, curvas, atributos (diferencial anime)

| Ferramenta | Arquivo | Prioridade | Val. |
|---|---|---|---|
| **Transferência de normais** (esfera/proxy/direcional, "face shading proxy" à la Genshin/GGXrd), blend por peso, reset, align, average | `normals_tools.rs` | P0 | A+H |
| **Normais suavizadas de contorno** (para inverted hull sem frestas) | `normals_tools.rs` | P0 | A |
| Pintura de atributos de vértice NPR (AO, shadow shift, outline width, spec mask) | `attributes.rs` | P0 | A+H |
| Weight paint / transferência de pesos (roupa ← corpo) / auto-skin por envelope | `attributes.rs` | P1 | A |
| Shape keys ⇄ morph targets esparsos do core | `shapekeys.rs` | P1 | A |
| UV: costuras, unwrap (LSCM), pack, projeções (planar/caixa/cilindro/esfera), métricas | `uv/` | P0 | A |
| Curvas Bézier/Catmull → tubo, fita, **hair card** (taper, twist, perfil) | `curve.rs` | P0 | A |

## 6. Ferramentas de viewport do editor (em `anigo-studio`)

Gizmos de transformação, snapping (grade, vértice, aresta, face, incremento), seleção por
caixa/laço/pincel/clique-em-loop (Alt+clique), X-ray, wireframe overlay, normais overlay,
pivô (mediana, ativo, cursor 3D), orientação (global/local/normal), modo de simetria no editor,
hide/reveal, isolar. Val.: H (interação) — o motor expõe a matemática (picking, snapping) com testes.

## 7. Integração com o ProjectState (fase 2)

Malhas autorais viram `AssetEntry { kind: Mesh, uri: "anigo://authored/<id>" }` com payload
`EditMesh` serializado (JSON canônico + hash). Cada edição confirmada é um `Command`
(`ReplaceAuthoredMesh { asset_id, before_hash, after_hash }` + blob no log) — undo/redo e
autosave seguem o pipeline canônico. Ver `ARQUITETURA_NATIVA.md` §Integração.
