# Pesquisa — workspace Posing (posar personagens 3D de anime)

**Data:** out/2026. **Escopo:** como as ferramentas de referência modelam, editam e guardam uma *pose*; limites de movimento
articular; IK de membros; olhar; pés no chão; centro de massa; biblioteca de poses. Fecha com uma **proposta de escopo em fases**
e a divisão nuvem × local (seção 8). Código novo: `anigo-core/src/pose.rs`, `anigo-ik`, `anigo-assets` (tipo `Pose`), `anigo-studio` (`posing_ui`).

Legenda de confiança: **[V]** documentação oficial lida nesta sessão · **[S]** fonte secundária (tutorial, fórum, snippet de busca) ·
**[C]** conjectura/convenção minha, a validar com artista. Nada abaixo é citado de memória como se fosse fato.

## 1. Blender (Pose Mode) — a referência de fluxo

* **Pose = offset sobre o repouso.** Em Pose Mode cada osso tem posição/rotação/escala *neutras* no repouso (0/0/1) e editar um osso cria um
  *offset* sobre ele; o repouso só muda em Edit Mode **[V]** (`animation/armatures/posing/introduction`). → modelo ANIGO: a pose guarda **delta de
  rotação local por osso** (identidade = repouso) + deslocamento de translação do root/quadril; o repouso nunca é tocado.
* **Cor do osso por estado** (cinza padrão, contorno azul em Pose Mode, verde com constraint, amarelo com IK, laranja IK sem alvo) **[V]**. → overlay
  do ANIGO: osso neutro, selecionado, hover, e osso "em IK" com cor própria.
* **IK Solver constraint**: *Target*, *Pole Target* ("determina o roll da cadeia"; no braço, a posição do cotovelo), *Chain Length* (nº de ossos
  afetados a partir do osso dono; 0 = até a raiz), *Iterations*, *Stretch* (só em ossos com `IK Stretch > 0`), pesos de posição/rotação,
  *Influence*; o IK "pode funcionar sem alvo" e roda **depois** das demais constraints **[V]**. Painel IK do osso: *Lock*, *Stiffness*, *Limit* por eixo **[V]**.
  Aviso explícito: com Pole Target o *lock* de IK não funciona no osso-raiz da cadeia **[V]**. → ANIGO: cadeia de 2 ossos (braço/perna) com pole,
  sem stretch por padrão, e limites aplicados à saída do IK (não "lock").
* **Auto IK** (ferramenta de pose rápida): transladar um osso rota os pais; só estende por ossos *conectados*; Page Up/Down ou roda mudam o
  comprimento da cadeia **[V]**. → o arrasto do handle de IK do ANIGO tem comprimento fixo por membro (2 ossos) e um modo "corpo" (FABRIK, cadeia longa).
* **X-Axis Mirror / Relative Mirror**: com nomes `.L/.R`, transformar um osso transforma o "outro lado" por simetria no eixo X local do armature **[V]**
  (`posing/tool_settings`). → toggle **Simetria (X)** do ANIGO: editar um osso aplica o espelhado no par.
* **Copy / Paste / Paste Flipped** (`Ctrl+C`, `Ctrl+V`, `Shift+Ctrl+V`): copia só os **ossos selecionados**, cola **por nome** (selecionado na colagem não
  importa), um único buffer de sessão, copia rotação/posição/escala "no espaço do próprio osso" — o resultado depende do repouso e da pose dos pais **[V]**
  (`posing/editing/copy_paste`). → ANIGO: o mesmo contrato (por nome, só selecionados, buffer de sessão), mais a opção "colar espelhado".
* **Pose Library = Asset Browser** (Blender ≥ 3.0): um *pose asset* guarda as transformações **dos ossos selecionados** na criação; *Apply Pose* aplica só
  aos ossos selecionados (permite "dedo apontando" = punho + índice/polegar abertos), *Apply Pose Flipped* espelha esquerda↔direita (uma pose de mão
  esquerda vira direita), *Blend Pose* mistura com arrasto (pode subtrair e extrapolar com `E`), com miniatura gerada na criação **[V]**
  (`posing/editing/pose_library`). → ANIGO: ativo `.anigopose.json` guarda **só os ossos incluídos** (máscara), aplica por seleção, "aplicar espelhado" e
  "misturar" (slider 0–100 %) usando a mesma API que a Animação usará.

## 2. Clip Studio Paint (figura 3D), DesignDoll, PoseMyArt — o que o artista de mangá/anime espera

* **CSP** [S: `tips.clip-studio.com/en-us/articles/6967`, texto lido]: três modos — arrastar uma parte do corpo (a força propaga até o quadril, que não se move);
  **FK** (hierarquia: mexer no ombro leva braço e dedos) com **limites de ângulo "humanamente apropriados"** (não dá para quebrar a junta);
  **IK** com manipuladores nos pontos terminais, onde "você pode sobrescrever qualquer junta bloqueada ou limite". Há *Lock selected joints*,
  **simetria** que reflete a pose, **Pose Hand** (cruz 2D que interpola presets: mão aberta/dedos fechados/punho, com 4 posições de polegar),
  *Register full body pose as material* e arrastar uma pose da biblioteca/loja sobre o personagem para aplicar, mais um modo de mover o objeto
  deslizando no plano do chão. → decisões: (a) FK limita por padrão, IK pode exceder (toggle global "limites" + 3 perfis); (b) simetria e
  copiar/colar são de primeira classe; (c) mão = presets numa cruz/slider, não 15 sliders; (d) salvar pose = ativo da Biblioteca.
* **DesignDoll** [S: `terawell.net/en/page-news.php`, snippets]: histórico de versões menciona "controlador de rotação do cotovelo/joelho
  acessível logo após posicionar mão/pé" (o *pole/roll* vem colado ao handle do efetuador), **Hand Library** e **Hand Presets**, e relaxamento das
  restrições de tornozelo. → o handle de IK do ANIGO tem um *pole* visível junto do efetuador, e a mão tem biblioteca/presets próprios.
* **PoseMyArt** [S: `posemy.art/blog/design-doll-alternative`]: IK para velocidade ("arraste as caixas cinza"), FK para precisão ("clique nas esferas de
  rotação de cada eixo"), 5800+ poses prontas. → confirma o par FK/IK por membro com toggle.
* **VRoid Studio / VRM** [V: `VRMC_vrm-1.0/tpose.md`, `humanoid.md`]: rest pose = **T-pose** olhando +Z, simétrica em X, braços ao longo de X, pés paralelos a +Z,
  solado em Y=0, *hips* é o osso que se translada ("usually only this bone moves. Other bones rotate only"); escala uniforme positiva em todos os nós. →
  `PoseState` translada **só** root/hips; todos os outros ossos só giram (pureza SE(3) já exigida por `bone_sync`). **Achado do repo:** o esqueleto canônico
  `BondSyncManager` está em T-pose e é mais baixo que a malha canônica (junta da cabeça a ~1,49 m contra centro de cabeça da malha masculina em 1,68 m), enquanto a **malha tem os braços pendurados** (A estendido, `mesh.rs`) — por isso o
  rig de posing é **ajustado à malha** (seção 6), não copiado do esqueleto T.
* **Mixamo** [S: `mixamo.com`, Adobe Help]: autorrig + biblioteca de milhares de animações *transferidas por nome de osso* para o esqueleto do usuário,
  exportação FBX. → o ativo de pose do ANIGO também é **por nome de osso** (24 nomes canônicos VRM), o que permite retarget trivial e é a ponte que a Animação usa
  para keyframes (uma animação = sequência de poses).

## 3. Unreal Control Rig / Maya — IK de membro e pole vector

* **Unreal `Basic IK` (two-bone)** [V: `node-reference/ControlRig/BasicIK`, UE 5.8]: "resolve o IK de dois ossos dados dois ossos", **opera em espaço de mundo**;
  entradas `ItemA/ItemB/EffectorItem`, `PrimaryAxis` (eixo ao longo do osso) e `SecondaryAxis` ("em direção ao pole vector") com `SecondaryAxisWeight`,
  `PoleVector` (**posição ou direção**, `PoleVectorKind`), `bEnableStretch` com `StretchStartRatio=0.75`/`StretchMaximumRatio=1.25`, `Weight`
  (quanto do IK aplicar), comprimentos opcionais (0 = pela hierarquia), `bPropagateToChildren`. → a API `anigo-ik::TwoBoneIk` copia esse contrato:
  `root, mid, end`, `target` (mundo), `pole` (ponto de mundo), `weight`, `stretch` opcional (desligado por padrão), comprimentos tirados do rig.
* **Maya** [V: `Pole Vector constraints`, guia 2014; S: fóruns]: o pole vector só existe no solver **Rotate Plane** (não no single-chain); *flipping* ocorre quando o
  vetor do handle se aproxima do pole — recomenda-se restringir o pole a um alvo posicionado onde o handle improvavelmente o cruza (para braço: **atrás** do personagem).
  → no ANIGO o pole é um ponto arrastável com posição padrão **à frente do joelho / atrás do cotovelo** e o solver trata o caso degenerado (alvo colinear ao pole) mantendo a pose anterior.
* **Two-bone IK analítico** [C: lei dos cossenos; coincide com o contrato do Unreal acima]: dado `a=|AB|`, `b=|BC|`, `d=|A→alvo|` limitado a `[|a−b|+ε, a+b−ε]`, o ângulo no
  vértice A é `acos((a²+d²−b²)/(2ad))`; o plano de dobra contém A, o alvo e o pole. Precisão numérica fechada (erro de comprimento ≈ 1e-6), sem iteração → ideal para arrasto ao vivo.
* **FABRIK** [V: `andreasaristidou.com/FABRIK`; Aristidou & Lasenby, *Graphical Models* 73(5), 2011]: "evita ângulos e matrizes; acha cada junta localizando um ponto numa reta", converge em poucas
  iterações, suporta restrições e múltiplos efetuadores. → usado na **cadeia longa** (quadril→coluna→pescoço→cabeça) onde o analítico não serve; os comprimentos dos segmentos são invariantes (testado).

## 4. Limites de rotação por articulação

Valores normativos de amplitude (graus), **[V]** Physiopedia "Range of Motion Normative Values" (que cita AAOS/AMA; página lida) com conferência cruzada **[S]** em tabelas AAOS de busca:

| Articulação | Movimento | Graus |
|---|---|---|
| Ombro | flexão 180 · hiperextensão 50 · abdução 180 · adução 50 · rot. interna 90 · rot. externa 90 | |
| Cotovelo | flexão 140–150 · extensão 0 | |
| Antebraço | pronação 80 · supinação 80 | |
| Punho | flexão 60 · extensão 60 · desvio radial 20 · ulnar 20 | |
| Coluna cervical | flexão 60 · extensão 75 · flexão lateral 45 · rotação 80 | |
| Coluna tóraco-lombar | flexão 45–50 · extensão 25 · flexão lateral 25 · rotação 30 | |
| Quadril | flexão 100–120 · hiperextensão 30 · abdução 40 · adução 20 · rot. interna 40 · rot. externa 50 | |
| Joelho | flexão 135–150 · extensão 0 | |
| Tornozelo | plantiflexão 40 · dorsiflexão 20–30 | |
| Dedos II–V | MCP flexão 80 · PIP 135 (até) · DIP 90 (até) | |
| Polegar | CMC abdução 45–70 · MCP flexão 50 · IP flexão 80 | |

Decisões **[C]** (a validar com artista):
* **Swing-twist, não Euler.** Limitar por componentes Euler tem *gimbal* justamente nas poses comuns (braço à frente ou em T). Cada osso tem um **eixo de torção** (Y local do repouso) e a rotação
  é decomposta em *swing* (cone elíptico com extensões assimétricas nos eixos X e Z) × *twist* (faixa em graus). É o modelo de limite do Blender-IK/UE ("cone constraint") sem a singularidade.
* **Distribuição na coluna/pescoço**: a amplitude total é repartida entre ossos (ex.: flexão tóraco-lombar 50° = Spine 20 + Chest 15 + UpperChest 15; rotação cervical 80° = Neck ±40 + Head ±40) para o total bater com a tabela.
* **Três perfis**: *Humano* (a tabela), *Anime* (×1,25 em swing/twist: o estilo exagera dobras e arqueamentos; mangaká "quebra" a anatomia por clareza) e *Livre* (sem limites — também é o comportamento do IK no CSP).
* O osso *Pelvis* (osso morto abaixo do quadril) e *Root* têm limites mínimos/livres; *Hips* é livre em perfil Anime e largo no Humano (sentar, deitar e agachar precisam de >90°).

## 5. Olhar, mãos, pés, equilíbrio

* **Olhar/look-at:** `anigo-ik::LookAtSolver` (já no repo) resolve yaw/pitch dos *olhos* com clamps físicos (45°/35°) e microssacadas; os olhos não são ossos da paleta (24 ossos congelados). O Posing usa o mesmo
  vocabulário para **cabeça e pescoço**: dado um alvo de mundo, calcula-se a rotação que leva o eixo +Z da cabeça ao alvo, repartida (≈ 35 % pescoço / 65 % cabeça) e limitada pelos limites de torção/swing. O
  *alvo de olhar* é um ponto arrastável em tela. **[C]** a repartição; **[V]** o contrato do solver de olhos existente.
* **Poses de mão:** o esqueleto canônico **não tem ossos de dedo** (24 ossos; paleta congelada em 1536 B) e a malha tem dedos como cilindros rígidos presos ao osso da mão. Portanto: o *dado* da pose de mão
  (curvatura 0–1 por dedo, 5 dedos × 2 mãos) entra no modelo e na biblioteca; o *efeito visual* nesta fase vem de uma deformação **CPU analítica dos vértices dos dedos** (articulada em 3 falanges) aplicada à malha
  antes do skinning, sem tocar a paleta nem o contrato. Quando o rig ganhar ossos de dedo (VRM os define: `humanoid.md` **[V]**), o mesmo dado passa a girar ossos. Ver riscos na seção 7.
* **Pés no chão / foot lock** [C, prática comum de ferramentas de pose]: (1) *auto-ground*: após qualquer edição, o root é deslocado em Y para que o ponto de contato mais baixo (tornozelo − altura do solado, ponta do pé) fique em y=0;
  (2) *foot lock*: ao mover o quadril, cada pé travado mantém sua posição de mundo no início do arrasto (two-bone IK por perna a cada passo) e a pose resultante é gravada em FK.
* **Equilíbrio / centro de massa** [S/C]: segmentos com frações de massa tipo Dempster (cabeça+pescoço ≈ 8 %, tronco ≈ 50 %, braço 2,7 %, antebraço 1,6 %, mão 0,6 %, coxa 10 %, perna 4,65 %, pé 1,45 % — tabela
  clássica citada em biomecânica, **não validada nesta sessão**). O CoM projetado no chão é testado contra a **envoltória convexa** dos pontos de contato dos pés (polígono de suporte); a UI mostra a projeção e um aviso
  "fora da base de apoio". É indicador, nunca bloqueia a edição.
* **Snap / auto-key:** snap de rotação por passo (5°/15°/45°) no gizmo FK. *Auto-key* (gravar keyframe a cada edição) é da Animação: o Posing só expõe a API de pose (seção 6) e o gancho `PoseChange` na `CoreSession`.

## 6. Modelo de dados decidido (resumo; detalhes e API em `POSING_UI.md`)

* `CharacterState.pose: PoseState` — campo **aditivo** (`#[serde(default, skip_serializing_if = "PoseState::is_rest")]`): projetos antigos, fixtures e JSON canônico de projeto em repouso **não mudam**.
  Guarda `rotations: BTreeMap<nome_do_osso, [x,y,z,w]>` (esparso, só ossos fora da identidade), `root_translation`, `hips_translation` e `hands` (curvatura dos dedos).
* **Um comando só**, `Command::SetPose` (antes de `Batch`, mesma posição em Rust e TS), com entradas por osso e o inverso calculado do estado. Mirror/Reset/Apply/Paste/Preset/IK/LookAt são
  *planejadores puros* que produzem um `SetPose` — igual ao padrão do Cenário (`level.rs`) — e `PoseOp` só rotula o histórico.
* **Arrasto = `CoreSession::preview`** (fora do histórico) e **um** `commit_preview` ao soltar (ou `cancel_preview` no Esc).
* `PoseRig` (calculado, não persistido): posições de bind dos 24 ossos **ajustadas à malha deformada** (cada junta canônica é arrastada pelos deslocamentos dos vértices vizinhos entre a malha
  de referência e a final — proporções + morphs), rotações de bind = identidade. Paleta = `mundo_posado · inverso(bind)`, rígida (SE(3)), plugada em `Scene.skin.palette`/`SkinPayload`.
  Com pose em repouso a paleta é **idêntica à identidade bit a bit** (o frame dourado e o contrato congelado não mudam).
* Ativo da Biblioteca: `.anigopose.json` (`format: "anigo.pose"`, `version: 1`, máscara de ossos, nomes canônicos, `rig_hint` da proporção de origem, miniatura PNG da pose), tipo de ativo `Pose` em `anigo-assets`.

## 7. Riscos e o que não dá para validar aqui

* Fatores de pesos de skin do mesh canônico são aproximados (duas influências, transições suaves); poses extremas (ombro a 170°, agachamento profundo) mostram *candy-wrapper* e colapso de axila inerentes ao LBS. O teste mede ausência de NaN,
  rigidez da transformação e deslocamento máximo plausível — **não** qualidade visual (isso é julgamento de artista; inspeção por captura Xvfb/lavapipe no sandbox).
* Calibração das juntas ajustadas à malha foi feita sobre as duas bases canônicas (masculina/feminina); morphs extremos podem deslocar a junta do centro anatômico (limitação do ajuste por vizinhos).
* Sensação do gizmo/IK com mouse e GPU reais, gosto de cores/espessuras do overlay, escala de pesos de cintura: **humano/local**.
* Valores de limites por osso e frações de massa são **[C]/[S]** até um artista/biomecânico revisar.

## 8. Proposta de escopo em fases e divisão nuvem × local

| Fase | Entrega | Onde se valida |
|---|---|---|
| **MVP** | `pose.rs` + `Command::SetPose` + `PoseRig` + paleta na `CoreSession` (cel real posado); overlay de esqueleto, seleção por clique/marquee, gizmo FK de rotação por osso (eixos locais/mundo), painel de osso (euler/quat, reset), reset osso/seleção/tudo, espelhar pose, undo/redo/histórico, presets de corpo (T, A, em pé, sentado, andando, ação) | nuvem: testes numéricos + Xvfb/lavapipe (screenshots inspecionados, smoke xdotool em Xvfb próprio) |
| **Completo** | IK de braços/pernas (two-bone + pole, toggle FK/IK por membro), simetria na edição, copiar/colar (+ espelhado), limites ligáveis (3 perfis) com visualização, look-at da cabeça com alvo arrastável, foot lock + auto-ground, CoM/polígono de suporte, biblioteca de poses (`.anigopose.json`, salvar da pose atual, aplicar de card, miniatura), presets e cruz de mão, snap de rotação | nuvem: idem; local: sensação e gosto |
| **Avançado** | FABRIK de coluna/pescoço (handle de cabeça arrasta o tronco), mistura de poses (*blend*, extrapolação), "aplicar só aos ossos selecionados", dedos articulados (CPU) no render; hooks de auto-key para a Animação | nuvem: testes; local: qualidade visual de dedos/pesos e desempenho com GPU real |
| **Fora de escopo agora** | ossos de dedo/olho na paleta (contrato congelado), retarget entre rigs diferentes, captura de movimento, física/ragdoll | — |

Fontes lidas ([V] salvo nota): docs.blender.org (posing/introduction, bone_constraints/inverse_kinematics/introduction, constraints/tracking/ik_solver, posing/tool_settings, posing/editing/copy_paste,
posing/editing/pose_library — por snippet [S] na íntegra do texto de busca), dev.epicgames.com (Control Rig › Basic IK), Autodesk Maya 2014 User's Guide (Pole Vector constraints), github.com/vrm-c/vrm-specification (tpose.md, humanoid.md/README por snippet),
andreasaristidou.com/FABRIK, physio-pedia.com (ROM normative values), tips.clip-studio.com (artigo 6967 de Manleonardo, [S]), terawell.net (DesignDoll, [S]), posemy.art ([S]), mixamo.com e helpx.adobe.com ([S]).
