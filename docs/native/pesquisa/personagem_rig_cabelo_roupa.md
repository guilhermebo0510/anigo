# ANIGO - Pesquisa: mesh base anime, rig, olhos, cabelo, roupa e física (out/2026)

Escopo: workspace **Personagem** (e ferramenta de cabelo no workspace **Modelagem**). Pesquisa feita na nuvem, sem GPU, sem ver as imagens do usuário (`C:\ANIGO\IMAGENS`). Nada do repo foi alterado além deste arquivo. O modelo `modelos/feminino/animebasemesh.glb` (CC-BY-4.0, Sketchfab, autor "Marcelo") foi só inspecionado em metadados (contagens e nomes); nada dele foi copiado.

Legenda de fonte:
- **[V]** doc/código oficial ou dado lido por mim nesta sessão (spec VRM, docs VRoid/Magica/Blender/Apple, crates.io, código do repo, glb).
- **[S]** secundário: blog, tutorial, resposta de fórum, página de terceiro lida nesta sessão.
- **[C]** conjectura ou conhecimento geral meu, **não verificado aqui**. Trate como hipótese a testar.

---

## 0. Resumo executivo

1. **Estratégia recomendada para o mesh base:** *malha única soldada* de corpo+cabeça (pescoço contínuo, sem emenda) + peças separadas só onde o render exige (olhos em camadas, brilho do olho, cílios, sobrancelhas, interior da boca, dentes, língua, cabelo, roupa). Topologia 100 % quad, simétrica em X, com loops obrigatórios nas articulações e no rosto. Orçamento alvo **10-14 k vértices / ~20-26 k triângulos** para o corpo+cabeça [C], mesma ordem do mesh de referência (um skin mesh de 9 104 vértices / 16 136 tris + 5 peças de olho/sobrancelha/cílio, total 11 050 vértices / 19 140 tris [V, glb]).
2. **Variação (magro/gordo/jovem/velho):** *morph targets esparsos para a superfície* (já é o desenho do catálogo: 157 sliders, 123 Morph / 22 BoneDelta / 12 Dual [V, `morph_catalog.rs`]) **+ re-ajuste dos ossos a partir da malha morfada** (ossos posicionados por "localizadores de junta" = combinação fixa de vértices). Pesos de skin ficam **constantes** porque a topologia é fixa. Escala de osso só para comprimento de membro, nunca para espessura (gera cisalhamento) [C, ver A7].
3. **Achado no repo, importante:** o catálogo de 157 sliders **não tem slider de idade** (`grep` por age/aging/wrinkle: nada) [V]. Idade ("jovem/velho") é macro obrigatório e hoje não existe.
4. **Achado no repo, importante:** `anigo-vrm` lê/escreve `VRMC_springBone` num formato **próprio** (`secondaryRig.groups[].joints[].spring_stiffness`, colliders com `spheres[]/capsules[]`) [V, `vrm1.rs:532-610`, `exporter.rs:585-611`] que **não é** o schema oficial (`specVersion`, `colliders[].shape.sphere|capsule`, `colliderGroups`, `springs[].joints[].{hitRadius, stiffness, gravityPower, gravityDir, dragForce}`, `center`) [V, spec]. Também o exporter coloca a extensão em `extensionsRequired`. Um VRM exportado hoje **não anima o cabelo em VSeeFace/three-vrm/UniVRM**. Corrigir isto vem **antes** de construir física em cima (ver F3, passo 0).
5. **Achado no repo:** paleta de skinning congelada em **24 ossos** (`array<mat4x4<f32>,24>`, 1 536 B no contrato) [V, `bone_sync.rs:8`]. VRM humanoid completo tem **55** ossos (15 obrigatórios + 40 opcionais, 30 deles dedos) [V, spec], e cabelo/saia/seios precisam de dezenas a centenas de ossos secundários. A paleta tem de virar um storage buffer de N ossos (decisão de contrato: ver F1.4).
6. **Física - o que implementar primeiro em Rust puro:** (a) **spring bone exatamente como na spec VRM** (Verlet, fórmula publicada) + colisores esfera/cápsula/plano/"inside", porque dá interoperabilidade imediata e é 100 % testável por unidade; (b) cadeia **XPBD** (distância + dobra + shape-matching/angle restoration) com ancoragem em vértices fixos, para cabelo/rabo/fitas; (c) **XPBD de malha-proxy** (distância, bending, tether, backstop, colisão com cápsulas do corpo) para saia/capa, com malha de render colada ao proxy por pesos. **Não depender** dos crates de cloth existentes (todos com < 100 downloads ou presos a Bevy/Rapier; ver E.5). Roupa em painéis+costura (estilo Marvelous/CLO) fica para fase posterior como *ferramenta de criação offline* (drapejar uma vez e congelar), não como simulação em tempo real.
7. **Cabelo estilo VRoid:** o modelo do VRoid é *grupos de cabelo → mechas → pontos de controle sobre uma grade-guia*, com mechas "freehand" (pincel sobre a grade) ou "procedurais" (contagem, seção transversal, torção), materiais base+highlight, e *bone groups* (1-30 grupos, 1-16 ossos por grupo, ponto fixo 0.01-0.99, stiffness 0-1, gravidade 0-1, hit radius 0-0.1) [V, FAQ VRoid]. O repo já tem `curve_to_ribbon`, `hair_card`, `hair_clump` em `anigo-modeling/curve.rs` [V]; falta a camada de **grupo + desenho sobre a cabeça + conversão em cadeias de osso**.
8. **O que a nuvem consegue com qualidade (honesto):** gerar uma **Gen-0 correta por contrato** (topologia, simetria, loops, ordem de vértices, grupos, pesos, ossos, morphs de teste) com verificação automática. **Não consegue** julgar nem produzir *beleza de rosto anime* nem gosto de proporção sem as referências. Proposta: a nuvem congela o **contrato de topologia** (hash da ordem de vértices/faces); o usuário esculpe só posições no Blender/ANIGO olhando as referências; o importador valida o contrato e tudo (morphs, pesos, roupa, física) continua valendo (F2).
9. **Decisão de produto pendente (não é técnica):** faixa do slider de idade (o macro "young" pode ser jovem-adulto estilizado ou incluir adolescente/criança). Bibliotecas de terceiros como OxiHuman impõem piso de 18 anos [V, SAFETY.md do repo deles]; MakeHuman tem alvos `baby/child/young/old` [V]. Deixar explícito no spec do macro; isto muda o conjunto de targets a autorar.

---

## 1. Estado atual do repo (lido, não alterado)

| Peça | O que existe | Lacuna para este escopo |
|---|---|---|
| `anigo-core/mesh.rs` | `create_canonical_base_unskinned(gender)`: **patches paramétricos** (cabeça 16x24, pescoço 6x16, tronco 20x24, pelve 14x24, membros por `add_surface_grid`), `Vec::with_capacity(4200)` [V]. Cada patch tem borda aberta; não é uma malha soldada. | É placeholder paramétrico: sem loops de rosto, sem mãos com dedos, sem orelhas, sem pálpebras, sem interior de boca. Não serve como base anime final. |
| `anigo-core/bone_sync.rs`, `skinning.rs` | 24 ossos (`Root, Hips, Pelvis, Spine, Chest, UpperChest, Neck, Head, Clavicle/Shoulder/Elbow/Wrist x2, Thigh/Knee/Ankle/Toes x2`), BOND (7 proporções), LBS 4 influências, `PROPORTIONS_BAKED_INTO_BASE_MESH = true` [V]. | Sem dedos, olhos, mandíbula, twist, secundários. Nomes não são os do VRM (`Shoulder` = braço superior; `Clavicle` = `shoulder` VRM). `Pelvis` filho de `Hips` pode colidir com a regra "pelve invertida obsoleta" da spec [V spec humanoid.md] - **checar posição** [C]. |
| `anigo-core/morph_catalog.rs`, `morph.rs`, `somatotype.rs` | 157 sliders em 18 zonas, mecanismos Morph/BoneDelta/Dual, deltas esparsos `{vertex_index, delta_position, delta_normal}`, somatotipo Heath-Carter (endo/meso/ecto) e gênero contínuo [V]. | Sem idade. Sem shapes de expressão (VRM/ARKit). Sem correctives por pose. |
| `anigo-vrm` | Parser/exportador VRM 1.0, MToon, node constraint, springBone (formato próprio, ver item 4 do resumo) [V]. | Corrigir schema do springBone; humanoid completo (dedos/olhos/mandíbula); expressions. |
| `anigo-modeling` | `EditMesh` n-gon, `ShapeKeySet` (captura, espelho, export para `SparseMorphSet`), `Curve`, `curve_to_tube/ribbon`, `hair_card` (taper, curl, flare, UV de atlas, **normais customizadas** por corner), `hair_clump` (guia -> fios), BVH, subdiv, normais [V]. | Falta: grupo de cabelo, desenho sobre a cabeça, conversão em ossos, mirror de grupo. |
| `anigo-cloth` | `ClothParticle{position, prev_position, acceleration, inv_mass}`, `DistanceConstraint{stiffness}`, `ClothMesh` - 27 linhas, **sem solver** [V]. | Tudo. Atenção: `stiffness` por iteração depende de iterações/dt; XPBD usa **compliance** (ver E.3). |
| `anigo-hair` | `HairStrand{control_points[position,width], segment_resolution}` - 27 linhas [V]. | Tudo. |
| `anigo-ik` | IK (não lido a fundo) | Integrar com retarget/pose. |
| Renderer | Cel shading, `anime_eye` (parallax + 2 highlights procedurais), SDF de rosto, hull, `hair_cloth` no grafo [V, `cel_shading_estado_da_arte.md`]. | Tangente de fio para angel ring; hair-through; sombra do cabelo no rosto (já listados no roadmap T-3.8/3.9). |

---

## A. Mesh base anime com variação

### A1. Topologia (quads, loops) - regras para o spec

Fontes: o que segue é prática padrão de modelagem de personagem para deformação [C - conhecimento geral, não verificado em fonte citável aqui], combinado com requisitos que o repo já impõe (morph esparso por índice de vértice, simetria, `EditMesh::validate`) [V].

**Regras globais**
- 100 % quads no corpo e na cabeça (triângulos só no couro cabeludo escondido pelo cabelo e, no máximo, no interior da orelha). Polos de 5 arestas posicionados em regiões que não dobram: lateral da bochecha, testa lateral, ponta do ombro (lado), lateral do quadril. Polos de 3 arestas perto de fendas (canto do olho, narina).
- **Simetria exata em X**: a linha central tem vértices em `x = 0` exato; mapa `mirror_index[i]` armazenado; é o que permite `ShapeKeySet::mirror_key` [V] e sliders L/R.
- Densidade uniforme (comprimento de aresta ~ constante por região), exceto aumento intencional no rosto.
- Ordem de vértices **estável e documentada** (contrato; ver F2): morph, pesos, grupos e roupa referenciam índices.

**Loops obrigatórios por região**

| Região | Loops/regras | Por quê |
|---|---|---|
| Olho | >= 3 anéis concêntricos em volta da órbita (pálpebra superior/inferior + cantos) + 1 anel de transição para a bochecha | Piscar (blendshape `blink`), squint, wide, sliders de formato/inclinação (tsurime/tareme) |
| Sobrancelha | 1 loop na crista superciliar (mesmo que a sobrancelha seja card) | `brow_ridge`, `browDown/InnerUp` |
| Nariz | loop nas asas e na ponta; sulco nasolabial contínuo | `noseSneer`, sliders de nariz anime (pequeno, quase plano) |
| Boca | >= 3 anéis (lábio, pele em volta, queixo) + loop interno que continua para dentro da boca (interior) | `jawOpen`, visemas `aa/ih/ou/ee/oh`, `mouthSmile/Frown/Pucker/Funnel` |
| Mandíbula/queixo | loop do ângulo da mandíbula + linha do queixo | slider V-line, `jawForward/Left/Right` |
| Orelha | hélice/anti-hélice/concha com loops próprios; ligação limpa com o crânio | orelhas élficas/animal, ear-to-head blend |
| Pescoço | >= 4 anéis; costura continua do crânio ao tórax (sem emenda) | torção da cabeça, normais contínuas no rosto |
| Ombro | 3-4 anéis em volta do deltoide + loop da clavícula + loop da axila | rotação do braço sem colapso |
| Cotovelo / joelho | 3 anéis: 1 sobre a junta + 1 de cada lado | dobra sem "candy wrapper" |
| Punho / tornozelo | >= 2 anéis | torção |
| Dedos | 3 anéis por falange (junta no meio), anéis do nó (knuckle), palma com 2 loops transversais | fechar a mão |
| Cintura / quadril | 1 loop na cintura mínima, 1 no umbigo, 2 na pelve, loop da virilha | sliders de cintura/quadril, saias e cintos prendem aqui |
| Pé | loop do calcanhar, do arco, da base dos dedos; dedos simples | pisar |

### A2. Contagem de polígonos (dados reais)

| Dado | Valor | Fonte |
|---|---|---|
| Mesh de referência (glb): skin mesh | 9 104 verts, 16 136 tris | [V] metadados glb |
| Mesh de referência: 4 camadas de olho / brilho / sobrancelha / cílios | 856+330+198+196 / 64 / 84 / 218 verts; 1472+384+256+192 / 112 / 160 / 428 tris | [V] metadados glb |
| Mesh de referência: total | 11 050 verts, 19 140 tris; **sem esqueleto, sem morph, sem animação**; 2 texturas PNG; materiais `doubleSided` | [V] metadados glb |
| VRoid com malhas transparentes deletadas vs. não deletadas | 55 914 vs 78 650 polígonos (exemplo de um autor); limite de upload do cluster citado: 72 000 | [S] nota de Melvina Sakura |
| VRoid: onde cortar polígonos | "cabelo tende a ter muitos vértices" - começar por ele | [V] FAQ VRoid "How to check the number of polygons" |
| Guilty Gear Xrd (referência de cel de alto nível) | ~40 k triângulos por personagem, sem normal map | [V] GDC 2015 Motomura, via `cel_shading_estado_da_arte.md` |
| "Acima de ~15 k triângulos já é muito" / modelo próprio de ~16 k | opinião de um usuário | [S] Reddit r/VRoid (fraco) |
| MMD / VTuber típico | faixa de dezenas de milhares de triângulos | [C] sem fonte verificada aqui; **não usar como número de projeto** |

Alvo proposto (ver F1): corpo+cabeça 10-14 k verts; extras (olhos, boca, cílios, sobrancelhas) ~2 k verts; cabelo e roupa fora do base e com orçamento próprio. Total de personagem completo em 25-45 k tris, dentro da faixa do VRoid e do Xrd.

### A3. Peças separáveis vs. malha única + slots de material

- **VRoid exporta peças separadas por função** (corpo, rosto, cabelo, roupa, acessórios) com materiais por peça; tem opção "deletar malhas transparentes" [V/S: FAQ e nota]. As formas de expressão ficam no mesh do rosto [S, lista `Fcl_*` abaixo].
- Vantagem de malha única soldada para corpo+cabeça: um único espaço de índices para morph/pesos; sem emenda de normais no pescoço; roupas e correctives se referem a um só mesh. Desvantagem: não dá para trocar a cabeça por outra - mas aqui a cabeça é morfável por sliders.
- **Peças que DEVEM ser separadas** (por render ou por visibilidade): olho (camadas), brilho do olho, cílios, sobrancelhas, interior da boca (dentes, língua, gengiva - escondidos por `Fcl_HA_Hide` no VRoid), cabelo (grupos), roupa (camadas), acessórios.
- Convenção: **slot de material = função de render** (skin_body, skin_face, mouth_inside, teeth, tongue, eye_white, eye_iris, eye_highlight, lash, brow, hair_*, cloth_*), pois o cel shading (limiar de sombra, contorno, SDF de rosto) é por slot [V, grafo do renderer].

### A4. Como o VRoid Studio estrutura corpo/rosto

Verificado [V - FAQ oficial]:
- Editores: **Face, Hairstyle, Body, Outfit, Accessories**; o Face Editor tem *Design* (ajusta formas de olhos/sobrancelhas etc. por sliders) e *Expression* (edita NEUTRAL/FUN/JOY... como conjunto - que vira `VRMBlendShapeProxy`) [V]. Body: sliders de altura, largura de ombro etc. (tudo abaixo do pescoço) [V].
- Foto: parâmetros base de expressão 0-100 para sobrancelhas, olhos, boca e **dentes** ("Teeth" muda a forma) [V].
- Exportação com contagem de polígonos visível; "Eye Alignment" habilita Look-at-camera/Sync Gaze [V].

Não verificado diretamente, mas amplamente citado [S] (lista de blendshapes do VRoid, 57 itens): `Fcl_ALL_{Neutral,Angry,Fun,Joy,Sorrow,Surprised}`, `Fcl_BRW_*` (sobrancelha), `Fcl_EYE_*` (Close, Close_L/R, Joy, Joy_L/R, Natural, Spread, Highlight_Hide, Iris_Hide...), `Fcl_HA_*` (dentes: Fung1/2/3, Short, Hide, cada um com `_Low/_Up`), `Fcl_MTH_*` (A, I, U, E, O, Close, Large, Small, Up, Down, SkinFung...). Lições para o ANIGO:
1. Expressões são **compostas de shapes regionais** (BRW+EYE+MTH+HA) e o `ALL_*` é o atalho; isso mapeia bem para `morphTargetBinds` de VRM com vários binds por expressão.
2. **Esconder partes por blendshape** (`Highlight_Hide`, `Iris_Hide`, `HA_Hide`): o brilho/íris/dentes são geometria separada que um shape colapsa. Em ANIGO pode ser máscara de material em vez de shape, mas o contrato VRM só tem morph/cor de material.
3. Dentes têm 3 formatos de presa/curto e variantes só do dente de cima/baixo.

### A5. Como o DesignDoll faz poses e proporções

[V - histórico de versões oficial, terawell.net]:
- **Pose**: controladores IK na tela (mão, pé, cintura, peito); duplo clique seleciona vários controladores; "cotovelo inverso" opcional; pose é preservada ao trocar de corpo.
- **Proporções por "Scale Tag"**: modo simples (dois controladores nas laterais do corpo: comprimento e espessura) e modo detalhado (por osso: comprimento, espessura na base e na ponta, **transição entre as duas por curva de Bézier**, zoom do corpo todo). Altura por valor ou por porcentagem; comprimento e espessura dos dedos; "body shape synthesis" (combina vários formatos); **Morphing Tag** separado (blend de formas).
- Conclusão: o DesignDoll é **esqueleto + deformação por osso com escala em dois pontos (base/ponta) + curva**, não malha de morphs. Para um app de *pose/referência* isso basta. Para o ANIGO (estilo anime com rosto e silhueta de autor) é insuficiente sozinho: use a ideia (escala por osso com perfil de transição) como **slider de BoneDelta/Dual do catálogo** (comprimento de membro, espessura de segmento com perfil), e morphs para volumes anatômicos.

### A6. MakeHuman / MB-Lab / Character Creator / Daz

- **MakeHuman** [V - docs técnicas oficiais]: *modifiers* controlam *targets*; um **macro modifier** controla uma variável (gênero, idade, raça, músculo, peso, proporção) e re-aplica uma **mistura de muitos targets cujo nome contém tokens** (`female/male`, `baby/child/young/old`, `minmuscle/averagemuscle/maxmuscle`, `minweight/averageweight/maxweight`, ...), com dependências automáticas entre macros (mudar idade re-mistura os targets de peso). Face/corpo têm modifiers locais. A UI de modelagem tem Gender, Age, Muscle, Weight, Height, Proportions + 3 etnias [V].
  - **Roupa/proxy**: cada vértice do proxy referencia **3 vértices do basemesh + 3 pesos baricêntricos + offset** (formato `.mhclo`), com escalas x/y/z; os **pesos de osso do proxy são derivados dos vértices do basemesh** salvo arquivo próprio `vertexboneweights_file` [V]. É exatamente o mecanismo certo para "roupa que acompanha os sliders" (F1.6).
  - **Licença**: código AGPL, **assets (basemesh, targets, skins) CC0** [V - página de licença e FAQ MPFB]. MPFB (addon Blender) é GPL-3.0 [S].
  - O MakeHuman é realista; usar como **modelo de arquitetura de macros**, não como fonte de forma anime.
- **OxiHuman** (crate Rust, Apache-2.0, dados CC0): 21 833 vértices, 38 targets CC0 (30 de cantos macro + 8 de medidas), export GLB/VRM, ~259 KB WASM [V, README]. Muito novo (v0.2.1), ~700 downloads, README com métricas de marketing (5 774 testes, 897 arquivos .rs) [V/C - maturidade **não confirmada**]. **Piso de 18 anos** nos dados [V, SAFETY.md]. Útil só como referência de organização de macros em Rust; não como base anime.
- **MB-Lab** (Blender, derivado do mesmo princípio de targets) [C - não pesquisado aqui].
- **Character Creator (Reallusion)**: CC5 e CC4 têm "Bone Adjustment" e "Morph slider creation" na lista de recursos [V, página oficial]; profile de *skin weights* e *expression profile* (ARKit) são parte do produto [C].
- **Daz Genesis**: shapes "Joint Controlled Morphs" (correctivos por ângulo de junta) e esqueleto que se adapta ao shape [C - conhecimento geral].

### A7. Blendshape vs. ossos para proporção; manter o rig em corpos magros/gordos/velhos

Estado do repo: cada slider já declara `SliderMechanism::{Morph, BoneDelta, Dual}` [V]; BOND tem 7 proporções que movem juntas [V]; hoje as proporções estão "assadas" na malha base (`PROPORTIONS_BAKED_INTO_BASE_MESH = true`), e a constante prevê virar `false` quando o rig dirigir as proporções [V].

Proposta [C, a validar com testes numéricos]:
1. **Superfície = morph esparso** (peso, músculo, idade, gênero, rosto). **Comprimento de segmento = BoneDelta**.
2. **Localizador de junta**: para cada osso humanoid, `joint_pos = Σ w_k · v_k` com 4-16 vértices fixos (centro da órbita, centro do cotovelo etc.), armazenado como "landmarks" no contrato. Depois de aplicar os morphs, recomputar a pose de repouso do esqueleto (posições; orientações por regras do padrão VRM: T-pose, eixos). É o princípio do "joint helper" do MakeHuman [C - não verificado aqui] e dispensa ajuste manual de ossos para cada slider.
3. **Skinning com bind morfado**: `M_skin = M_pose · inverse(M_bind_morphed)`. Como topologia e pesos não mudam, a malha morfada com o esqueleto re-ajustado deforma como a base. Isso precisa de `PROPORTIONS_BAKED... = false` e dos pesos separados do bake.
4. **Não usar escala não-uniforme de osso para espessura** (gera cisalhamento nas juntas girando) - usar morph. Escala uniforme de osso só em casos pontuais (mão/pé grandes) e nunca em ossos que carregam outros ossos com rotação [C].
5. **Corpos extremos (muito gordo/muito magro/velho)**: (a) reaplicar um passe de suavização Laplaciana dos pesos nas regiões afetadas (axila, virilha) se o delta passa de um limiar - testável como "pesos continuam normalizados e sem saltos"; (b) *shapes corretivos por junta* (cotovelo, joelho, ombro, quadril) escalados por macros (peso/músculo): valor = `corretivo(ângulo) · f(peso)`; (c) skinning por dual-quaternion ou "delta mush" opcional em juntas problemáticas [C]. O repo já tem `somatotype.rs` com interpolação baricêntrica; usar a mesma para escalar correctivos.
6. **Teste automático (sem olho)**: para cada macro nos extremos, aplicar uma pose padrão (cotovelo 90°, joelho 90°, ombro 120°) e medir: volume do cotovelo/joelho (colapso), distância mínima entre vértices de partes vizinhas, estiramento de arestas (< X %), autointerseção por BVH. Limiares são [C] e precisam do olho humano para calibrar.

### A8. Normais do rosto para cel

Já pesquisado no repo [V, `cel_shading_estado_da_arte.md`]: Guilty Gear Xrd editou normais à mão em toda feature; normal suavizada assada para o hull; transferência de normais de proxy (esfera/elipsoide) para a cabeça é item T-1.6/T-3.x. Para o base mesh isso significa:
- Máscara de vértice `mask_face_normal_proxy` (0..1) e proxy (elipsoide cabeça com centro/raios por landmark).
- **As normais proxy precisam ser recalculadas depois dos morphs** (o proxy segue os landmarks do rosto), por isso entram como função analítica `n = normalize(lerp(n_geom, normalize(p - c_proxy), mask))`, não como atributo assado. Para peças de morph que mexem muito na forma do rosto isto evita sombra suja.
- Caso `anigo-modeling/normals_tools.rs` já tenha a transferência (1977 linhas) [V arquivo existe, não lido a fundo]: reaproveitar.

### A9. Layout de UV convencional para texturas anime

- No glb de referência: 2 PNG (normal/oclusão), UV com `v` até 0.75 no mesh principal e peças pequenas usando faixa própria [V, accessors]. Sem convenção formal a extrair.
- Convenção prática [C]: *tile de rosto* grande e **sem espelhamento** (para pintar bochecha/marca assimétrica) com o resto do corpo espelhado; UV0 = pintura de pele; **UV1** = máscaras de sistema (SDF de sombra do rosto, normal suavizada de contorno, ID) porque os canais de vertex color já estão esgotados [V, doc do renderer item 7]. Olho: UV radial por camada (íris), cílios/sobrancelhas: UV de cartão (comprimento em V). Cabelo: V ao longo da mecha (raiz 0 -> ponta 1), igual `hair_card` do repo [V].
- Ilhas: cabeça-frente, orelhas, interior da boca, pescoço, tronco (frente/costas), braço (tubo), mão (palma/dorso), perna, pé. Manter costuras (seams) em locais cobertos: atrás da orelha, linha lateral do tronco, face interna do braço/perna. Usar xatlas (`anigo-native`) só para peças auto (roupa) [V repo]; o skin base tem UV autoral.

---

## B. Rig

### B1. Esqueleto padrão

- **VRM 1.0 humanoid** [V, spec `humanoid.md`]: **obrigatórios (15):** `hips, spine, head, left/rightUpperLeg, LowerLeg, Foot, left/rightUpperArm, LowerArm, Hand`. **Opcionais:** `chest, upperChest, neck, left/rightShoulder, left/rightToes, left/rightEye, jaw` e **30 ossos de dedos** (por mão: `ThumbMetacarpal/Proximal/Distal`, e `Proximal/Intermediate/Distal` para Index/Middle/Ring/Little). Total = 55. Pode haver nós não-humanoides entre ossos humanoides (útil para twist). "A pelve invertida abaixo de hips é obsoleta" [V].
- **Unity Humanoid/Mecanim** usa o mesmo conjunto de 55 [S/C]; é por isso que VRM/VRMA retargetam entre si.
- **Rigify** [V, manual Blender]: *metarig* (esqueleto de molde com tipos de sub-rig) -> botão **Generate** produz o rig completo (controles IK/FK, mecanismos, ossos de deformação); o metarig é mantido para regenerar. Convenção de nomes `ORG-`/`MCH-`/`DEF-` e camadas/coleções [C/S - memória, não lida aqui]. Para o ANIGO: só os **ossos de deformação** vão para o runtime/VRM; controles IK/FK são camada de `anigo-ik` (não exportada).
- **Proposta**: esqueleto de deformação = VRM humanoid completo (55) + secundários: twist (antebraço, braço, coxa, perna), seio L/R, cadeias de cabelo, cadeias de saia/capa/fita, orelhas/cauda (opcional). Nomes: VRM para humanoid, prefixos `sec_hair_*`, `sec_skirt_*`, `tw_*`, `bust_*`.

### B2. Rosto: ossos vs. blendshapes (VRM e ARKit)

- **VRM 1.0 expressions** [V, spec]: presets opcionais - emoções `happy, angry, sad, relaxed, surprised`; lábios `aa, ih, ou, ee, oh`; piscada `blink, blinkLeft, blinkRight`; olhar `lookUp, lookDown, lookLeft, lookRight`; `neutral`; mais expressões custom. Cada expressão tem `morphTargetBinds`, `materialColorBinds`, `textureTransformBinds`, `isBinary`, `overrideBlink/LookAt/Mouth` (`none|block|blend`). Olhar: LookAt por **ossos dos olhos** ou por expressão. A spec não define a forma da deformação.
- **ARKit** [V, doc Apple]: 52 coeficientes: `brow{Down,OuterUp}{L,R}`, `browInnerUp`, `cheekPuff`, `cheekSquint{L,R}`, `eyeBlink/LookDown/LookIn/LookOut/LookUp/Squint/Wide{L,R}`, `jawForward/Left/Open/Right`, `mouthClose/Dimple{L,R}/Frown{L,R}/Funnel/Left/LowerDown{L,R}/Press{L,R}/Pucker/Right/RollLower/RollUpper/ShrugLower/ShrugUpper/Smile{L,R}/Stretch{L,R}/UpperUp{L,R}`, `noseSneer{L,R}`, `tongueOut`. **Perfect Sync** = ter os 52 como expressões custom VRM com o mesmo nome (VSeeFace exige os 52 presentes, mesmo vazios) [V/S: VSeeFace, Warudo].
- **Decisão**: face por **blendshapes**, não por ossos (convenção VRM/ARKit/VTuber; o catálogo já é morph-first). Ossos faciais: só `leftEye`, `rightEye`, `jaw` (opcional, para boca muito aberta e dentes). Conjunto mínimo de shapes de expressão: os 15 presets VRM + 52 ARKit (os 52 geram os visemas e as emoções por combinação) = ~67 alvos esparsos, autorados *sobre* o rosto base (e correctivos de rosto são relativos à base; morph de identidade e de expressão **somam**, então o delta de expressão deve ser construído em espaço normalizado - ver A7/F1.5).
- Eye shape do slider (tsurime/tareme) **não pode** quebrar `blink`: o alvo `blink` deve ser definido como "fecha até a linha da pálpebra inferior" (offset em espaço local à pálpebra) para continuar fechando quando o olho cresce. Testável: com o slider de olho no máximo e no mínimo, `blink=1` deve ter abertura vertical < limiar [C].

### B3. Ossos auxiliares (twist, corretivos)

- **Twist por constraint VRM**: `VRMC_node_constraint` com **roll constraint** (copia parte da rotação em um eixo) / aim / rotation [V, spec]. Ordem de execução recomendada: humanoid -> lookAt -> expression -> **constraints** -> **springBone** [V, spec VRMC_vrm README]. Logo: twist de antebraço = osso `tw_forearm` filho de `leftLowerArm` com roll constraint 50 % do punho; funciona em todo player VRM.
- **Corretivos por junta**: morph esparso disparado pelo ângulo (ombro, cotovelo, joelho, quadril) - fora do padrão VRM; implementar no runtime ANIGO e **bake** (ou simplesmente omitir) no export VRM [C].

### B4. Skin weights

- **Autoridade**: como o mesh base tem topologia fixa, os pesos do corpo são **autorados uma vez** (Blender: automático + pintura) e **entregues com o mesh**. Não calcular pesos do corpo em runtime.
- **Auto-pesos (para roupa, acessórios e para a primeira passada do corpo)**:
  - *Bone heat* (Blender "Automatic Weights", Baran-Popović "Pinocchio" 2007) [C].
  - *Geodesic Voxel Binding* (Dionne & de Lasa 2013) - sem exigir malha estanque; ainda há uma versão para geometria degenerada [V - resultados de busca acadêmica].
  - **Transferência de pesos corpo -> roupa por "weight inpainting"** (Abdrashitov, Raichstat, Monsen, Hill; SIGGRAPH Asia 2023 Tech Comm): copia os pesos onde há alta confiança (vizinho próximo, normais compatíveis) e **resolve o resto por um problema de Laplaciano** (inpainting); código de referência C++ sob **MIT** [V - página do projeto e repo `rin-23/RobustSkinWeightsTransferCode`]. É o método certo para roupa.
  - Alternativa simplíssima, e já suficiente para MVP: o método do **MakeHuman**: cada vértice da roupa herda os pesos dos 3 vértices do corpo a que está preso por coordenadas baricêntricas [V].
- **Crates Rust**: busca em crates.io por "bone heat", "skinning weights", "automatic rigging", "geodesic", "biharmonic", "heat diffusion mesh": **nenhum crate dedicado e maduro** [V - busca feita nesta sessão]. Candidatos de apoio: `nalgebra-sparse 0.12.0` (Apache-2.0), `sprs 0.11.5` (MIT/Apache-2.0), `faer 0.24.4` (MIT) para resolver o sistema esparso Laplaciano; `halfedge 0.3.0` (MIT/Apache) e `smesh 0.2.7` (**MPL-2.0**) para malha de meia-aresta; `manifold_harmonics 0.2.1` (Apache-2.0, pouquíssimo uso). `russell_sparse` usa bibliotecas C (evitar). Em C++: libigl (MPL-2.0 [C]) tem Bounded Biharmonic Weights; Pinocchio [C - licença não verificada]. Conclusão: **implementar em Rust**: (1) transferência por vizinho mais próximo + (2) inpainting por Laplaciano com CG/Cholesky; testável por unidade (soma dos pesos = 1, suavidade, sem influência fora dos ossos vizinhos).

### B5. Retarget e animação

- **VRMA** (animação padrão do ecossistema VRM, glTF com `VRMC_vrm_animation`) - suportado por `bevy_vrm1 0.10.0` (MIT/Apache-2.0, ~7.4 k dl) e `vrm-runtime 0.1.0` (MIT/Apache-2.0, 78 dl, "VRMA, retarget humanoid, SpringBone, expressions, MToon") [V, crates.io + README]. Úteis como **oráculo de teste** do que `anigo-vrm` exporta e como referência de implementação do springBone (ler o código; não adicionar como dependência de produção sem avaliar maturidade).
- **BVH**: `bvh_anim 0.4.0` (MIT, 2019, 46 k dl) [V]. **FBX**: `ufbx` já integrado em `anigo-native` [V]. **MMD (PMX/VMD)**: `mmd-anim-*` 0.5.2 (MIT, família de crates ativa em 2026) [V]; se interessar importar poses MMD.
- Retarget: mapa por nome humanoid VRM -> rotação local relativa à T-pose de repouso (padrão VRMA). Como o ANIGO re-ajusta o esqueleto por morph (A7), o retarget tem de usar **a pose de repouso morfada** de cada personagem.

---

## C. Olhos anime

Base real [V, glb de referência]: o modelo de referência separa o olho em **4 malhas/camadas** (856, 330, 198, 196 vértices; materiais distintos por `baseColorFactor`: cinza claro de brilho alto = córnea/esclera, preto = pupila, verde = íris, verde escuro = anel da íris), um **EyeShine** separado (64 vértices, material emissivo - `KHR_materials_emissive_strength`), **sobrancelhas** (84 vértices) e **cílios** (218 vértices) como malhas à parte; só o skin tem tangentes e normal map. O renderer do repo já tem `anime_eye` (parallax + 2 highlights procedurais) [V].

Recomendações para o spec (A+C):
1. **Camadas do olho (por olho)**: (a) esclera/globo (esfera achatada ~ 200 verts, `eye_white`); (b) íris+pupila como **disco côncavo** na frente (50-120 verts; UV radial, a profundidade falsa/parallax vem do shader); (c) "eye shine"/highlight como **malha plana separada ligeiramente à frente** (32 verts por olho) para poder trocar a forma (círculo, estrela, duplo) e mover independentemente (no VRoid há `Highlight_Hide`/`Iris_Hide` [S]); (d) cílios como cards; (e) sobrancelha como card/malha com alfa.
2. **Tipos/estilos de olho** [C]: grandes redondos (kawaii), tsurime (olho alongado para cima), tareme (caído), olho de gato/fenda, olho semi-fechado, olho "só íris" (sem esclera visível), olho brilhante com múltiplos highlights, olhos assimétricos (heterocromia: dois materiais). Sliders do catálogo já cobrem escala, largura, abertura, espaçamento, inclinação cantal (`eye_canthal_tilt`), proporção da íris, dobra palpebral, formato da pupila [V].
3. **Pálpebra/cílios**: a pálpebra é parte do skin (anéis do olho); cílios em cards presos à borda da pálpebra (vertex groups `lash_upper`, `lash_lower`); fecham junto com `blink` via o mesmo morph (cílios são malha separada -> precisam de **shape próprio de blink**, igual ao skin). Se cílios forem colados ao skin como textura, só o skin precisa do shape (mais barato, menos expressivo).
4. **Olhar (LookAt)**: ossos `leftEye/rightEye` rotacionam o globo+íris (spec VRM, LookAt por bone) [V]; o shader faz parallax da íris dentro do disco.
5. **Interior da boca**: dentes superior/inferior (~300 verts cada), língua (~150), cavidade escurecida; morph `jawOpen`/visemas movem tudo junto; "Teeth" como parâmetro no VRoid indica que dentes têm variantes de forma [V]. Esconder/mostrar dentes por material (cel: sem sombreamento, só cor).
6. **Sobrancelhas**: cards com alfa, morphs `brow*` do ARKit/VRM; "hair-through" (cabelo translúcido sobre olho/sobrancelha) via stencil ou máscara é decisão pendente do renderer [V, doc cel].

---

## D. Cabelo

### D1. A ferramenta de cabelo do VRoid (verificado)

Fonte: [V] FAQ oficial "How do I customize hairstyles?" e "I want to edit hair bounce".
- **Hierarquia**: *Hairstyle item -> Hair Groups -> Hairs (mechas)*. Ações no menu de grupo: renomear, visibilidade, **Smoothing** (reduz pontos de controle, só freehand), duplicar, **Flip** (espelha grupo no eixo central do modelo; para mecha individual, simetria pela guia), deletar, **Convert to Freehand** (procedural -> freehand).
- **Dois modos de criar**: (1) **Freehand Group**: aparece uma **grade-guia** (guidelines) sobre a cabeça; o usuário pinta *chunks* de cabelo com o pincel ao longo da grade; pontos verdes deformam a guia; "Retouch Brush" refaz a forma de um grupo seguindo novos traços; **Control Point** ajusta comprimento/ponto de fixação; "Add Freehand Hair Guides" cria nova grade. (2) **Procedural Group** ("Add Procedural Hair Guides"): parâmetros de **contagem de mechas, seção transversal, torção (twist)** etc.
- **Parâmetros por grupo** (sliders): comprimento, espessura, etc.
- **Materiais**: cor e textura por grupo; editor de textura com **base + highlight** (equilíbrio entre os dois dá volume) [V].
- **Hair Bounce** (física): o usuário agrupa mechas em **Bone Groups**; "Auto-generate Bone Group" (1-30 grupos) ou manual; cada grupo tem um **eixo** e parâmetros: **Bones 1-16**, **Fixed Point 0.01-0.99** (onde o primeiro osso começa; antes dele o cabelo é rígido), **Stiffness 0-1**, **Gravity 0-1**, **Hit Radius 0-0.1** (colisor que evita atravessar rosto/corpo). "Poucos ossos = movimento mecânico; muitos = fluido, mas custa CPU". **A direção dos ossos segue a direção do traço** em que a mecha foi desenhada. Editar o penteado depois **não** atualiza os ossos já criados (é preciso "Move Axis to Group Center") [V].
- Contagem de polígonos: o cabelo é a maior fonte de vértices; há artigo oficial para reduzi-los [V].

### D2. Alternativas

- Blender: *Hair Tool* (addon pago/comunidade que gera cards a partir de curvas) e curvas->mesh; cards texturizados em atlas [S - blog, não confirmado].
- Maya XGen cards, Unreal Groom/hair cards [C]: fluxo igual - guias -> cards com UV de atlas.
- **Em tempo real, profissional**: TressFX (AMD, GPUOpen) simula tiras de cabelo na GPU; versão 4.1 menciona "Velocity Shock Propagation" e "Local Shape Constraints" simplificadas [V, GPUOpen]. Princípio: restrições de comprimento + shape local/global; não é o caminho para mechas anime em cards.
- **Para anime**: cards/tiras com poucas cadeias de ossos (VRoid) ou "BoneCloth/BoneSpring" (Magica Cloth 2) [V].

### D3. Sombreamento anime do cabelo

Já levantado em `cel_shading_estado_da_arte.md` [V]: tangente atual do cabelo = `cross(N, up)` (errada, faz o angel ring oscilar); correção = **tangente do fio** vinda do `anigo-hair`/`hair_card` (a curva já tem frames de transporte paralelo [V, `curve.rs`]) + deslocamento vertical em espaço de vista; hair-through (cabelo translúcido sobre o olho); sombra do cabelo no rosto. Para a nova ferramenta: **exportar `TANGENT` por mecha** (direção raiz->ponta) como atributo; a spec VRM diz que TANGENT pode ser ignorado e recalculado por MikkTSpace [V], então a tangente-de-fio deve ir em outro atributo (`_ANIGO_STRAND_DIR`) ou UV1, não em TANGENT.

### D4. Física do cabelo (VRM springBone, verificado)

Algoritmo oficial (não normativo, mas é o de UniVRM/three-vrm) [V, `VRMC_springBone-1.0/README.md`]:
```
inertia   = (currentTail - prevTail) * (1 - dragForce)
stiffness = dt * parentWorldRot * initialLocalRot * boneAxis * stiffnessForce
external  = dt * gravityDir * gravityPower
nextTail  = currentTail + inertia + stiffness + external
nextTail  = worldPos + normalize(nextTail - worldPos) * boneLength   // mantém comprimento
// colisão: para cada collider, se distance < 0 -> nextTail -= direction*distance; renormaliza o comprimento
// aplica rotação: node.rotation = initialLocalRot * fromTo(boneAxis, to)
```
- Parâmetros por junta: `hitRadius` (m), `stiffness`, `gravityPower`, `gravityDir`, `dragForce [0,1]`. Cada `spring` = cadeia de `joints`; `colliderGroups`; `center` (espaço de inércia, evita balançar com o movimento do personagem). Ordem: da raiz para os descendentes. Cadeias sem ramificação (ramificação é indefinida). O último joint é só "tail" (adicione um nó vazio se quiser rotacionar o último) [V].
- Colliders: `sphere{offset,radius}`, `capsule{offset,radius,tail}`; extensão `VRMC_springBone_extended_collider` adiciona `inside` (esfera/cápsula "por dentro", para manter o cabelo dentro) e `plane` [V, spec].
- Faixas do VRoid (UI): ossos 1-16, stiffness 0-1, gravidade 0-1, hit radius 0-0.1 [V]. Valores "típicos" de dragForce/stiffness para cabelo (ex. 0.4-0.6 / 0.5-1.0) [C - não verificado; calibrar por olho].
- **Cabelo em mechas por cadeia de ossos**: cada mecha (ou grupo de mechas) ganha uma cadeia de N ossos ao longo da curva-guia; os vértices do card pesam nos 2 ossos vizinhos do parâmetro `t` (interpolação linear), com raiz rígida até o Fixed Point. Melhor para anime: poucas cadeias grossas.

---

## E. Roupa e física

### E1. Como os líderes fazem [S/C - memória; só Magica Cloth 2 foi lido em detalhe]

| Abordagem | Quem | Ideia | Custo |
|---|---|---|---|
| Painéis 2D + costura + simulação | Marvelous Designer, CLO3D [C] | Padrões 2D, pontos de costura, simulação FEM/PBD de alta resolução em CPU/GPU, depois exporta malha ou cache | Offline/semi-interativo, caro |
| Blender Cloth | Blender [C] | Massa-mola/bending sobre malha, com pin groups; "bake" | Offline |
| Unreal Chaos Cloth | Epic [C] | Malha de simulação (proxy) + "Max distance" pintada, cápsulas de colisão do Physics Asset, resultado blendado no mesh de render | Tempo real |
| **Magica Cloth 2** | Unity asset | **BoneCloth** (cadeias de Transform), **BoneSpring**, **MeshCloth** (proxy mesh), "baseline" + "depth" (0..1), parâmetros: Force (gravidade/arrasto), **Angle Restoration**, Angle Limit, **Shape Restoration** (Distance, **Tether**, Triangle Bending), Inertia (mundo/local), **Movement Limit/Backstop**, Collider Collision, Self Collision (beta) [V] | Tempo real, Job+Burst |
| VRM springBone | VRM | Cadeias de osso com Verlet, colisores esfera/cápsula | Muito barato |

Pontos do Magica Cloth 2 que valem copiar [V]: (1) **proxy mesh simulado** separado da malha de render; (2) **vértices fixos/móveis** pintados, com *depth* normalizado da raiz à ponta e **curvas de parâmetro por depth**; (3) **Backstop/Movement Limit**: vértice só se afasta até `MaxDistance` e não entra além do plano da normal do proxy - evita atravessar o corpo *sem* colisores; (4) *Tether compression* >= 0.8 recomendado; (5) auto-colisão custa muito e é recurso beta/desktop.

### E2. Qual abordagem é prática no ANIGO

1. **Roupa = malha com pesos + spring bones (VRM-compatível)**: camisa, calça, luvas, botas, cabelo curto. Zero custo extra, exporta em VRM, funciona em qualquer player. Saia/capa soltas com spring bones são o ponto fraco: colunas de ossos sem restrição entre colunas -> a saia abre e penetra as pernas.
2. **PBD/XPBD em CPU (proxy)**: saia, capa, mangas largas, fitas/laços grandes. Resultado não exporta em VRM (padrão não tem cloth de malha); no export VRM, *converter* para cadeias de ossos ("BoneCloth"), perdendo restrições laterais.
3. **Painéis + costura**: ferramenta de *criação* (roupa fácil de fazer), não de runtime: desenhar painel 2D -> triangular -> costurar (pares de arestas) -> drapejar sobre o corpo em T-pose com XPBD offline -> congelar a malha ajustada ao corpo e ligar os pesos. Fase posterior.
**Recomendação: 1 e 2 em paralelo (mesmo núcleo de solver), 3 depois.**

### E3. Algoritmos

- **XPBD** (Macklin, Müller, Chentanez, MIG 2016; "Small Steps in Physics Simulation", SCA 2019) [V - referências citadas em artigos arXiv encontrados; fórmulas de memória]:
  - Restrição `C(x)=0`, compliance `α` (inverso de rigidez, unidade física), `α~ = α/Δt²`.
  - `Δλ = (-C - α~·λ) / (∇C·M⁻¹·∇Cᵀ + α~)`; `Δx = M⁻¹·∇Cᵀ·Δλ`. `λ` zera no início de cada passo.
  - Com *substeps* (n pequenos passos, 1 iteração cada) a rigidez deixa de depender do número de iterações. Consequência: a API do repo (`DistanceConstraint.stiffness`) deve ser **compliance**.
- **Restrições**: distância (aresta), flexão (dobra entre dois triângulos adjacentes - Müller 2007 - ou distância entre vértices opostos, mais barata) [S], *tether* (distância máxima/mín à raiz), *shape/angle restoration* (puxa de volta à pose skinada: é o que dá o "Angle Restoration" do Magica e o `stiffness` do VRM) [V/C], volume (balão) opcional.
- **Colisão com o corpo**: cápsulas/esferas ligadas aos ossos (derivadas do esqueleto morfado e escaladas pelos sliders; mesmo conjunto que alimenta o springBone) - teste esfera-ponto e cápsula-ponto são fórmulas fechadas [V, pseudocódigo VRM]. Para pernas/tronco de formas estranhas: SDF do corpo (campo 3D coarse atualizado por pose) - crate `mesh_to_sdf 0.4.0` (MIT/Apache, 13 k dl) calcula SDF de malha estática [V]; para corpo animado, preferir cápsulas + backstop.
- **Auto-colisão**: hash espacial uniforme + restrição de distância mínima entre partículas; custo O(n) com bom hash, mas muda a estabilidade; **deixar para depois** (Magica também marca como beta) [V].
- **Estabilidade**: substeps 4-10 por frame a 60 Hz, 1 iteração, damping de velocidade, limitar deslocamento por substep, reiniciar ao teletransportar (Magica faz "reset on teleport") [V]; dt fixo para determinismo.
- **Custo (estimativa [C])**: saia 24 colunas x 12 anéis = 288 partículas / ~1 100 restrições; 10 substeps = ~11 k projeções/frame, bem abaixo de 0,5 ms em uma thread. 50 cadeias de cabelo x 8 juntas = 400 partículas, trivial. O gargalo real será skinning da malha de render, não a simulação.

### E4. Roupa "fácil de criar" (criação)

1. **Ajuste ao corpo por vínculo baricêntrico** (modelo MakeHuman `.mhclo`: 3 vértices do corpo + 3 pesos + offset por vértice da roupa) [V]: a roupa acompanha todos os sliders de corpo sem retrabalho, e **herda pesos de skin** do corpo [V].
2. **Gerar por região do corpo**: selecionar faces do corpo (grupos `region_torso`, `region_legs`...), duplicar, deslocar pela normal (espessura) - `Solidify` já está no roadmap do `anigo-modeling` [V]; cortes de bainha por curva.
3. **Moldes paramétricos**: camisa, saia (N painéis + curva de bainha), calça, vestido, jaqueta; saia como cone/malha em anéis com a **cintura presa** a anel de vértices do corpo.
4. **Máscara de corpo sob a roupa**: `mask_hide_under_cloth_*` zera vértices do skin escondidos (evita z-fight e reduz custo).
5. **Proxy de simulação** gerado automaticamente por decimação do pedaço "solto" (meshopt `simplify` já em `anigo-native` [V]) + pintura de vértices fixos/móveis (depth) + backstop.

### E5. Crates Rust existentes (consultados no crates.io em 12/out/2026)

| Crate | Versão | Licença | Downloads | Observação |
|---|---|---|---|---|
| `rapier3d` | 0.36.1 | Apache-2.0 | 1.8 M | Física de corpos rígidos; não é cloth/hair. Útil se precisar de colisão geral. |
| `parry3d` | 0.31.1 | Apache-2.0 | 2.8 M | Colisão/consultas geométricas (cápsula, esfera, trimesh, BVH). Candidato para consultas, mas cápsula/esfera-ponto são triviais de escrever. |
| `avian3d` | 0.7.0 | MIT/Apache-2.0 | 398 k | Engine XPBD para Bevy (ECS) - amarrado ao Bevy. `bevy_xpbd_3d` deprecado. |
| `rapier-cloth` / `rapier-cloth-core` | 0.3.0 | MIT | 24 / 46 | XPBD CPU engine-independent (core) + integração Rapier 3D; README admite "performance em tempo real ainda em desenvolvimento" [V]. Útil só para **ler**; `rapier3d 0.34` pinado na doc [V]. |
| `bevy_softbody` | 0.1.0 | MIT | 29 | XPBD de cloth na GPU (wgpu) para Bevy 0.19. Referência. |
| `bevy_silk` | 0.10.0 | MIT | 12.8 k | Cloth para Bevy (massa-mola). |
| `oxihuman-physics`, `oxiphysics-softbody` | 0.2.1 / 0.1.2 | Apache-2.0 | 519 / 263 | Soft body/colisão cápsula; muito novos, qualidade não confirmada [C]. |
| `bevy_skeleton_modifiers` | 0.1.1 | MIT | 20 | Spring bones + IK derivados do Godot para Bevy. |
| `vrm-runtime` | 0.1.0 | MIT/Apache-2.0 | 78 | Loader+runtime VRM 1.0 com SpringBone, expressions, LookAt, VRMA: **melhor referência de comportamento** para testar `anigo-vrm`. |
| `bevy_vrm1` | 0.10.0 | MIT/Apache-2.0 | 7.4 k | VRM+VRMA no Bevy. |
| `vrm-spec`, `serde_vrm` | 0.1.1 / 0.4.0 | Apache-2.0 / MIT-Apache | 6.4 k / 7.7 k | Tipos serde do VRM (o `vrm-spec` é da pixiv): comparar com o parser próprio. |
| `mesh_to_sdf` | 0.4.0 | MIT/Apache-2.0 | 13.7 k | SDF de malha estática. |
| `bvh` / `obvhs` | 0.13.0 / 0.4.0 | MIT / MIT-Apache | 1.65 M / 186 k | BVH. O `anigo-modeling` já tem `bvh.rs` próprio [V]. |
| `meshopt` | 0.6.2 | MIT/Apache-2.0 | 603 k | Já integrado via `anigo-native` [V]. |
| `glam` | 0.34.1 (workspace usa 0.29) | MIT/Apache-2.0 | 159 M | Alinhar versão antes de adicionar crates que dependam de glam novo [V]. |

**Veredito**: nenhum crate de cloth/hair é adequado como dependência de produção. Escrever `anigo-physics` próprio (XPBD pequeno e determinístico, ~1-2 k linhas) e usar `vrm-runtime`/`bevy_vrm1` apenas como **oráculo** em testes de integração (lendo VRM exportado e comparando trajetórias do spring bone com tolerância) - a avaliar se compensa a dependência de dev.

### E6. O que implementar primeiro em Rust puro

Ver F3 (ordem completa). Resumo: spring bone VRM-exato -> colisores -> cadeia XPBD -> malha-proxy XPBD com backstop -> auto-colisão -> painéis/costura.

---

## F. Proposta concreta para o ANIGO

### F1. Especificação do mesh base (v0 do contrato)

**F1.1 Peças e orçamento** (alvos [C], a calibrar contra o mesh de referência e contra as imagens do usuário)

| Peça | Verts | Notas |
|---|---|---|
| `Skin` (corpo+cabeça soldados) | 10 000-14 000 | Cabeça 3 000-4 000; pescoço 300; tronco 2 000; braços 2x800; **mãos 2x1 200 (5 dedos)**; pernas 2 000; pés 2x400. Quad puro. Aberturas: boca (interior fecha em `MouthInside`), olhos (aberturas para os globos), base do pescoço contínua. |
| `EyeWhite` x2 | ~200 cada | esfera achatada |
| `EyeIris` x2 | 50-120 cada | disco côncavo, UV radial |
| `EyeHighlight` x2 | 16-32 cada | plano à frente, forma trocável |
| `Lashes` x2 | ~100 cada | cards, preso a vertex group do skin |
| `Brows` x2 | ~40 cada | cards com alfa |
| `MouthInside` | ~600 | dentes sup/inf, língua, gengiva |
| Cabelo | fora do base | 3-8 k tris típico para anime [C]; ferramenta D |
| Roupa | fora do base | ver E |

Dois meshes **isomórficos** (masculino/feminino): mesma contagem, mesma conectividade, mesmos UV, mesmos índices de junta - o repo já testa isso [V, mesh.rs ~1372-1376]. Gênero = interpolação contínua.

**F1.2 Loops obrigatórios**: ver A1 (tabela). Verificáveis automaticamente: `loop_id` por região no contrato; teste confere que cada anel obrigatório existe como ciclo fechado de arestas e tem o número mínimo de vértices.

**F1.3 Grupos de vértice (nomes)**
- Deform (pesos): nomes VRM humanoid (`hips`, `spine`, `chest`, `upperChest`, `neck`, `head`, `leftEye`..., `jaw`, `leftShoulder`..., dedos `leftIndexProximal` etc.) + `tw_*` (twist) + `bust_L/R`.
- Zona anatômica (18, iguais ao enum `AnatomicalZone`): `zone_global_silhouette`, `zone_craniofacial`, `zone_eyes`, `zone_eyebrows`, `zone_nose`, `zone_mouth_lips`, `zone_jaw_chin`, `zone_ears`, `zone_neck_trapezius`, `zone_shoulders_clavicles`, `zone_chest_pectorals`, `zone_bust_female`, `zone_abdomen_waist`, `zone_pelvis_hips`, `zone_gluteus`, `zone_upper_limbs`, `zone_hands_fingers`, `zone_lower_limbs` [V, enum do repo]. Servem para pintar/zerar influência de sliders e para a UI.
- Máscaras: `mask_face_normal_proxy`, `mask_outline_weight`, `mask_shadow_threshold_offset` (substitui canais de vertex color), `mask_hide_under_cloth_{torso,arms,legs,feet,hands}`, `mask_skin_sss`, `lash_upper/lower`, `hair_scalp_region` (onde nasce o cabelo), `cloth_attach_{waist,shoulder,neck,wrist,ankle}`.
- Landmarks (ordem fixa, vértices com peso baricêntrico) para localizador de juntas (A7) e para cabelo/roupa: `lm_eye_center_L/R`, `lm_nose_tip`, `lm_mouth_corner_L/R`, `lm_chin`, `lm_ear_top/lobe`, `lm_head_top`, `lm_neck_base`, `lm_shoulder`, `lm_elbow_outer`, `lm_wrist`, `lm_hip`, `lm_knee`, `lm_ankle`, `lm_waist`, ...

**F1.4 Esqueleto e paleta**
- 55 humanoid VRM + twist (4 por lado) + `bust_L/R` + secundários dinâmicos (cabelo/saia: gerados pelas ferramentas).
- Hierarquia exatamente a da spec (parents fixos) [V]. Mapear `Root`/`Pelvis` do canônico para: `Root` fora do humanoid; `Pelvis` fica como nó não-humanoid **ou** é removido, para evitar o osso "pelve invertida" [V/C].
- **Contrato do renderer**: trocar `array<mat4x4,24>` por *storage buffer* de N matrizes (N <= 256 por personagem para MVP; 4 influências por vértice continuam, `JOINTS_0` u16 já aceita). Isto altera `render_contract_v1.json`; o layout de vértice de 72 B não precisa mudar (já tem `joints` u16x4 e 4 pesos), só a paleta cresce. **Decisão do usuário**, porque o contrato está congelado [V].

**F1.5 Morph targets mínimos**
- **Macros (a autorar; deltas esparsos aditivos por gênero + acoplamentos)**:
  - `age` (3 âncoras: jovem estilizado / adulto / idoso) - **não existe no catálogo**; inclui forma de rosto (bochechas, queixo, olheiras/rugas como normal/máscara, pescoço), postura (curvatura da coluna - existe `spine_s_curvature` [V]), volume de seios/glúteos, proporção cabeça/corpo. Ver decisão de faixa etária (resumo, item 9).
  - `weight` = `somatotype_endomorph`, `muscle` = `somatotype_mesomorph`, magreza = `somatotype_ectomorph` [V, já existem]; `height_overall`, `head_to_body_ratio`, `torso_to_limb_ratio` como BoneDelta [V].
  - `gender` contínuo (existente `interpolate_canonical_mesh` [V]).
  - Estratégia MakeHuman: o resultado final = soma de targets de macro com **pesos produto** (ex.: `gender x age x weight x muscle`) apenas para o conjunto reduzido de combinações que realmente mudam a forma; o resto aditivo [V como princípio - tokens nos nomes de target].
- **Rosto (já no catálogo, zonas 2-8)**: olhos (escala, largura, abertura, espaçamento, posição vertical, profundidade orbital, inclinação cantal, íris, dobra palpebral, aegyo-sal, curvatura dos cantos, pupila), sobrancelhas (6), nariz (8), boca/lábios (8), mandíbula/queixo (10), orelhas (5), craniofacial (11) [V].
- **A adicionar**: (1) 15 presets VRM + 52 ARKit (~67 alvos) como *expressões*; (2) visemas via combinação (`aa ih ou ee oh` = ARKit `jawOpen/mouthFunnel/mouthPucker/mouthSmile...`) ou alvos diretos; (3) blink relativo à pálpebra (B2); (4) correctivos de junta; (5) variantes de dente (curto, presa) e de orelha (humana, élfica, gato); (6) estilo de olho (alvos "preset": redondo, tsurime, tareme, gato).
- **Normalização**: shapes de expressão e de identidade somam; para evitar conflito quando a identidade muda muito (olho grande), definir expressões como delta *proporcional* à geometria local (sobre landmarks), ou limitar o range dos sliders [C].
- **Formato**: `SparseMorphSet` (já existe) + mapa nome->índice no glTF `extras.targetNames` (recomendação da spec VRM) [V]; sem tangentes de morph [V].

**F1.6 Roupa**: arquivo de roupa = malha + vínculo baricêntrico ao corpo (3 verts + pesos + offset) + pesos herdados + proxy de simulação + grupos fixo/móvel/depth + parâmetros físicos (E.1) + lista de colisores [V pelo modelo MakeHuman].

### F2. Pipeline de criação - o que a nuvem pode e o que fica local

**O que a nuvem consegue fazer com qualidade (verificável sem olhos)**

| Item | Viável? | Qualidade | Verificação automática |
|---|---|---|---|
| Esqueleto VRM completo (55 + extras), hierarquia, nomes, T-pose | Sim | Alta | Testes de contrato (parents, simetria, comprimentos) |
| Tronco/membros por **seções transversais** (loft entre elipses/curvas por altura, vértices alinhados em anéis -> quads limpos) | Sim | Boa (formas limpas, "manequim anatômico") | Manifold, Euler, valência, simetria, razão de 7-8 cabeças |
| Mãos (palma + dedos por tubos de 3 falanges + web) | Sim, mas | Média (funcional e riggável; estética precisa de retoque) | Loops por falange, pesos por dedo |
| Pés | Sim | Média | idem |
| Cabeça "neutra" (crânio + loops de olho/boca/nariz/orelha por regras) | Parcial | **Baixa a média**: topologia correta, mas rosto sem charme anime | Loops obrigatórios, simetria |
| Rosto anime com beleza/identidade, proporção de olho/queixo, orelha "bonita" | **Não** sem referências | - | Só métricas objetivas (simetria, curvatura contínua, razões numéricas) - não julgam estilo |
| Pesos iniciais de skin (transferência/auto) | Sim | Boa no corpo; revisar ombro/quadril | soma=1, suavidade, sem vazamento entre braços/tronco |
| Morphs de macro (peso/músculo/gênero/idade) como deformações paramétricas (escalas locais ao longo de seções, ruído suave) | Sim, "primeira aproximação" | Média | Deltas limitados, sem inversão de normal, sem autointerseção |
| Morphs de expressão (blink, jawOpen etc.) por deformação de landmarks | Parcial | Média | boca/olho fecham; ruim para cantos/lábios sem arte |
| Hair tool, física, roupa-molde, contrato, testes | Sim | Alta | muitos A |

**Proposta de pipeline em 3 fases**
1. **Nuvem - Gen-0 por contrato**: gerador procedural determinístico -> `Skin_v0` (corpo+cabeça), peças auxiliares, grupos, landmarks, esqueleto, pesos iniciais, morphs de macro "aproximados", **hash do contrato de topologia** (contagem, conectividade de faces em ordem, UV, índices de grupos). Exporta glb + `topology_contract.json`. Substitui o placeholder de 4 patches.
2. **Local (usuário + Blender/ANIGO Modelagem, com as imagens)**: importar Gen-0; esculpir/mover **somente posições** (proportional editing, sculpt - já no roadmap) para obter o rosto e o corpo anime desejados; salvar como "basis atualizada". O importador `anigo-modeling` recusa mudanças de topologia (verifica o hash) ou oferece "rebind" (transferência por vizinho mais próximo para o contrato). Opcional: Blender com addon que só exporta posições na ordem do contrato.
3. **Nuvem/CI**: com a nova basis, **re-derivar** o que depende da forma: landmarks, localizadores de juntas, pesos refinados (inpainting), morphs de macro por *retarget de deltas* (aplicar deltas da Gen-0 em espaço local), testes de qualidade. O que depende de gosto (sliders de olho, rosto) o usuário ajusta olhando.

**Importante**: o `animebasemesh.glb` é CC-BY e "não redistribuir": não usar como fonte de geometria; no máximo como referência local do usuário no Blender. Dados CC0 (MakeHuman/OxiHuman) são utilizáveis legalmente, mas são realistas e de topologia diferente; só como referência de proporção/organização.

### F3. Ordem de implementação da física (crates)

Estrutura proposta:
- **`anigo-physics` (novo)**: solver puro (sem dependência de GPU/Bevy): `SpringChain` (VRM-exato), `Collider{Sphere,Capsule,Plane,Inside}`, `ColliderGroup`, `XpbdWorld` (partículas, restrições com compliance, substeps), `Proxy`.
- **`anigo-hair`**: geometria e autoria (grupos, guias, cards, atlas, conversão guia->cadeia), usa `anigo-physics` para simular.
- **`anigo-cloth`**: autoria (moldes, vínculo ao corpo, proxy, pintura fixo/móvel, backstop), usa `anigo-physics`.
- **`anigo-vrm`**: (passo 0) schema oficial do springBone; mapeamento `SpringChain` <-> `VRMC_springBone`.

| # | Passo | Testável sem GPU/sem olho? | Como testar |
|---|---|---|---|
| 0 | **Corrigir `VRMC_springBone`** no parser/exportador para o schema oficial (`specVersion`, `colliders[].shape`, `colliderGroups`, `springs[].joints[]{hitRadius,stiffness,gravityPower,gravityDir,dragForce}`, `center`); só em `extensionsUsed`; manter leitura legada do formato antigo | Sim | round-trip; validação contra o JSON-schema oficial do repo vrm-specification; ler arquivo de modelo exemplo; comparar com `vrm-runtime`/`vrm-spec` |
| 1 | **Spring bone VRM-exato** (Verlet, fórmulas da spec, ordem raiz->folha, `center`) | Sim | pêndulo: período vs. teoria para `stiffness=0` e gravidade; energia decai com `dragForce>0`; comprimento do osso conservado (erro < 1e-5); determinismo (hash do estado após N passos) |
| 2 | **Colisores** esfera/cápsula/plano/"inside" + grupos | Sim | cadeia caindo sobre esfera para; nunca penetra (invariante `dist >= r` após cada passo); casos de borda (ponta da cápsula) |
| 3 | **XPBD núcleo**: partículas, compliance, substeps, restrições de distância, flexão, tether, shape-matching à pose skinada | Sim | rigidez independente do número de iterações (propriedade XPBD); corda pendurada converge à catenária (tolerância); resíduo de restrição < eps; estável com dt grande (sem NaN) |
| 4 | **Cadeias de cabelo/fita em XPBD** + conversão guia->cadeia (`anigo-hair`) | Sim | mesmas propriedades; mechas não cruzam o colisor da cabeça |
| 5 | **Malha-proxy de saia/capa**: restrições laterais, backstop, colisão com cápsulas do corpo, vértices fixos por pesos | Sim (números) / Não (aparência) | saia sobre cápsulas de perna: sem penetração; estiramento de arestas < 5 %; comportamento em pose "perna levantada" (mede penetração); estabilidade 10 s |
| 6 | **Integração com skinning/morph**: cápsulas e landmarks seguem o esqueleto morfado; proxy -> render por vínculo; **reset ao teletransportar** | Sim | slider de peso muda raio das cápsulas; saia acompanha cintura |
| 7 | **Vínculo de roupa ao corpo + herança de pesos**; **weight inpainting** (Laplaciano) para roupa livre | Sim | soma=1; roupa move com o corpo em 20 poses; erro de posição vs. vínculo |
| 8 | **Auto-colisão** (hash espacial) | Parcial | contagem de interseções, custo por partícula; efeito visual precisa do olho |
| 9 | **Painéis+costura** (ferramenta offline) | Parcial | costuras fecham (distância 0), área preservada; "cai bem" precisa do olho |
| 10 | Export: cadeias -> `VRMC_springBone`; cloth de malha -> bake em cadeias (com aviso) | Sim | round-trip por `vrm-runtime` |

**Exigem olho/mão humana (H)**: calibrar `stiffness/drag/gravity` por "sensação"; atravessamentos visíveis em close; estética da saia/cabelo em movimento; limiares de colapso de junta; gosto da silhueta.

### F4. Ferramenta de cabelo estilo VRoid no workspace Modelagem

**Modelo de dados** (em `anigo-hair`, serializável, determinístico):
```
HairStyle { groups: Vec<HairGroup>, scalp: ScalpRef /*head mesh + hair_scalp_region*/, mirror_x: bool }
HairGroup { name, kind: Freehand|Procedural, guide: GuideGrid /*superfície/rede de curvas*/,
            strands: Vec<Strand /*Curve com CurvePoint{pos,radius,tilt}*/>,
            params: { count, cross_section: Flat|Arc|Triangle, curl, twist, taper, taper_power, flare, width, length },
            material: { base_tex, highlight_tex, atlas_cell }, bounce: Option<BounceGroup> }
BounceGroup { strands: Vec<usize>, bones: 1..=16, fixed_point: 0.01..=0.99, stiffness, drag, gravity, hit_radius, axis_strand }
```
Reaproveita `Curve`, `curve_to_ribbon`, `hair_card` (taper, curl, flare, atlas, normal bias), `hair_clump` e o BVH existentes [V].

**Fluxo de uso (UX)**
1. *Superfície-guia*: sobre a cabeça (BVH), mostrar a grade-guia em offset da cabeça (como o VRoid), editável por pontos de controle.
2. *Pincel freehand*: traço do mouse/caneta é projetado por raycast na cabeça/guia, amostrado (comprimento uniforme), suavizado (Chaikin/spline) e vira curva da **raiz (couro cabeludo) à ponta**; a direção do traço define a direção da mecha e depois a direção dos ossos [V - VRoid]. Largura/afinamento por pressão.
3. *Extrusão em card*: `hair_card` com `width`, `taper`, `curl`, `flare`; **ponta afinada** por `taper≈0`; UV para atlas de highlight; normais de card viesadas ao centro da cabeça (já implementado) [V].
4. *Procedural*: contagem, seção transversal, torção, espalhamento ao longo da guia; "converter em freehand" cria pontos de controle individuais.
5. *Simetria*: espelho em X de grupo e de mecha (flip), simétrico ao vivo (como `mirror_x` do VRoid) [V].
6. *Retouch*: refazer a forma de um grupo seguindo novo traço (reprojetar pontos preservando raiz).
7. *Smoothing/decimate*: reduzir pontos de controle (Ramer-Douglas-Peucker + re-ajuste).
8. *Materiais*: base+highlight por grupo; angel ring pela tangente do fio (atributo de direção) [V doc cel].
9. *Física (bounce)*: botão "Auto-gerar grupos" (1-30 por k-means/farthest-point nas raízes; ossos 1-16 amostrados por comprimento de arco da curva); parâmetros VRoid-compatíveis; pesos dos vértices do card nos 2 ossos vizinhos por `t`; cabeça/pescoço/ombro como colisores; simulação ao vivo em `anigo-physics`; export `VRMC_springBone`. Regra do VRoid a replicar e **corrigir**: editar o penteado depois **recalcula** o eixo/ossos (no VRoid isso não acontece [V]).
10. Mesmo editor, em **Modelagem**, vira "curva -> geometria" genérica (cabo, fita, cordão, franja de roupa).

**Testes sem GPU**: curva projetada fica a offset constante da cabeça (erro < eps); espelho é involução (flip(flip(g))=g); `taper=0` -> ponta degenerada em triângulo; contagem de ossos por grupo respeitada; pesos de card somam 1; mechas simuladas não penetram o colisor da cabeça; serialização determinística (hash). H: gosto de forma, naturalidade do movimento.

---

## G. Riscos e decisões que precisam de você

1. **Contrato do renderer** (paleta de 24 ossos, 72 B por vértice, canais de vertex color) - mudar para N ossos e UV1 de máscaras. Sem isso, dedos, cabelo e saia não animam no renderer.
2. **Faixa do macro de idade** (item 9 do resumo).
3. **Topologia congelada**: depois de fixado o contrato, mudar a topologia invalida morphs, pesos, roupas e física. Por isso a Gen-0 da nuvem deve ser aprovada pelo usuário (olhos) antes de autorar 150+ morphs sobre ela.
4. **VRM springBone atual está fora do padrão** (item 4 do resumo) - corrigir antes de qualquer física.
5. **Dependências de terceiros muito novas** (`rapier-cloth`, `oxihuman-*`, `vrm-runtime`): usar como leitura/oráculo, não como base.
6. **Licenças**: MakeHuman código AGPL (não copiar código; assets CC0 ok); MPFB GPL-3; StarRailNPRShader GPL-3 (só ler) [V, doc cel]; `smesh` MPL-2.0; `RobustSkinWeightsTransferCode` MIT (pode portar com crédito); `animebasemesh.glb` CC-BY (atribuição, não redistribuir).
7. **Validação local necessária (H)**: ver F3 (olho humano) + comparar Gen-0 com as imagens de referência (silhueta, olhos, mãos), peso dos sliders e sensação da física.

## H. Fontes consultadas

[V] código do repo: `crates/anigo-core/src/{mesh,skinning,bone_sync,morph_catalog,morph,somatotype}.rs`, `crates/anigo-vrm/src/{vrm1,exporter,lib}.rs`, `crates/anigo-cloth|anigo-hair/src/lib.rs`, `crates/anigo-modeling/src/{curve,shapekeys}.rs`, `docs/native/MODELAGEM_FERRAMENTAS.md`, `docs/native/pesquisa/{cel_shading_estado_da_arte,cpp_vs_rust_modelagem}.md`, `docs/native/CPP_FFI.md`, `docs/native/VALIDACAO_LOCAL.md`; metadados de `modelos/feminino/animebasemesh.glb`.
[V] VRM spec (vrm-c/vrm-specification, master): `VRMC_vrm-1.0/{README,humanoid,expressions}.md`, `VRMC_springBone-1.0/README.md`, `VRMC_springBone_extended_collider-1.0/README.md`, `VRMC_node_constraint-1.0/README.md`, `VRMC_materials_mtoon-1.0/README.md`.
[V] VRoid FAQ (vroid.pixiv.help): "How do I customize hairstyles?", "I want to edit hair bounce", "How to check the number of polygons", "Face Editor", "Getting Started with VRoid", "Photo Booth parameters".
[V] Magica Cloth 2 - Parameter Setting (magicasoft.jp/en/mc2_baseline/) e Shape Restoration.
[V] Apple ARKit `ARFaceAnchor.BlendShapeLocation` (52 nomes, via JSON da doc).
[V] Blender manual Rigify (basics, metarigs). [V] MakeHuman: technical notes (targets/modifiers/proxy `.mhclo`), Modeling the body, licença CC0/AGPL, FAQ MPFB.
[V] DesignDoll - Update History (terawell.net). [V] GPUOpen TressFX. [V] Abdrashitov et al. 2023 (página do projeto + repo MIT). [V/S] resultados de busca: Dionne & de Lasa 2013; Macklin et al. 2016/2019 (citados em arXiv).
[V] crates.io (API): rapier3d, parry3d, avian3d, bevy_xpbd_3d, rapier-cloth(-core), bevy_softbody, bevy_silk, vrm-runtime, bevy_vrm1, vrm-spec, serde_vrm, oxihuman-*, symbios-avatar, mesh_to_sdf, bvh_anim, mmd-anim-*, nalgebra-sparse, sprs, faer, halfedge, smesh, manifold_harmonics, meshopt, glam, bvh, obvhs.
[V] READMEs: neka-nat/rapier-cloth, nanowater/vrm-runtime-rs, cool-japan/oxihuman (+SAFETY.md).
[S] nota "Should I delete transparent meshes when exporting VRM from VRoid Studio?" (Melvina Sakura); lista `Fcl_*` (ordinaryanimator.com); VSeeFace/Warudo/VMagicMirror (Perfect Sync); Reddit r/VRoid.
[C] todo o resto marcado [C] (regras gerais de modelagem, estimativas de custo, parâmetros típicos, Marvelous/CLO/Chaos/Daz/MB-Lab/Pinocchio/libigl).
