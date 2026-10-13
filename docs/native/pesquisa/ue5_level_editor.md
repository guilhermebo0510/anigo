# Pesquisa: o que o editor de níveis do Unreal Engine 5 faz para montar cenários — e o que replicar no workspace "Cenário" do ANIGO (out/2026)

Legenda (a mesma de `biblioteca_assets.md`): **[V]** = página lida por inteiro nesta sessão (doc oficial da Epic / manual do Blender, Unity, Godot);
**[S]** = visto só em trecho de busca, em artigo da Epic de autoria da comunidade/tech blog, ou em fonte secundária; **[C]** = conhecimento prévio/inferência,
NÃO verificado agora. Estado em 2026-10-12; a documentação oficial lida é a do **UE 5.8** (as páginas mostram 5.8 como versão corrente), salvo quando indicado "4.27".

Limitações desta pesquisa (honestidade): a Epic **não publica** (nas páginas que consegui achar) uma tabela única "atalhos padrão do level editor" — a URL
`.../unreal-editor-keyboard-shortcuts-and-hotkeys` redireciona para a home. A tabela de atalhos da §8 é, portanto, uma **composição**: o que as páginas oficiais
dizem [V] + o guia de atalhos do tech blog da Epic (Carbon Studio, 2019) [S] + uma lista da comunidade no portal da Epic, feita lendo Editor Preferences [S]
+ sites de atalhos de terceiros [S]. O que só aparece em terceiros está marcado. Alguns slugs de doc (Layers, Mesh Paint, Spline, Sky Atmosphere, Lumen) não
foram lidos na íntegra — marcados [S]/[C]. O Firecrawl teve limite de taxa (429) intermitente; WebFetch falha por DNS (como previsto).

Contexto local lido no repo [V] (afeta a proposta): `crates/anigo-core` **já tem** `SceneState{nodes: Vec<NodeSlot>, camera, lights}`, `NodeSlot{node_id, name, transform,
mesh: Option<MeshRef{asset_id,primitive_index}>, material_id, visible, parent_id, kind: NodeKind}`, `NodeKind{CharacterRoot,…,Mesh,Light,Camera,Group}`, hierarquia
com ordem topológica/ciclos (`hierarchy.rs`), `Command::{AddNode, RemoveNode, SetNodeParent, SetNodeTransform, SetNodeVisibility, Batch}` e `CommandHistory` com
`HistoryEntry{description, inverse, command}`; `crates/anigo-studio` já tem `OrbitCamera` (orbit/pan/zoom_to_cursor/frame_bounds/ViewPreset/ray_from_pixel/intersect_plane),
`app/gizmo.rs` (mover/rotacionar/escalar, hit-test de alças, tamanho constante em pixels), `viewport/picking.rs` (`ray_triangle`, `raycast_mesh` linear, caixa/clique de v/e/f),
`library_ui` publica `AssetDragPayload{ids,label,kind}` para arrastar cards (o "soltar no viewport" está documentado como **não implementado**, `BIBLIOTECA_UI.md` §7),
`anigo-assets` guarda `AssetStats.bbox` (AABB), `tags`, `AssetKind` {model, character, outfit, hair, accessory, material, texture, pose, animation, hdri, …}.
O workspace `Cenario` existe como enum/aba mas T-2.11 ("Posing/Shading/Iluminação/Cenário… nativos") está ⬜.

---------------------------------------------------------------------------------------------------

## 1. Viewport do nível

### 1.1 Navegação (perspectiva) — [V] `viewport-controls-in-unreal-engine`
| Gesto | Efeito |
|---|---|
| LMB + arrastar | move a câmera para frente/trás e gira esquerda/direita (modo "walk" do mouse) |
| RMB + arrastar | gira a câmera (olhar em volta) |
| LMB+RMB + arrastar **ou** MMB + arrastar | pan (cima/baixo/esquerda/direita sem girar) |
| Roda do MMB | avança/recua em incrementos (dolly) |
| **RMB segurado + W/A/S/D** (ou Numpad 8/4/2/6, setas) | voo (fly): frente/esq./trás/dir. |
| RMB + E / Q (Numpad 9/7, PgUp/PgDn) | sobe / desce no espaço **global** |
| RMB + R / F | sobe / desce no espaço **local** da câmera |
| RMB + Z / C (Numpad 1/3) | aumenta / diminui o **FOV** da câmera |
| **RMB + roda** | aumenta/diminui a **velocidade da câmera** |
| **F** | focar (enquadrar) o objeto selecionado |
| **Alt+LMB** arrastar | **orbitar** em torno de um pivô/ponto de interesse (o ponto que o F focou) |
| Alt+RMB arrastar | dolly (zoom) em direção ao pivô |
| Alt+MMB arrastar | tracking (pan) |

* O esquema WASD é configurável: Editor Preferences → Level Editor → Viewports → **Flight Camera Control type** = "WASD sempre", "WASD só com RMB" (padrão) ou "nunca" [V `unreal-editor-preferences`].
  Atenção: quando ativo, o WASD sobrepõe atalhos de letra (inclusive show flags) — daí o padrão ser "só com RMB".
* Opções de câmera/ortho [V `viewport-toolbar` e `unreal-editor-preferences`]: **Orbit Around Selection**, **Link Ortho Camera Movement** (todas as ortográficas se movem juntas),
  **Ortho Zoom to Cursor**, **Grab and Drag to Move Orthographic Cameras**, **Aspect Ratio Axis Constraint** (mantém FOV em Y, X ou eixo maior ao redimensionar),
  Mouse Sensitivity / Mouse Scroll Zoom Speed / Invert Middle Mouse Pan / Invert Orbit Axis / Invert Right Mouse Dolly / Scroll Gestures.
* **Velocidade da câmera** [V]: slider "Camera Speed" na toolbar; **Speed Scalar** multiplica o slider; **Distance Based Camera Speed** escala a velocidade pela distância câmera↔ponto de interesse
  (isto é o que torna o fly utilizável em cena pequena e grande — vale copiar).
* **Perspectiva**: FOV padrão **90°**; Near View Plane e Far View Plane ajustáveis (o Far não afeta Nanite) [V].
* **Pilot** [V]: "Pilot [ator selecionado]" prende a viewport ao ator (move o ator com os controles da câmera); "Stop Piloting"; "Exact Camera View"; atalho Ctrl+Shift+P [S comunidade].
  **Move Camera to Object** / **Move Object to Camera** [V] (câmera↔objeto; o tech blog [S] destaca isso para pôr uma câmera exatamente onde se olha).
* **Preview Selected Cameras** [V]: ao selecionar um ator-câmera, aparece um picture-in-picture com a visão dessa câmera dentro da viewport (tamanho configurável).
* **Bookmarks de câmera** [V toolbar: "Create → Bookmarks: Set Bookmark, Manage Bookmarks (Clear, Compact, Clear All), lista com o atalho de cada um"]. Atalhos: **Ctrl+1…9** grava, **1…9** volta [S blog Epic: "Set camera (Ctrl+1-9)"; recall pela tecla do número = [C]]. Até ~10 slots [S].
* **Layouts de viewport** [V]: vários painéis (1, 2, 4…) e alternância maximizar/layout (botão no overlay); **F11 immersive** (viewport em tela cheia), Shift+F11 abre a barra de tarefas do editor [S].
* **Tipos de viewport**: o layout "Cinematic Viewport" adiciona o menu **Composition Overlays** (guias de enquadramento/máscara/composição) [V `viewport-toolbar`] — é o análogo UE da "regra dos terços".

### 1.2 Ortográficas — [V] controles, [S] atalhos
Controles [V]: LMB = clique/caixa de seleção (marquee); RMB arrastar = pan; LMB+RMB = dolly; roda = zoom; nas ortográficas só se move/escala em **2 eixos** (sem o centro 3D do gizmo).
Câmera → Orthographic: top, bottom, left, right, front, back [V]. Atalhos: **Alt+G** perspectiva, **Alt+H** front, **Alt+J** top, **Alt+K** side [S: tech blog da Epic lista "Alt+G/K/H/J"; ordem exata confirmada por lista de terceiros (Front = Alt+H)].
> Correção a uma premissa do pedido: **Alt+H não é "unhide"** — é a vista Front. Mostrar tudo é **Ctrl+H** [V]; esconder selecionados é **H** [V].

### 1.3 Modos de visualização (View Modes) — [V] `viewport-modes-in-unreal-engine`
| Modo | Atalho | O que mostra |
|---|---|---|
| **Lit** | Alt+4 | resultado final (materiais + luz) |
| **Unlit** | Alt+3 | só Base Color, sem luz |
| **Wireframe** | Alt+2 | arestas dos polígonos |
| **Detail Lighting** | Alt+5 | material neutro, **com** normal maps (isola se o BaseColor esconde a luz) |
| **Lighting Only** | Alt+6 | material neutro, **sem** normal maps |
| Light Complexity | Alt+7 | nº de luzes não-estáticas por pixel |
| Shader Complexity | Alt+8 | custo de instruções por pixel |
| Lightmap Density | Alt+0 | densidade de texels de lightmap |
| Stationary Light Overlap, Reflections, Player/Visibility Collision, LOD/HLOD Coloration | só menu | diagnósticos |
| Buffer Visualization | só menu | Base Color, Roughness, Metallic, Scene Depth, World Normal, AO, Shading Model, … |
| Landscape visualizers | menu | Normal, LOD, Layer Density |
Também **Exposure** (automática vs fixa) para não "pulsar" ao passar de claro a escuro [V]. Console: `viewmode lit`, `viewmode unlit`, etc. [V].
Há ainda **Preview Platform** (aproxima a aparência em outra plataforma/shader model), **Viewport Scalability**, **Material Quality Level**, **Screen Percentage** [V toolbar].

### 1.4 Game View, realtime, show flags, stats, grade
* **Game View (G)** [V]: renderiza só o que existiria no jogo — esconde gizmos, ícones, volumes, grade, outline de seleção. Também no menu Options ("Game View").
* **Realtime**: **Ctrl+R** alterna atualização por frame [V]; desligado, aparece um ícone de aviso na toolbar que o religa com um clique (padrão "indicador de aviso acionável") [V].
* **Show Flags** [V toolbar]: lista de recursos do motor para ligar/desligar na viewport (partículas, pós-processo individual, geometria, luz…) — ver página `viewport-show-flags-in-unreal-engine` (não lida) [S].
* **Stats**: `stat fps`/`stat unit` por console [C]; **Ctrl+Shift+H** mostra FPS e **Shift+L** liga/desliga rápido os stats ativos [S comunidade].
* **Grade**: na ortográfica a grade mostra linhas de referência com intervalo configurável (**Decimal Grid Intervals** / **Pow2 Grid Intervals**) [V `actor-snapping`]. Na perspectiva a grade do mundo aparece como no "Game View off" [C].
* **High Resolution Screenshot** (menu Options): captura com recorte (Crop) [V]; F9 screenshot rápido [S].
* **Translucent Selection (T)** [S blog Epic]: permite clicar através de objetos translúcidos (vidro, god rays).

Preferências de aparência [V]: "Highlight Objects Under Mouse Cursor" (realce de hover), "Enable Combined Translate/Rotate Widget" (gizmo combinado mover + girar Z), "Preserve Actor Scale on Replace".

---------------------------------------------------------------------------------------------------

## 2. Seleção e transformação

### 2.1 Seleção
* Perspectiva: LMB seleciona (substitui); **Ctrl ou Shift + LMB** adiciona [V]. **Ortográfica**: LMB arrastar = caixa (marquee); Shift+arrastar adiciona; **Ctrl+RMB arrastar remove** da seleção [V].
  Na perspectiva, a caixa de seleção é um modo próprio ("Box Select"/Ctrl+Alt+LMB arrastar) [C].
* **Ctrl+A** seleciona tudo; **Esc** limpa a seleção [S comunidade/Domestika]. **Ctrl+Shift+A**: selecionar todos os atores da mesma classe [S comunidade]; **Shift+E**: todos que usam o **mesmo static mesh** do selecionado [V `transforming-actors`].
  Menu de contexto → Select → **Select Matching (Selected Class)** e **Invert Selection** (também por material, Blueprint etc.) [S blog Epic]. Selecionar filhos/ascendentes/"select all children" existe no menu Select [C].
* Ocultar: **H** esconde selecionados; **Ctrl+H** mostra todos [V]. "Isolar" não é um comando nativo nítido (o fluxo é ocultar o resto + Ctrl+A) — o blog da Epic [S] cita exatamente esse truque; o equivalente forte é o **Local View do Blender** (§9).
* **Del** apaga [S]. **M** torna o nível do ator selecionado o nível corrente [S blog Epic]. **Ctrl+E** abre o asset do ator selecionado [S comunidade].
* **Hover highlight** (preferência) [V]; seleção usa contorno amarelo/laranja (não verificado em doc) [C].

### 2.2 Gizmo — [V] `transforming-actors-in-unreal-engine`
* **Q** seleção (esconde gizmo), **W** mover, **E** girar, **R** escalar; **Espaço** cicla mover→girar→escalar [V]. Eixos X vermelho, Y verde, Z azul; **Z é a vertical** [V].
* **Mover**: setas por eixo; **quadradinho entre dois eixos = plano** (XY/XZ/YZ); **esfera/quadrado central cinza = livre nos 3 eixos** (a roda do mouse aproxima/afasta durante o arrasto) [V].
* **Girar**: 3 arcos coloridos (o arco no plano XY gira em Z) + arco branco externo (eixo apontando à câmera) + **arcball** (arrastar no espaço entre arcos = rotação livre); mostra o ângulo em tempo real; ao girar o gizmo reduz ao arco ativo [V].
* **Escalar**: alças com cubos por eixo; **triângulos entre eixos = escala em 2 eixos (plano)**; **cubo central = uniforme**; com scale-snap ligado o centro vira esfera [V]. Escala usa sempre espaço **local** [V].
* **Alt + arrastar** uma seta duplica o ator (original fica na origem do arrasto); Alt+girar duplica sobrepondo; **tocar Alt durante um arrasto "carimba" cópias** no ponto atual [V].
* Visibilidade: Settings → "Show Transform Widget" [V]. Preferências **Gizmos** (5.8): *Gizmo Presets* (padrão novo, UE Classic, estilo Maya), *Gizmo Interaction*, *Gizmo Appearance*, *Legacy Gizmo* (volta ao gizmo ≤5.7) [V]. Atalhos **[ / ]** (grade de posição), **Shift+[ / ]** (grade de rotação), **Alt+[ / ]** (tamanho do gizmo) [S comunidade].

### 2.3 Espaços de coordenadas — [V] mesma página
**World** (padrão), **Local**, **Parent** (eixos do pai) e **Explicit** (mover nos eixos do pai e girar com eixos de gimbal; habilitado em Editor Preferences → Gizmos → Gizmo Interaction → *Additional Coordinate Systems*).
Alternância: ícone na toolbar cicla; menu "⋮" escolhe direto; atalho **Ctrl+\`** alterna World/Local [V `viewport-toolbar`].
No painel Details a transformação é **relativa ao pai** por padrão e cada propriedade tem um dropdown para alternar relativo/mundo [V].

### 2.4 Pivô
* Pivô = onde os 3 eixos do gizmo se cruzam. **Alt+MMB arrastando uma alça de mover** reposiciona o pivô **temporariamente** (gira/escala em torno dele; pode ficar fora do objeto) [V].
* Menu de contexto → **Pivot → Set Pivot Offset Here / Reset Pivot** (pivô "permanente" por ator) [S blog Epic: "RMB → pivot → Set pivot offset here" e Alt+MMB para colocar onde se clicou]; **Ctrl+End = "Snap Origin to Grid"** (não é "pivô") [S Domestika]; **Alt+End = Snap Pivot to Floor**; **Shift+End = Snap Bounds to Floor** [S Domestika + comunidade].
* Para ativos, o pivô "de verdade" vem da malha (editado no Static Mesh Editor/Modeling Mode — "Pivot" tool) [C]; **Level Instance** pergunta o tipo de pivô: *Center Min Z*, *Center*, *Pivot Actor* [V `level-instancing`].

### 2.5 Duplicar / copiar / colar / apagar
* **Ctrl+W** duplica no mesmo local; **Alt+arrastar** duplica ao mover [V]. "Duplicate and Transform: Alt+Transform" [S].
* Ctrl+C / Ctrl+X / Ctrl+V e **Edit → Paste Here** (cola sob o cursor) existem [C — não verificado em doc]; Ctrl+V cola na **posição original** [C]. **Duplicate Special/offset** não é recurso nativo do level editor [C] (Blender tem Shift+R "repeat" e modificador Array; ver §9).
* **Del** apaga; **Replace Selected Actors with…** (menu de contexto, depende do asset selecionado no Content Browser) troca N atores pelo asset; preferência *Preserve Actor Scale on Replace* [V `placing-actors`, `unreal-editor-preferences`].
* **Merge Actors** [S `merging-actors-in-unreal-engine`]: *Merge* (uma seção de malha por material) e *Simplify* (malha-proxy única; materiais podem ser "baked"). **Group (Ctrl+G) / Ungroup (Shift+G)** + Lock/Unlock group no menu de contexto [S blog Epic].
* **Attach To** (menu de contexto) [S blog Epic: "ambos precisam ter a mesma Mobility"] e **arrastar um ator sobre outro no Outliner** (anexa) [V].

---------------------------------------------------------------------------------------------------

## 3. SNAP

### 3.1 O que existe — [V] `actor-snapping-in-unreal-engine`
1. **Surface Snapping** (botão na toolbar + dropdown): *Rotate to Surface Normal* (liga: o objeto gira para alinhar ao normal; padrão ON na 5.x [V toolbar]) e *Surface Offset* (distância objeto↔superfície). **Tecla End** com ator selecionado = snap ao chão/superfície mais próxima.
2. **Grid Snapping**: três grades independentes — **Drag Grid** (posição, grade 3D implícita), **Rotation Grid**, **Scale Grid** (aditiva; "Use Percentage Based Scaling" volta ao modo multiplicativo legado). Cada uma liga por ícone na toolbar; o valor sai do dropdown ao lado.
3. **Vertex Snapping**: segurar **V** com o gizmo de mover — aparecem os vértices alvo; ideal combinado com **mover o pivô até um vértice** (Alt+MMB) para encaixar canos/paredes/portas.
4. **2D Layer Snapping** (opcional): camadas Foreground/Default/Background editáveis — snap para "layers" de profundidade em projetos 2D/2.5D. Relevante ao anime (§10).
Atalhos [V `transforming-actors`]: **Z** (toque = alterna, segurar = ativa durante o arrasto) → Surface Snapping; **X** → snap de translação/rotação/escala conforme o gizmo atual; **V** → vertex snap.

### 3.2 Valores e onde configurar
* Toolbar: dropdown ao lado de cada ícone; **Editor Preferences → (Level Editor →) Viewports → Snap** [V]:
  *Enable Grid/Rotation/Scale Snapping*, *Use Power of Two Snap Size* (1,2,4,8,16…), arrays editáveis **Decimal Grid Sizes**, **Decimal Grid Intervals** (espaçamento das linhas na ortográfica), **Pow2 Grid Sizes/Intervals**, **Common Rot Grid Sizes** e **Divisions Of 360 Rot Grid Sizes** (dois menus de ângulo), **Scaling Grid Sizes**.
* Valores *padrão* dos arrays (não confirmados nesta sessão [C]): grade decimal 1, 5, 10, 50, 100, 500, 1000, 5000, 10000 cm; rotação "comum" 5°, 10°, 15°, 30°, 45°, 60°, 90°, 120°; "divisões de 360": 360/2, /3, /4, …; escala 10, 1, 0.5, 0.25, 0.125, 0.0625. (As unidades da engine são **centímetros**.)
* O pedido mencionava "escala de tempo": não há snap de tempo no level editor (isso é do Sequencer) — ignorado aqui.

### 3.3 Comandos de menu "Transform → Snap/Align" — [S blog Epic: existe "RMB → Transform → Snap/Align"]; nomes exatos [C]
Snap/Align Origin to Grid (**Ctrl+End** = Snap Origin to Grid [S]), Snap/Align to Floor (**End** [V]), Snap Pivot to Floor (Alt+End), Snap Bounds to Floor (Shift+End), Snap/Align Origin to Actor, Snap/Align to Actor ("Align to Actor" copia posição e rotação do ator-alvo).
* **Move Camera to Object / Move Object to Camera** [V]. "Place at origin": não há comando dedicado; faz-se com *Reset to Default* do campo Location no Details [V] (zera para o padrão).
* **Snap de socket**: sockets são pontos nomeados de malhas (Static/Skeletal); atores se anexam a sockets via *Attach To → (ator) → socket* [C]. Não existe "snap modular por socket" no level editor — a Epic resolve kits modulares com **grade + pivô nos cantos + vertex snap** (e no Modeling Mode/Level Instances) [C]. É um ponto onde o ANIGO pode **superar** o UE (§10, §11).
* **Pivot Offset**: ver 2.4.

---------------------------------------------------------------------------------------------------

## 4. Content Browser → viewport; Place Actors; prefabs

### 4.1 Colocar atores — [V] `placing-actors-in-unreal-engine`
* **Painel Place Actors** (Window → Place Actors): busca + abas **Recently Placed** (até 20 tipos, por projeto), **Basic**, **Lights**, **Shapes** (cubo, esfera, cilindro, cone, plano), **Cinematic** (câmeras), **Visual Effects** (fog, post process, reflexão), **Geometry** (brushes aditivos/subtrativos), **Volumes**, **All Classes**. Arrastar item para a viewport.
* **Arrastar e soltar do Content Browser**: Blueprint → instância do BP; Skeletal Mesh → SkeletalMeshActor; Static Mesh → StaticMeshActor; Sound Cue/Wave → AmbientSound [V]. O ator cai **no ponto sob o cursor** (na superfície atingida por raio; com "ghost"/prévia durante o arrasto na viewport) — o comportamento do raio+prévia é de uso conhecido [C], não descrito no texto lido.
* **Menu de contexto da viewport**: com um asset selecionado no Content Browser, RMB → *Place Actor* → asset (cai onde se clicou) [V].
* **Class Viewer** (Tools → Class Viewer): arrastar classes colocáveis (tingidas de azul) [V].
* **Replace Selected Actors with…** [V], **Merge Actors** [S], **Group** [S].

### 4.2 Reuso: grupos, Level Instances, Packed Level Actors, Blueprints — [V] `level-instancing-in-unreal-engine`
* **Level Instance** (ator que referencia um sub-nível): criado a partir da seleção (RMB → Level → *Create Level Instance*; opções *External Actors* (One File Per Actor), *Pivot Type* Center Min Z/Center/Pivot Actor). As cópias são **instâncias de uma só fonte**: editar uma propaga a todas.
  **Edit** (RMB → Level → Edit, ou botão no Details) deixa os demais atores **acinzentados** e edita *em contexto*; **Commit Changes** no Details salva; **Esc duas vezes** sai sem salvar. **Break** troca a instância pelos atores originais (**não desfaz**). Os atores internos ficam filhos no Outliner e **não são selecionáveis** fora do modo de edição.
* **Packed Level Actor / Packed Level Blueprint**: junta apenas **componentes visuais estáticos** (static mesh, ISM, HISM) num único ator-Blueprint otimizado (instancing) — ideal para prédios/kits estáticos; o resto é descartado [V + S `World Building Guide`: a partir da 5.4 atualiza só ISMs e preserva overrides]. Limitações [S]: sem overrides por ator dentro de uma Level Instance (variações só via *Actor Filters*, 5.3+, e *Property Overrides*, experimental na 5.5).
* **Blueprint como prefab** [C]: um BP com componentes é o "prefab" clássico; o equivalente Unity é o **Prefab** (§9).
* **Hierarquia (attach/detach)**: arrastar no Outliner [V], RMB *Attach To*/*Detach* [S].

---------------------------------------------------------------------------------------------------

## 5. Painéis

### 5.1 Outliner (antigo World Outliner)
[V `outliner-in-unreal-engine` 5.8 e `world-outliner(-ui)` 4.27]: árvore hierárquica de todos os atores; **LMB seleciona**, **RMB abre o mesmo menu de contexto da viewport**, **arrastar anexa** (pai/filho); **F** com ator selecionado no Outliner enfoca na viewport e vice-versa rola a lista até o ator; **Always Frame Selection** (setting) rola sempre ao selecionar na viewport.
Até **4 instâncias** do Outliner, cada uma com configuração própria. Colunas mostráveis/ocultáveis (botão direito no cabeçalho) e redimensionáveis. Coluna **Info** (4.27) mostra Level/Layer/ID Name. Ícones de **Mobility** (laranja = Movable, amarelo = Stationary, nada = Static) [V 4.27].
**Busca** [V]: termos parciais, todos precisam casar; `-termo` exclui, `+termo` exige casamento exato, `"aspas"` casa o nome longo inteiro; a busca vale para colunas ocultas também; pode **salvar a busca como filtro customizado** (guardado globalmente por usuário, igual aos filtros do Content Browser) e usa o *advanced search syntax*.
Pastas, olho de visibilidade, cadeado/lock e arrastar para pasta: existem no Outliner moderno [C — as páginas lidas não cobrem; "Actor Folder" aparece como contexto editável em *Actor Editor Context* [S]].

### 5.2 Details — [V] `level-editor-details-panel-in-unreal-engine`
Campo de **nome** do ator no topo (nome amigável, buscável no Outliner); **Search Details**; **Favorites** (estrela; experimental); indicador "diferente do padrão" com botão **Reset to Default** por propriedade; *Edit Conditions* (checkbox à esquerda habilita/desabilita a propriedade); propriedades `editconst` realçadas como não editáveis; categorias por código (Rendering, Lighting, Collision…) e widgets personalizados (**Transform**, Static Mesh, **Materials**, Actor, **Layers**).
**Transform** [V `transforming-actors`]: Location/Rotation/Scale numéricos por eixo (digitar ou **arrastar o valor**); multiseleção mostra *Multiple Values* e digitar sobrescreve em todos; **Reset to Default** por linha; **cadeado de escala** (escala uniforme); alternar relativo/mundo por propriedade. **Mobility** (Static/Stationary/Movable) fica logo abaixo do Transform [V `actor-mobility`].
Mobility [V]: *Static* = não muda no jogo e contribui para lightmaps pré-calculados (Lightmass); *Stationary* = pode mudar (cor/intensidade de luz) mas não mover; *Movable* = pode mover/escalar/trocar, sombras 100 % dinâmicas, e luzes Movable suportam **Lumen**. Slots de **Materials** com override por instância [V pelo painel] e seção *Rendering* (visibilidade, cast shadow) / *Tags* / *Collision* [C — categorias padrão, texto não lido].
Atores também têm **Tags** (lista de nomes) e **Actor Hidden In Game** [S tutorial Epic de layered rendering].

### 5.3 Layers, Levels, World Partition, Data Layers
* **Layers panel** [S]: "organiza atores no nível; seleciona e controla visibilidade de grupos relacionados" — RMB: *Add Selected Actors to Selected Layers*, *Select Actors*.
  Em mapas **World Partition** o sistema de Layers é **substituído por Data Layers** [V `world-partition---data-layers`]: layers carregáveis em editor *e* runtime; ações: *Add/Remove Selected Actors to/from Data Layer*, *Select Actors in Data Layer*, *Append/Deselect*, *Make Current Data Layer* (vira o **Actor Editor Context**: novos atores caem nele), *Make All Data Layers Visible*, *Hide Editor Data Layers/Actors/Unloaded Actors*, *Highlight Selected*.
* **Levels (sublevels)**: janela Levels com nível persistente + streaming; "M" torna o nível do ator o corrente [S]. **World Partition**: mundo dividido em células carregadas por região; **One File Per Actor (OFPA)** salva cada ator em arquivo próprio (menos conflito de merge; **não** dá para acessar atores sem o mundo carregado) [S `World Building Guide`].
* Para um app de anime com cenas pequenas, **World Partition/OFPA são desnecessários** [C]; "Layers" e "Data Layers" viram, no ANIGO, *Camadas de cena* (BG/MG/FG) e *Variações* (dia/noite) — §10.

---------------------------------------------------------------------------------------------------

## 6. Undo/Redo, autosave, recuperação
* **Ctrl+Z** desfaz, **Ctrl+Y** refaz; **Undo History** abre um painel com o histórico de ações [V: menu Edit dos editores — `static-mesh-editor-ui`, `material-editor-ui`, etc.; o level editor usa o mesmo sistema de transações].
* O sistema é **baseado em transações**: o editor registra cada alteração em Atores/Assets como uma transação [V `recovery-hub`: "Recovery Hub lê cada mudança em um Ator ou Asset como uma transação e mantém a lista das mais recentes"]; um arrasto de gizmo vira **uma** transação ao soltar [C]. Ferramentas do **Modeling Mode** têm undo fino *durante* a ferramenta e, ao aceitar, o histórico global guarda só o estado final [S `modeling-mode-in-unreal-engine`: "while using a tool you can undo or redo individual changes… once you accept, UE's undo history only tracks the state…"].
* **Autosave** [V `unreal-editor-preferences`]: *Enable AutoSave*, *Save Maps*, *Save Packages*, *Frequency in Minutes*, *Warning in Seconds* (aviso antes de salvar). **Recovery Hub** (plugin beta, antes "Disaster Recovery"): após crash oferece **Recover All** reaplicando a lista de transações; funciona **melhor junto com autosave**; só cobre edição de Level e Sequencer; cada save/autosave também é uma transação [V].
  → O ANIGO já tem `session.json` (restauração de sessão) e `CommandLog` (log de operação reproduzível: `ReplayedLog` em `command.rs`) — é o análogo exato do Recovery Hub.

---------------------------------------------------------------------------------------------------

## 7. Ferramentas de cena (resumo)

| Ferramenta | O que faz | Fonte |
|---|---|---|
| **Landscape** (Modes → Landscape) | 3 modos: **Manage** (criar/gerir componentes, copiar/importar/exportar), **Sculpt** (pincéis para elevar/abaixar/suavizar/erosão…), **Paint** (pinta *target layers* do material da paisagem, ex.: Soil). Heightmaps até 8192², LOD geo-mipmap contínuo, colisão por heightfield, *edit layers* | [V] `landscape-overview`, [S] quick start |
| **Foliage Mode** (**Shift+3**) | pincel de **instâncias**: ferramentas *Select, All, Deselect, Invalid, Lasso, Paint (Shift = apagar), Reapply, Single, Fill, Erase, Remove, Move*; opções: Brush Size, Paint/Point Density, Erase Density, *Single Instance Mode* (All Selected / Cycle Through Selected), *Place in Current Level*; filtros de alvo (Landscape, Static Mesh, BSP, Foliage, Translucent); *Foliage Type* = **Static Mesh Foliage** (hardware instancing: 1 draw call por cluster) ou **Actor Foliage** (custo de ator normal); por tipo: Radius (não sobrepor), Scale Min/Max, **Align to Normal**, Cull Distance Start/End (+ PerInstanceFadeAmount), LOD, densidade escalável; **End** reaterra instâncias soltas | [V] `foliage-mode-in-unreal-engine` |
| **Mesh Paint** | pinta cores de vértice/texturas em instâncias de malha | [C] |
| **Modeling Mode** | ferramentas de modelagem dentro do editor de níveis (extrude, bevel, UV, pivot, ISM editor…); undo com granularidade por ferramenta | [S] snippets |
| **Splines** | componentes spline (cercas, estradas, rios); Landscape Splines para estradas deformando o terreno | [C]; "Landscape spline" aparece em settings [V] |
| **Water**, **PCG**, **Sequencer/Cinematics** | só citados: sistema de água, geração procedural e linha do tempo/câmeras | [C] |
| **Luzes** | **Directional** (sol, "distância infinita"), **Sky Light** (captura o céu e ilumina), **Point**, **Spot**, **Rect**; mobilidade **Static / Stationary / Movable** (Stationary limitado a 4 por objeto; Movable = sombra dinâmica mais cara; Movable sem sombra é barato) | [V] `light-types-and-their-mobility` |
| **Sky Atmosphere** + Volumetric Clouds | céu físico dirigido pela Directional Light (atmosfera) | [C] |
| **Post Process Volume** | volume com **Priority** (maior vence), **Blend Weight** (0–1) e **Infinite Extent (Unbound)** (vale na cena toda); *Blend Radius* para transição | [V] `post-process-effects` |
| **Light Build / Lightmass** | bake de luz pré-calculada para Static/Stationary; **Lumen** = GI e reflexos dinâmicos para luzes/atores Movable | [V] (menções em `actor-mobility`) |
| **Merge Actors**, **Instanced Static Mesh (ISM/HISM)** | fundir malhas / instanciar muitas cópias | [S] |

Desempenho relativo das luzes locais [S tutorial Epic comunitário]: Sky Light < Directional < Spot < Point (~6× o spot: cubemap de sombra) < Rect.
O ANIGO usa cel shading — **sem GI física**: luzes são `StylizedLight` (direção, cor, sombra com hue-shift, hemisfério) e não há bake; logo Mobility/Lightmass/Lumen **não se aplicam** (§10).

---------------------------------------------------------------------------------------------------

## 8. Atalhos padrão do viewport/level editor (tabela)

Todos configuráveis em **Edit → Editor Preferences → (General →) Keyboard Shortcuts** [V `viewport-controls`/blog Epic]; exportáveis [S blog Epic].
Coluna "Fonte" usa a legenda acima. Itens com "—" no atalho não têm padrão confirmado.

| Categoria | Ação | Atalho | Fonte |
|---|---|---|---|
| Navegação | Orbitar (em torno do ponto focado) | Alt+LMB arrastar | [V] |
| | Dolly para o pivô | Alt+RMB arrastar | [V] |
| | Pan/tracking | MMB arrastar · Alt+MMB · LMB+RMB | [V] |
| | Olhar em volta | RMB arrastar | [V] |
| | Voar | RMB + W/A/S/D (Numpad 8/4/2/6, setas) | [V] |
| | Subir/descer (global) | RMB + E/Q (PgUp/PgDn, Numpad 9/7) | [V] |
| | Subir/descer (local) | RMB + R/F | [V] |
| | FOV maior/menor | RMB + Z/C (Numpad 1/3) | [V] |
| | Velocidade da câmera | RMB + roda | [V] |
| | Focar seleção | F | [V] |
| | Bookmark de câmera (gravar) | Ctrl+1…9 | [S] |
| | Bookmark (voltar) | 1…9 | [C] |
| | Perspectiva / Front / Top / Side | Alt+G / Alt+H / Alt+J / Alt+K | [S] |
| | Immersive (tela cheia) | F11 | [V] |
| | Pilotar ator selecionado | Ctrl+Shift+P | [S] |
| Visualização | Game View | G | [V] |
| | Realtime | Ctrl+R | [V] |
| | Wireframe / Unlit / Lit / Detail Lighting / Lighting Only | Alt+2 / 3 / 4 / 5 / 6 | [V] |
| | Light Complexity / Shader Complexity / Lightmap Density | Alt+7 / 8 / 0 | [V] |
| | Mostrar navmesh | P | [S] |
| | Translucent Selection | T | [S] |
| | Mostrar FPS / alternar stats | Ctrl+Shift+H / Shift+L | [S] |
| | Screenshot | F9 | [S] |
| Seleção | Selecionar (esconde gizmo) | Q | [V] |
| | Adicionar à seleção | Ctrl+clique ou Shift+clique | [V] |
| | Marquee (ortho) / remover da seleção (ortho) | LMB arrastar / Ctrl+RMB arrastar | [V] |
| | Selecionar tudo | Ctrl+A | [S] |
| | Desselecionar | Esc | [S] |
| | Mesma classe | Ctrl+Shift+A | [S] |
| | Mesmo static mesh | Shift+E | [V] |
| | Torna o nível do ator corrente | M | [S] |
| Transformar | Mover / Girar / Escalar | W / E / R | [V] |
| | Ciclar ferramentas | Espaço | [V] |
| | World ↔ Local | Ctrl+\` | [V] |
| | Pivô temporário | Alt+MMB na alça de mover | [V] |
| | Surface snap | Z (toque alterna, segurar ativa) | [V] |
| | Snap da ferramenta atual | X | [V] |
| | Vertex snap | V (segurar) | [V] |
| | Snap to Floor | End | [V] |
| | Snap Bounds to Floor / Pivot to Floor / Origin to Grid | Shift+End / Alt+End / Ctrl+End | [S] |
| | Redimensionar grade de posição / rotação | [ ] / Shift+[ ] | [S] |
| | Redimensionar gizmo | Alt+[ ] | [S] |
| Edição | Duplicar | Ctrl+W | [V] |
| | Duplicar arrastando / carimbar cópia | Alt+arrastar / Alt (tocar durante o arrasto) | [V] |
| | Apagar | Del | [S] |
| | Esconder selecionados / mostrar todos | H / Ctrl+H | [V] |
| | Agrupar / desagrupar | Ctrl+G / Shift+G | [S] |
| | Desfazer / Refazer | Ctrl+Z / Ctrl+Y | [V] |
| | Copiar/Colar | Ctrl+C / Ctrl+V | [C] |
| | Editar asset do ator | Ctrl+E | [S] |
| | Achar no Content Browser | Ctrl+B | [S] |
| Modos | Selection / Landscape / Foliage / Modeling | Shift+1 / Shift+2 / Shift+3 / Shift+4 | [S: Foliage = Shift+3 confirmado em doc oficial [V]; os demais por lista de terceiros] |
| Outliner | Focar na viewport / rolar até o ator | F | [V] |
| Level Instance | Sair de edição sem commit | Esc Esc | [V] |

---------------------------------------------------------------------------------------------------

## 9. Equivalentes em outros editores (só o que acrescenta)

**Blender 5.2** [V `editors/3dview/controls/snapping.html`] — o modelo de snap mais completo e mais fácil de **copiar como UI**:
* *Snap Base* (que ponto do objeto encaixa): **Closest** (canto da bbox/vértice mais perto do alvo), **Center** (pivô), **Median**, **Active**.
* *Snap Target* (a que encaixa): **Increment** (passos a partir da posição original), **Grid**, **Vertex**, **Edge**, **Face**, **Volume**, **Edge Center**, **Edge Perpendicular**, **Face Center**; vários ao mesmo tempo com Shift+LMB; individuais: **Face Project** (projeta ao longo da visão) e **Face Nearest** (inclusive geometria oculta).
* Opções: **Align Rotation to Target** (Z ao normal), Backface Culling, Exclude Non-Selectable, **Absolute Increment Snap**, Affect (mover/rotar/escalar), Rotation Increment + *Precision Increment* (Shift).
* **Ctrl segurado inverte** o snap temporariamente; **Shift+Tab** liga/desliga. Pivot Point (Median, Individual Origins, 3D Cursor, Active, Bounding Box Center) e **3D Cursor** como pivô manual — melhor que o pivô temporário do UE.
* Coleções/Local View: **Collections** (grupos hierárquicos com visibilidade/seleção/render por coleção) e **Local View** (Numpad /) = "isolar". Blender não é verificado aqui além do snapping [C].

**Unity 6.6** [V `PrefabVariants.html`]: *Prefab Variant* herda de um prefab-base; **overrides** da variante têm precedência; variantes podem ter como base outras variantes; menu de overrides com **Revert All** / **Apply all to Prefab Variant parent**; o Inspector mostra a árvore de herança e a contagem de overrides. Aninhamento (*Nested Prefabs*) e *Prefab Mode* (editar em contexto) [S títulos das páginas vizinhas]. **ProBuilder** (blocagem por polígonos in-editor) [C].
→ É o modelo certo para "set pronto + variações" (sala dia/noite, mesa com/sem objetos) — melhor que as Level Instances do UE (sem overrides até a 5.5).

**Godot 4.7 GridMap** [V `using_gridmaps.html`]: *MeshLibrary* (coleção de malhas com colisão/nav, materiais dentro da malha) + nó GridMap com `Cell Size`; barra: Transform, **Selection** (área), **Erase**, **Paint**, **Pick** (conta-gotas), **Fill** (preenche a seleção), **Move**, **Duplicate**, **Delete**, **Cursor Rotate X/Y/Z** (gira o que será pintado), **Change Grid Floor** (Ctrl+roda = andar do piso), Filter Meshes, Zoom, **Pick Distance** máximo. "Não é um sistema de colocar nós na grade; é um sistema otimizado para malhas".
→ O fluxo **pintar peças modulares numa grade com cursor rotacionável e piso ajustável** é exatamente o que falta ao UE e é perfeito para sets de anime (corredor escolar, quarto).

---------------------------------------------------------------------------------------------------

## 10. Para ANIME / cel shading: o que um cenarista precisa além do UE

Nota: fontes verificadas são poucas; quase tudo abaixo é **[C]** (raciocínio de produção) com apoio [S] onde indicado.

1. **Câmera como ferramenta principal, em lente real.** O UE tem *Cine Camera* com **Filmback/Sensor** + **Focal Length** e **Current Horizontal FOV** calculado (read-only) [V/S `cinematic-cameras`, 4.27 `cinematic-depth-of-field-method`: "quanto maior a focal, menor o FOV"]. No ANIGO: câmera de cena com *sensor* (35 mm full-frame, Super35, 16:9…), **focal em mm** (24/35/50/85/135…) → FOV vertical = `2·atan(sensor_h / (2·f))`; botão "snap câmera à vista" (Move Object to Camera) e **vários planos/shots** (cada shot = câmera + luz + visibilidade de camadas).
2. **Guias de composição** sobre a viewport: regra dos terços, centro, diagonais, **golden spiral**, safe areas (title/action safe), **moldura do aspect ratio de saída** (letterbox 16:9/2.39:1) — o UE só tem isso no *Cinematic Viewport → Composition Overlays* [V]; para o cenarista deve estar sempre acessível. Barato (egui painter).
3. **Fundos pintados / billboards / cards.** O fluxo clássico de anime 3D usa **cartões planos com arte pintada** em planos de profundidade (BG/MG/FG) e **camera mapping** (projetar a pintura na geometria simplificada) [S: tutorial "3D Anime Backgrounds in Blender … camera mapping"; talk Blender Conference 2025 "Reimagining Iconic Anime Backgrounds in 3D" (Akira, Eevee, sem Grease Pencil)]. Recursos: nó **BackgroundCard** (textura, escala em pixels/m, *sempre voltado à câmera* opcional, unlit ou cel, sem receber sombra), **Parallax por camada** (profundidade → deslocamento), e **Camera projection** (projetar uma imagem a partir da câmera ativa sobre malhas — Fase 3).
4. **Layout 3D tipo Clip Studio** [S `tips.clip-studio.com/articles/16937`]: planejar quadro com cenário 3D, **plano próximo (near) para paredes sumirem/ficarem transparentes** quando a câmera está dentro do cômodo, **projeção paralela** (verticais continuam verticais ao inclinar a câmera), régua de perspectiva ligada ao 3D. → *Near clip por câmera*, *ortográfica real* e *auto-transparência de paredes entre câmera e personagem* (flag por nó: "ocultar se ocluir a câmera").
5. **Props instanciados e dressing** (cadeiras, livros, folhas, postes): **scatter por pincel** à moda Foliage Mode [V] (raio, densidade, escala min/max, *align to normal*, filtro de superfície, *Erase* com Shift) + instâncias baratas (um nó `InstanceSet` com N transformações, 1 draw). Em anime o scatter é mais **dirigido** (poucos objetos bem postos) que denso.
6. **Iluminação por cena/plano**: a luz de anime é *dirigida* (key + rim + hue-shift de sombra) e muda por plano. → **Light Rig presets** (Dia limpo, Entardecer, Noite com lua, Interior quente…), 1 `StylizedLight` principal por cena + luzes de preenchimento só-cor, **ambiente hemisférico** (já em `StylizedLight.ambient_sky/ground`), e **Post Process Volume**-like (volume com prioridade/peso/unbound [V]) para trocar paleta/bloom por área ou por shot. Sem bake.
7. **Sets modulares / kit-bash**: kits de parede/piso/porta/janela/teto numa **grade** (módulo de 1 m/0.5 m), com *snap por vértice/pivô nos cantos* (UE [V]) **e snap por sockets** (inexistente no UE): cada asset do kit declara sockets (`socket:wall_left`, `socket:floor_corner`, com *tipo* e *direção*) → ao arrastar, a peça "imanta" ao socket compatível mais próximo. Mais **GridMap-like paint** (Godot [V]) para pisos/paredes.
8. **Camadas de cena** (substituem Layers/Data Layers do UE): `BG / MG / FG / Personagens / FX / Guias`; visibilidade, lock, **render layer** (para exportar passes separados), *holdout vs hidden* (o tutorial de layered rendering da Epic usa Actor Layers `MG_ENV`, `FG_ENV`, `FX`, `CHAR` e dois modos: Holdout e Is Hidden [S]). **Variações** (dia/noite/chuva) como sets de overrides — modelo Prefab Variant do Unity [V].
9. **Proporção e escala** corretas para personagens anime (cabeça 1/7–1/8, porta ~2,0 m…): grade em **metros**, réguas e uma **silhueta de escala** (manequim/ator de referência) para checar o set.
10. **Outline/cel no viewport**: o preview deve ser o mesmo do render final (regra já do projeto: `celview.rs`); o Cenário precisa alternar **Cel / Workbench (sólido) / Wireframe / Só linhas / Unlit(cor base) / Luz** — análogo a Lit/Unlit/Wireframe/Detail Lighting/Lighting Only [V], com atalhos Alt+2…6 reaproveitados.

---------------------------------------------------------------------------------------------------

## 11. PROPOSTA DE ESCOPO em 3 fases — workspace Cenário

### 11.0 Princípios e estrutura de dados

**Fonte única**: continuar com `ProjectState.scene: SceneState` do `anigo-core` (invariante: toda alteração persistente é `Command`). O "SceneDocument" pedido é, na prática, **a evolução de `SceneState/NodeSlot`**, não um tipo paralelo; criar um tipo novo duplicaria a autoridade (o doc de `NodeSlot::parent_id` já justifica "uma só fonte de topologia").
Extensões aditivas (`#[serde(default)]`, compatíveis com projetos antigos):

```text
NodeSlot (existente)  + locked: bool            // cadeado: não seleciona/transforma
                      + tags: Vec<String>       // também filtráveis no Outliner
                      + folder: Option<FolderId>// pastas do Outliner (organização, não transformação)
                      + layer: LayerId          // BG/MG/FG/CHAR/FX (render layer)
                      + source: Option<NodeSource>   // de onde veio
                      + overrides: NodeOverrides     // material slots, flags de render por instância
                      + sockets_override: Vec<SocketDef> (opcional)
NodeSource = LibraryAsset{ asset_id: AssetId(uuid v7 da Biblioteca), revision: Blake3 }   // MeshRef já existe p/ malhas
           | SetInstance{ prefab_id, variant: Option<VariantId> }                         // prefab de cenário (Fase 2)
           | Primitive(Cube|Sphere|Plane|…) | Light | Camera | Card{texture} | InstanceSet
Folder{ id, name, parent }          Layer{ id, name, visible, locked, render_pass }
Camera: + sensor{w,h}, focal_mm, near/far, projection{Persp|Ortho}, shift?   (já existe `Camera` em math.rs; estender)
SceneSettings: grid{unit, size, major}, snap{pos_step, rot_step_deg, scale_step, surface, vertex, socket}
Bookmarks: [Option<CameraPose>; 10]   // vão no session.json (é do usuário, não do projeto) ou no projeto, a decidir
```
* Reconciliar **ids**: o core usa `AssetId` string-slug (`AssetId::for_uri`) e `anigo-assets` usa UUID v7 → referenciar a Biblioteca por URI `anigo://library/<uuid>` (o mesmo padrão de `anigo://authored/<id>`) e resolver via `Library::resolve_files` [C — não conferi o resolver; `BIBLIOTECA_UI.md` §7 o cita].
* **ActorId = `NodeId`** (já estável, serializado).
* **Undo por transações**: `CommandHistory` já guarda `inverse`. Regras: (a) **arrasto de gizmo/fly** = estado transitório fora do histórico, e **um** `SetNodeTransform` (ou `Batch` p/ multisseleção) no *soltar*, com o `before` capturado no *pegar*; (b) pincel (scatter) = um `Batch` por traço; (c) criar+parentear+nomear ao soltar um asset = um `Batch` (1 Ctrl+Z desfaz tudo); (d) comandos novos: `AddNodes`, `RemoveNodes` (devolvem inverso completo, inclusive filhos), `SetNodesTransform`, `SetNodeLocked`, `RenameNode`, `SetNodeTags`, `DuplicateNodes`, `SetNodeFolder/Layer`, `SetSnapSettings` (não vai ao histórico); (e) **painel Undo History** lista `HistoryEntry.description` e permite pular para qualquer ponto aplicando inversos em lote (UE tem o painel [V]).
* **Autosave/Recovery**: já há `session.json` + `CommandLog`; autosave = salvar projeto a cada N min (config. como *Frequency in Minutes* do UE [V]) + recuperar reaplicando `CommandLog` após crash.

### 11.1 FASE 1 — MVP "montar um cenário e salvar" (≈ o que o Blender/UE básico dá)

| Feature | egui / wgpu | Estrutura | Algoritmo | Validável na nuvem? |
|---|---|---|---|---|
| Viewport com orbit/pan/zoom/**F**/ortho (Alt+G/H/J/K) | reuso `OrbitCamera`; egui capta input | — | já existe (`frame_bounds`, `ViewPreset`) | ✅ unit + screenshot; sensação ⛔ local |
| **Fly RMB+WASD/QE**, velocidade (RMB+roda), **distance-based speed** | egui lê teclas/ptr; sem grab de ponteiro | `FlyCamera{pos,yaw,pitch,speed}` (ou modo da OrbitCamera) | integração por dt; `speed = base·scalar·(dist^k)` | ✅ lógica (unit) e xdotool (keydown); sensação ⛔ |
| Modos de vista Cel / Sólido / Wire / Unlit (Alt+2..6), **G** game view, grade métrica | reuso Workbench + `celview`; grade já existe | `ViewSettings` | — | ✅ screenshot |
| **Soltar da Biblioteca no viewport** (+ prévia "ghost") | egui `dnd_hover_payload::<AssetDragPayload>()`/`dnd_release_payload` (já planejado em `BIBLIOTECA_UI.md` §7); Biblioteca ao lado do viewport (painel); prévia = nó temporário renderizado com `encode_scene` | `Batch[AddNode{source:LibraryAsset}]` | raio da câmera → hit mais próximo (ou plano do chão) → posição = hit + normal·offset; pé do objeto = `bbox.min_y` do `AssetStats.bbox` | ✅ lógica; DnD via xdotool (é DnD interno do egui, só ponteiro) ✅ provável; prévia visual ✅ screenshot |
| **Seleção**: clique, Ctrl/Shift, **marquee**, Esc, Ctrl+A, inverter, filhos, mesma malha/mesmo asset | picking CPU; retângulo = egui painter | `Selection{ids, active}` (UI, não no projeto) | clique = raio×AABB→raio×triângulos; marquee = projetar AABB 8 cantos → retângulo∩ | ✅ unit; ✅ xdotool |
| **Contorno de seleção** | fase 1: AABB/wire no painter (barato); ideal: passe wgpu | — | AABB projetada; (ideal) ID-buffer + sobel | ✅ screenshot |
| **Gizmo W/E/R** (eixo, **plano**, uniforme; World/Local) | painter para desenhar (já faz); matemática pura | `GizmoState{mode, space, active_handle}` | arrasto de eixo: ponto da reta-eixo mais próximo ao raio (retas reversas); plano: raio×plano; rotação: ângulo entre vetores no plano do anel; escala: razão de distâncias | ✅ unit (determinístico), ✅ xdotool |
| **Snap**: grade (pos/rot/escala), Ctrl inverte, **End** (snap ao chão), **Z** (superfície + alinhar ao normal), barra de snap | egui | `SnapSettings` | `round(x/s)·s`; End: raio para baixo a partir de 5 pontos da base (centro+cantos) ⇒ menor distância; Z: raio sob o cursor excluindo a seleção + `Quat::from_rotation_arc(up, n)` mantendo yaw | ✅ unit; ✅ screenshot |
| **Outliner**: árvore (parent via arrastar), olho, cadeado, busca (`-x`, `+x`, `"x"`), duplo-clique renomear, F | egui `CollapsingHeader`/DnD | `locked`, `folder` | filtro por tokens (sintaxe do UE [V]) | ✅ |
| **Details**: nome, Transform numérico (drag value, reset, cadeado de escala, relativo↔mundo), tags, visível/lock, material slot | egui `DragValue` | — | decomposição TRS; edição multiselecção "Multiple Values" | ✅ |
| **Edição**: H/Ctrl+H, Del, **Ctrl+W**, **Alt+arrastar**, Ctrl+C/V, Ctrl+G | — | `DuplicateNodes` | clonar subárvore com novos `NodeId` | ✅ |
| **Undo/Redo** por transação + painel histórico, autosave, restore | egui | `CommandHistory` existente | ver 11.0 | ✅ unit (fuzz de undo) |
| **Câmera de cena** com focal (mm) + sensor + **guias de composição** (terços, centro, safe area, moldura do aspect) | overlay painter | `Camera{sensor,focal_mm}` | `fov = 2·atan(h/2f)` | ✅ screenshot |
| Luz principal (`StylizedLight`) editável no Details + preset de rig | egui | já existe | — | ✅ |

**Critério de pronto da Fase 1**: arrastar 5 ativos da Biblioteca, posicioná-los no chão com snap, agrupar/duplicar, trocar a câmera de lente, desfazer/refazer tudo, fechar/abrir o app e voltar ao mesmo estado.

### 11.2 FASE 2 — Completo (nível "UE Level Editor" para cenas de anime)

| Feature | egui / wgpu | Estrutura | Algoritmo | Nuvem? |
|---|---|---|---|---|
| **Vertex snap (V)** + pivô temporário (Alt+MMB) + **Pivot Offset** permanente + Snap Origin/Pivot/Bounds to Floor, Align to Actor, Move Object↔Camera | — | `pivot_offset: Vec3` no nó | candidatos = vértices dos triângulos nas folhas BVH mais perto do cursor (raio de tela ≤ N px); mover para que `vértice_fonte`(do pivô) = `vértice_alvo` | ✅ unit |
| **Snap Base/Target à la Blender** (Closest/Center/Median/Active × Vertex/Edge/Face/Grid) | UI de menu | `SnapSettings{base,target[]}` | variantes acima | ✅ |
| **Snap de socket** (kit modular) | — | `SocketDef{name, kind, local: Transform, gender?}` em manifesto do asset (`anigo-assets`) ou nós `socket:*` no glTF | candidatos = sockets compatíveis a ≤ R do socket arrastado; `T_peça = T_alvo · flip · T_socket⁻¹`; histerese para não "tremer" | ✅ unit |
| **Bookmarks** Ctrl+1..9 / 1..9; câmera piloto; **Preview Selected Camera** (PiP) | segundo render-to-texture pequeno | `Bookmarks` | — | ✅ screenshot |
| **Prefabs de cenário** (grupo salvo na Biblioteca com variantes/overrides — modelo Unity [V]) e **Level Instance-like** (editar em contexto, Commit/Esc Esc [V]) | modo de edição "em contexto": demais nós acinzentados (flag no render) | `SetAsset{nodes, local ids}`, `SetInstance{prefab_id, variant, overrides}` | resolução = prefab + overrides (merge por NodeId local); Break = expandir | ✅ unit; visual ✅ |
| **Instanciação** (props repetidos) + **Scatter/Foliage brush** (raio, densidade, escala min/max, align to normal, Shift apaga, filtro de superfície) | renderer precisa de **instanced draw** (hoje 1 draw por nó [C: conferir]) | `InstanceSet{asset, transforms}` | amostragem de Poisson-disc dentro do círculo da tela projetado na superfície (raio×BVH), rejeitar por raio mínimo (Radius [V]) | ✅ lógica; perf ⛔ local |
| **Substituir** selecionados por outro asset (preservar escala), **Merge/Bake** (fundir malhas estáticas; `meshopt` já existe) | — | comando `ReplaceNodesAsset` | — | ✅ |
| **Camadas BG/MG/FG**, visibilidade/lock/render layer, **Variações** (sets de overrides) | — | `Layer`, `Variant` | — | ✅ |
| **Luzes**: Directional/Point/Spot/Rect-área (estilizadas), Sky/hemisfério, **Post/Look Volume** (prioridade, peso, unbound [V]) | renderer: luzes extras exigem uniform array + shader (`render_contract` congelado → mudança de contrato/hash!) | `LightSlot` já existe | blend de volumes por AABB: peso = smooth(1 − dist/blend_radius) · blend_weight, ordenar por prioridade | ✅ lógica; **contrato de render** = decisão do projeto |
| **Outliner**: pastas, filtros por tipo/tag/camada, **filtros salvos** [V], Info columns, F sincronizado | egui | `Folder` | — | ✅ |
| **Cards de fundo / billboards**, parallax, near-clip por câmera, **ortográfica real**, "ocultar se ocluir câmera" | quad + textura; unlit/cel | `Card{texture, size_m, facing}` | — | ✅ screenshot |
| **Modos de seleção avançada**: Select Matching, Invert, filhos/ascendentes, Shift+E, Ctrl+Shift+A | — | — | — | ✅ |
| **Preferências** (grades, WASD, sensibilidade, atalhos remapeáveis, importar/exportar) [V UE] | egui | `Prefs` em `session`/config do usuário | — | ✅ |

### 11.3 FASE 3 — Avançado

* **Grid paint modular (GridMap-like)** [V Godot]: paleta de peças, cursor rotacionável (X/Y/Z), "andar" (Ctrl+roda), Pick, Fill, Selection+Move/Duplicate — gerando `InstanceSet`s por célula. ⬛ nuvem: lógica ✅, sensação ⛔.
* **Auto-snap por grafo de sockets** (propor a próxima peça compatível; "conectar tudo").
* **Camera projection mapping** (projetar pintura no cenário) e **parallax por camada** com pré-visualização do plano final.
* **Terreno**: reaproveitar o motor de escultura do `anigo-modeling` como "Landscape Sculpt/Paint" simplificado (heightfield + camadas de cor) [V conceito UE]. **Splines** (cerca/estrada/trilho) com instanciação ao longo da curva.
* **Picking por ID-buffer na GPU** para cenas grandes (wgpu: render de `R32Uint` + leitura assíncrona `map_async`) e **TLAS** de cena; **HLOD-lite** (fundir instâncias distantes) — só se a contagem de nós justificar.
* **Level Instance completo** (propagação, Actor Filters equivalentes, Property Overrides), **Data-Layer-like**: variantes dia/noite por plano com troca de paleta de post.
* **Layout sheet/export** (planilha de câmeras/planos com miniaturas), integração com Animação/Render (cada shot = câmera+luz+camadas+personagens).
* **PCG-lite** (regras: "cadeiras em torno de mesa") — apenas citar; **Water/Sequencer** — fora de escopo.

### 11.4 Algoritmos-chave (resumo técnico)

**Raycast / colocar em superfície com BVH** — hoje `viewport/picking.rs::raycast_mesh` é linear em triângulos de **uma** `EditMesh`. Para cena: (1) **BLAS por asset**: BVH de triângulos no espaço local, construído uma vez e cacheado por *hash de conteúdo* do asset (mesma ideia do DDC [S `biblioteca_assets.md`]); build binned-SAH (16 bins), nós de 32 B, folhas ≤ 4 triângulos; travessia por pilha com *slab test* ordenado por distância; (2) **TLAS**: BVH sobre os AABBs mundiais dos nós (rebuild barato a cada commit; durante arrasto, refit); (3) raio: mundo → espaço local da instância (inversa da matriz mundial), `t` é invariante se a direção não for normalizada; (4) ignorar o subgrafo em arrasto (máscara por NodeId) e nós `locked`/ocultos. Crates a avaliar: `bvh` ou `parry3d` (BVH/queries) — **[C], não verificados**; escrever o BVH próprio (~300 linhas) mantém o projeto sem dependência. Fallback sem hit: `camera::intersect_plane` no plano do chão (já existe).
**Colocação**: `p = hit.point + hit.normal·surface_offset`; `R = align_to_normal ? from_rotation_arc(Y, n)·yaw : yaw`; compensar a base do asset: `p += R·(0, −bbox.min_y·scale, 0)`. **End (snap ao chão)**: raios para −Y a partir de centro e 4 cantos da base da bbox mundial; usar o **menor deslocamento** (a peça "senta" sem atravessar).
**Snap de grade**: posição em espaço mundo (ou local, se o gizmo for Local): `snap(v)=round(v/step)·step`; com *Absolute* vs *Increment* (Blender [V]): incremental = snap do **delta** a partir da pose original; absoluto = snap da pose final. Rotação: snap do **ângulo acumulado** do arrasto (não do euler resultante). Escala: aditiva `s0 + round(Δ/step)·step`.
**Vertex snap**: da folha BVH sob o cursor (ou vizinhas até raio de tela R) coletar vértices; escolher o de menor distância **em pixels** ao cursor; destacar com ponto no painter; mover `fonte → alvo`.
**Gizmo (já parcialmente em `app/gizmo.rs`)**: manter tamanho em pixels (`world_size`), *hit-test* em espaço de tela (distância ponto-segmento já existe), arrasto por **reta-reta mais próxima** (eixo) / **raio-plano** (plano/rotação), acumulando o delta desde o *pegar* (evita deriva); espaço Local usa os eixos do nó, Parent usa os do pai (`hierarchy::resolve_world_transforms`).
**Hierarquia**: `SetNodeParent` com `reparent_checked` (rejeita ciclos — já há testes no core); ao mover pai, filhos seguem via `W(child)=W(parent)·T(local)` [V `hierarchy.rs`]. *Attach mantendo a pose mundial* = recalcular local com a inversa do pai.

### 11.5 O que depende de egui/wgpu (resumo) e o que é validável onde

* **Puro (sem egui/wgpu) — testes unitários na nuvem, determinísticos**: snap, matemática de gizmo, BVH/raycast, seleção por marquee (projeção), filtros do Outliner, comandos/undo (fuzz: aplicar N comandos, desfazer tudo, comparar hash do estado), prefabs/overrides, scatter (semente fixa), câmera (focal↔FOV).
* **egui (UI)**: Outliner, Details, painéis, overlays (gizmo, marquee, guias de composição, grade rotulada), DnD interno de cards → viewport. Validável na nuvem via **Xvfb + lavapipe + xdotool** [V `ARQUITETURA_NATIVA.md` §4 e `VALIDACAO_LOCAL.md`: o app roda sob Xvfb, `--screenshot`/`--scenario`, e xdotool dá entrada real]: cliques, arrastos, teclas (incluir `keydown w` com RMB pressionado para o fly), **screenshots determinísticos por `--scenario`** (a convenção já existe em `app/automation.rs`: criar `cenario-*`: `cenario-vazio`, `cenario-gizmo-mover`, `cenario-snap-chao`, `cenario-marquee`, `cenario-guias`, `cenario-drag-biblioteca`).
* **wgpu (render)**: contorno de seleção por passe, prévia "ghost" translúcida, instanced draw, passe de ID-buffer, luzes extras (mexem no `render_contract_v1.json`/hashes — **decisão de contrato**; os golden frames só valem em lavapipe, [V] `VALIDACAO_LOCAL.md` §2.6). Imagem validável por screenshot na nuvem; **desempenho (FPS, nº de nós) só no PC com GPU real** (`VALIDACAO_LOCAL.md` §2).
* **Só local (humano/hardware)**: sensação do fly e do orbit (aceleração, "pulo"), caneta/trackpad (gestos do UE para Magic Mouse não se aplicam), DPI alto/multi-monitor, IME/acentos nos campos de nome, drag-and-drop de **arquivos do SO** para importar, FPS com milhares de instâncias, gosto visual (espessura de contorno, cor de seleção, tamanho dos pontos).

### 11.6 Riscos / decisões abertas
1. **Luzes extras vs `render_contract` congelado**: Fase 2 exige decidir se o contrato evolui (v2) — afeta CI de hash.
2. **Instanciação**: confirmar se `encode_scene` suporta N instâncias de uma malha sem N draws (não verifiquei o renderer).
3. **IDs de ativo** (slug do core × UUID v7 da Biblioteca): definir a ponte (`anigo://library/<uuid>`) antes de gravar `NodeSource` em projetos.
4. **Escala de unidades**: UE usa cm; ANIGO usa `LengthUnit` em `ProjectSettings` — grades e sensibilidade de câmera devem derivar dela (metros por padrão).
5. **Atalhos**: manter coerência com Modelagem (Blender-like: MMB orbit, G/R/S) **vs** UE (RMB fly, W/E/R). Proposta: Cenário usa esquema **UE** (W/E/R, RMB-fly, F, End, Ctrl+W), Modelagem mantém Blender; mapa de atalhos remapeável (como Keyboard Shortcuts do UE [V]).

---------------------------------------------------------------------------------------------------

## 12. Fontes

[V] lidas por inteiro (UE 5.8 salvo nota):
* https://dev.epicgames.com/documentation/unreal-engine/viewport-controls-in-unreal-engine
* https://dev.epicgames.com/documentation/unreal-engine/transforming-actors-in-unreal-engine
* https://dev.epicgames.com/documentation/unreal-engine/actor-snapping-in-unreal-engine
* https://dev.epicgames.com/documentation/unreal-engine/placing-actors-in-unreal-engine
* https://dev.epicgames.com/documentation/unreal-engine/viewport-toolbar
* https://dev.epicgames.com/documentation/unreal-engine/viewport-modes-in-unreal-engine
* https://dev.epicgames.com/documentation/unreal-engine/outliner-in-unreal-engine ; 4.27: `.../world-outliner` e `.../world-outliner-ui`
* https://dev.epicgames.com/documentation/unreal-engine/level-editor-details-panel-in-unreal-engine
* https://dev.epicgames.com/documentation/unreal-engine/actor-mobility-in-unreal-engine
* https://dev.epicgames.com/documentation/unreal-engine/level-instancing-in-unreal-engine
* https://dev.epicgames.com/documentation/unreal-engine/foliage-mode-in-unreal-engine
* https://dev.epicgames.com/documentation/unreal-engine/unreal-editor-preferences
* https://dev.epicgames.com/documentation/unreal-engine/recovery-hub-in-unreal-engine
* https://dev.epicgames.com/documentation/unreal-engine/light-types-and-their-mobility-in-unreal-engine (trecho longo da busca)
* https://dev.epicgames.com/documentation/unreal-engine/landscape-overview (trecho longo da busca)
* https://docs.blender.org/manual/en/latest/editors/3dview/controls/snapping.html (Blender 5.2 LTS)
* https://docs.unity3d.com/Manual/PrefabVariants.html (Unity 6.6)
* https://docs.godotengine.org/en/stable/tutorials/3d/using_gridmaps.html (Godot 4.7)

[S] trechos de busca / secundárias:
* https://www.unrealengine.com/tech-blog/designer-s-guide-to-unreal-engine-keyboard-shortcuts (tech blog Epic, 2019, UE4 — lido por inteiro, mas é artigo de terceiro hospedado pela Epic e de versão antiga)
* https://dev.epicgames.com/community/learning/tutorials/yp8j/the-unexplored-unreal-engine-keyboard-shortucts (tutorial da comunidade, lido por inteiro)
* https://www.domestika.org/en/blog/4733-basic-keyboard-shortcuts-for-unreal-engine-4-and-5 · skillademia.com/shortcuts/unreal-engine-shortcuts · evercast.us/blog/unreal-engine-shortcuts · defkey.com/unreal-engine-shortcuts
* https://dev.epicgames.com/documentation/unreal-engine/layers-panel-in-unreal-engine · `.../world-partition---data-layers-in-unreal-engine` · `.../world-partition-in-unreal-engine` · `.../actor-editor-context-in-unreal-engine`
* https://dev.epicgames.com/documentation/unreal-engine/merging-actors-in-unreal-engine · `.../modeling-mode-in-unreal-engine` · `.../instanced-static-mesh-component-in-unreal-engine`
* https://dev.epicgames.com/documentation/unreal-engine/post-process-effects-in-unreal-engine · `.../cinematic-cameras-in-unreal-engine` · 4.27 `cinematic-depth-of-field-method`
* https://dev.epicgames.com/community/learning/knowledge-base/r6wl/unreal-engine-world-building-guide (LI/PLA/OFPA)
* https://dev.epicgames.com/community/learning/tutorials/pYww/layered-rendering-in-unreal-engine-part-3-a-step-by-step-production-workflow (camadas/holdout)
* https://dev.epicgames.com/community/learning/tutorials/v78w/unreal-engine-5-local-lights-performance-breakdown-spot-vs-point-vs-rect-light
* https://tips.clip-studio.com/en-us/articles/16937 (layout 3D de mangá: near plane, ortográfica, régua de perspectiva)
* https://conference.blender.org/2025/presentations/3988/ ("Reimagining Iconic Anime Backgrounds in 3D"); YouTube "The ABSOLUTE BEST Way to Create 3D Anime Backgrounds in Blender" (camera mapping) — apenas títulos/descrições.

Não encontrados / não verificados (portanto [C]): página oficial única de atalhos do level editor; Show Flags; Mesh Paint; Spline; Sky Atmosphere; Lumen (além de menções); Paste Here/Duplicate Special; valores padrão dos arrays de grade; pastas/lock no Outliner 5.x; ProBuilder; Blender Collections/Local View; crates de BVH (`bvh`, `parry3d`).
