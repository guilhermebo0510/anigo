# Modelagem — auditoria de completude ("dá para modelar qualquer coisa?")

Data: 2026-10-13. Método: leitura do motor (`crates/anigo-modeling`), do painel (`anigo-studio`) e das ferramentas MCP
(`anigo_native_mesh_*`, `sculpt`, `modifiers`, `uv`, `normals`, `hair_*`, `skin_weights`). **Não** é um teste de modelagem de ponta a ponta com
um modelo real; isso fica para quando você enviar os modelos de teste.

**Resposta curta:** hoje dá para modelar **hard-surface e orgânico básico** (blocking, extrusões, loops, espelho, escultura, retopo manual
simples) e o fluxo anime (normais, UV, cabelo, pesos) é forte. Para modelar **um personagem a partir de referências**, ainda faltam peças que
todo modelador usa o tempo todo: imagens de referência no viewport, seleção por loop/anel, deslizar aresta/vértice, faca e encaixe em
superfície. A maior parte do que falta **já existe no motor** e só não chegou à interface.

## 1. O que existe (e funciona)

| Área | Estado |
|---|---|
| Primitivas | plano, grade, cubo, esfera quad, esfera UV, cilindro, cone, toro, cápsula, círculo |
| Modos | Objeto, Edição, Escultura, Cabelo; vértice/aresta/face; caixa, tudo/nada/inverter/mais/menos/ligados |
| Operadores de edição (35) | extrudar (região/individual), inset (região/individual), bevel, loop cut, bisect, deletar (5 variações), mesclar (centro/distância), dissolver (4), preencher buraco/grid fill, bridge, triangular/quadificar, inverter/recalcular normais, suavizar/relaxar, subdividir, poke, sharp/costura/vinco, esconder/revelar |
| Transformação | mover/girar/escalar com pivô, **edição proporcional** (7 falloffs), espelho de edição (motor), grade de encaixe |
| Modificadores (9, não destrutivos) | subsurf, mirror, solidify, array, weld, edge split, displace, decimate, triangulate; mover/ativar/aplicar |
| Escultura | 20 pincéis (draw, clay, clay strips, smooth, flatten, fill, scrape, pinch, inflate, grab, snake hook, nudge, crease, layer, elastic, 5 de máscara), simetria X/Y/Z + radial, falloffs, auto-máscara, dyntopo, voxel remesh (com sombreamento suave opcional) |
| Anime | normais por corner (sharp, ângulo, custom, transferir, média), UV completo (LSCM/ARAP, pack, projeções, costuras automáticas de personagem), cabelo por mechas, pesos de pele |
| Controle por MCP | todas as ações acima têm ferramenta (`mesh_op`, `mesh_select`, `mesh_transform`, `sculpt`, `modifiers`, `uv`, `normals`, `hair_*`, `skin_weights`) |

## 2. O que falta, em ordem de impacto para modelar personagens e assets

Legenda: **UI** = falta só na interface (o motor já tem) · **Motor** = falta de fato.

| # | Falta | Onde | Por que importa |
|---|---|---|---|
| 1 | **Imagens de referência no viewport** (frente/lado/costas, opacidade, escala, por vista ortogonal) | Motor + UI | É como se modela personagem a partir de turnaround. Você já mantém a pasta `imagens de referencia para modelo/` — hoje não há onde pô-las. **Maior lacuna.** |
| 2 | **Seleção por loop e por anel** (Alt+clique), selecionar similar, aleatório/xadrez, por característica (sharp, solto, não-manifold) | **UI** (`loops::edge_loop/edge_ring/face_loop` e `misc::select_*` existem) | Sem isso, selecionar um loop de edge é manual, vértice a vértice. |
| 3 | **Deslizar aresta/vértice** (edge slide, vertex slide) | Motor + UI | Ajustar o fluxo de loops sem estragar a forma. |
| 4 | **Faca** (corte livre), além do bisect por plano | Motor + UI | Cortar topologia nova em qualquer direção. |
| 5 | **Spin/Screw** (torno), **Rip**, **Separar/Dividir** malha, **Unir/Separar objetos**, definir origem, aplicar transformação | **UI** para spin/rip/separar (existem em `ops/spin.rs`, `ops/misc.rs`); **Motor+UI** para unir objetos/origem | Operações de todo dia (rodar um perfil, separar a cabeça do corpo, juntar peças). |
| 6 | **Encaixe em vértice/aresta/superfície na edição** + **Shrinkwrap** (retopologia) | **UI** para vértice/aresta (`snap_to_nearest_*` existem); **Motor** para shrinkwrap | Refazer a malha sobre uma escultura é o fluxo padrão de personagem. |
| 7 | **Shape keys na interface** (expressões, correções) | **UI** (`shapekeys.rs` existe) | Expressões faciais próprias; hoje só os morphs do Personagem. |
| 8 | Modificadores faltantes: **Boolean**, **Bevel**, **Shrinkwrap**, **Lattice**, **Curve**, **Remesh**, **Smooth/Corrective Smooth** | Motor | Boolean é a base do hard-surface; Lattice/Curve ajudam a deformar roupas e acessórios. |
| 9 | **Objetos de curva** (bezier/NURBS → malha, extrudar ao longo de caminho) | Motor (`curve.rs` existe, usado no cabelo) + UI | Cabos, cabelos longos, tubos, extrudar ao longo de caminho. |
| 10 | Escultura avançada: **pose**, **boundary**, **thumb**, **slide relax**, **cloth** | Motor | Refino; multires fica fora do escopo declarado do projeto. |
| 11 | Verificações de malha na UI (não-manifold, faces invertidas, interseção) com atalho de "selecionar problemas" | **UI** (`select_non_manifold` etc. existem) | Antes de exportar/animar. |

## 3. Recomendação de ordem

1. **Imagens de referência** (maior ganho imediato com os modelos que você vai enviar).
2. **Loop/anel + selecionar similar** (quase só interface).
3. **Spin, Rip, Separar, Unir objetos, definir origem** (interface + pouca coisa de motor).
4. **Deslizar + Faca**.
5. **Encaixe em vértice/aresta/superfície + Shrinkwrap**.
6. Shape keys na UI, Boolean, curvas.

## 4. Como validar quando você mandar os modelos

Para cada modelo de teste: (a) importar, (b) refazer um pedaço pela UI usando só as ferramentas acima, (c) anotar onde faltou
ferramenta. Tudo isso pode ser dirigido e medido pela bridge (`mesh_op`, `mesh_select`, `model_info include_geometry`), então cada lacuna
fica registrada com um comando reproduzível.
