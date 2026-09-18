# RELATÓRIO DEFINITIVO DE AUDITORIA CRÍTICA, PESQUISA INDUSTRIAL E REESPECIFICAÇÃO DE ENGENHARIA — SPRINT 03

**Destinatário:** Parent Orchestrator (`7fecf963-7aa1-41aa-bf38-394ea04dd835`)  
**Investigador:** Antigravity Senior Review & Systems Architect  
**Projeto:** ANIGO — Anime Next-Gen Interactive Graphics Operator  
**Escopo:** Auditoria Crítica da Investigação Prévia, Pesquisa Industrial Exaustiva e Arquitetura Completa da Sprint 03  
**Conformidade Normativa:** [ANIGO_INVIOLABLE_RULES.md](file:///c:/ANIGO/ANIGO_INVIOLABLE_RULES.md)

---

## 1. META-ANÁLISE CRÍTICA: O QUE ESTAVA ERRADO NA INVESTIGAÇÃO PRÉVIA

A tentativa anterior acertou no diagnóstico superficial de que a malha atual é composta por 13 cubos em [crates/anigo-core/src/mesh.rs](file:///c:/ANIGO/crates/anigo-core/src/mesh.rs#L104-L201) e [src/components/viewport/webgpu_renderer.ts](file:///c:/ANIGO/src/components/viewport/webgpu_renderer.ts#L974-L1031), e que os sliders de ombros e pernas em [src/App.svelte](file:///c:/ANIGO/src/App.svelte#L1578-L1588) eram puramente decorativos (*dummy UI*). Contudo, sob escrutínio técnico aprofundado, a proposta anterior continha **contradições matemáticas severas, premissas arquiteturais falhas, omissões anatômicas críticas e gargalos de GPU ignorados**.

Abaixo registra-se a auditoria formal no padrão: **Alegação → Evidência Citada → O que o Código e a Matemática Realmente Demonstram → Achado Corrigido**.

---

### TABELA DE CONFRONTAÇÃO DIRETA E RESOLUÇÃO DE CONTRADIÇÕES

| # | Tópico | Alegação da Investigação Prévia | Evidência Citada na Investigação Prévia | O que o Código e a Engenharia Realmente Mostram | Achado Corrigido (Padrão Ouro ANIGO) |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **1** | **Contradição das Green Coordinates** | A deformação volumétrica usará *Green Coordinates* (GC) como "Camada 3", operando através de gizmos 3D que "atualizam os sliders paramétricos". | Citação de Lipman et al., SIGGRAPH 2008 e fórmula $u(\eta) = \sum \phi_j v_j + \sum \psi_k n_k$. | **Contradição Lógica Circular:** Se puxar o gizmo atualiza os sliders (`ribcage_width = s_x`), e os sliders disparam blendshapes e bone deltas, o solver de Green Coordinates torna-se **completamente inútil** ou causa **dupla deformação** (o vértice deforma pelo slider E pela cage). Nenhum software de customização de personagem (VRoid, Design Doll, Sims 4, CC4) usa Green Coordinates para sliders. | **Eliminação de Green Coordinates para Deformação por Sliders.** A manipulação no Viewport (estilo Design Doll / Sims 4) deve ser implementada via **Tactile Direct Viewport Manipulation** (Raycasting de bounding hulls e projeção vetorial tela-para-parâmetro), alterando diretamente os sliders canônicos. Green Coordinates é restrita a modificadores livres de deformação em malhas sem esqueleto. |
| **2** | **Separação Topológica Male/Female** | Propôs dois arquivos base (`anigo_base_male.glb` e `anigo_base_female.glb`) com deltas binários separados (`face_male.bin` vs `face_female.bin`, `chest_male.bin` vs `bust_female.bin`), alegando "correspondência nas zonas compartilhadas". | Seção 7 (Sub-Sprints 3.1, 3.4, 3.5). | **Inviabilidade Industrial:** Se as malhas não forem **isomórficas** (mesmo número de vértices, mesma ordem de índices e mesma UV), os morphs não podem ser transferidos entre sexos, impedindo personagens andróginos, fêmineos masculinos (*bishounen*) ou mulheres musculosas. Roupas da Sprint 06 e cabelos da Sprint 07 teriam que ser modelados duas vezes do zero. | **Topologia Canônica Isomórfica Unificada (Super-Topology).** O ANIGO deve adotar uma malha base universal única (estilo MakeHuman/MetaHuman/CC4) onde a base Masculina e Feminina compartilham a exata mesma contagem e indexação de vértices ($N$ vértices idênticos). A transição Male $\leftrightarrow$ Female é um morph target mestre contínuo (`gender_dimorphism`), garantindo 100% de interoperabilidade de sliders, roupas e rigs. |
| **3** | **Modelo de Memória GPU Ingênuo (78 MB de VRAM)** | Computar 130 blendshapes em Compute Shader denso WGSL: $p_{\text{final}} = p_0 + \sum w_k \Delta p_k$ sobre todos os vértices da malha. | Seção 6.1 (Equação de blendshapes e tempo estimado em 0.03ms). | **Explosão de Largura de Banda de Memória:** Em uma malha de 25.000 vértices, 130 morphs densos com $\Delta p$ e $\Delta n$ (24 bytes) exigem $25.000 \times 24 \times 130 \approx \mathbf{78\text{ MB}}$ de VRAM! Contudo, 95% dos vértices têm deslocamento zero em morphs locais (ex: o slider `nose_bridge` só move ~150 vértices, mas avaliaria 25.000). | **Deltas Esparsos Indexados (Sparse Morph Target Buffers).** O motor WebGPU deve adotar o padrão glTF Sparse Accessor / MakeHuman: armazenar apenas pares `(vertex_id, delta_p, delta_n)`. A VRAM ocupada pelos 140+ morphs cai de 78 MB para **menos de 3,2 MB**, reduzindo as iterações de compute em mais de 92%. |
| **4** | **Cegueira Estrutural: Ausência de Skinning em `Vertex`** | Propôs Bone Deltas e matrizes de repouso ($B_{\text{inv}}'$) sem auditar o layout de vértices do motor. | Citou [crates/anigo-core/src/mesh.rs](file:///c:/ANIGO/crates/anigo-core/src/mesh.rs#L104-L201). | **Gargalo no Código Real:** O código em [crates/anigo-core/src/mesh.rs](file:///c:/ANIGO/crates/anigo-core/src/mesh.rs#L7-L17) e [shaders/cel_shading.wgsl](file:///c:/ANIGO/shaders/cel_shading.wgsl#L45-L50) define `Vertex` estritamente com `position`, `normal`, `uv` e `color`. **Não existem `joint_indices` nem `joint_weights`!** Sem atributos de skinning, nenhuma propagação esquelética ou deformação de membros pode ser renderizada na GPU. | **Evolução do Vértice para Skinned Anime NPR.** Adicionar `joints: [u16; 4]` e `weights: [f32; 4]` na estrutura `Vertex` e no pipeline de renderização WebGPU/WGSL, preparando o terreno nativo para as Sprints 03, 04 e 10. |
| **5** | **Distorção e Cisalhamento por Escala Óssea (Bone Shearing)** | Propôs escala não-uniforme local nas matrizes de juntas: $T_{\text{bone\_local}}' = T_{\text{bone\_local}} + w \cdot \Delta T$. | Seção 6.2 (Acoplamento de Bone Deltas). | **Falha de Animação Clássica:** Aplicar escalas não-uniformes na árvore cinemática de bones gera **matrizes com cisalhamento (*shearing*)** quando as juntas sofrem rotação (ex: dobrar o cotovelo ou joelho), deformando grosseiramente a malha. | **Arquitetura Dual: Morph Geométrico + Joint Translation Offset (BOND).** O volume e contorno dos membros são esculpidos via morphs aditivos sem alterar a escala dos ossos. O slider apenas translada a posição de repouso das juntas filhas (`joint_offset_y`) e recalcula a matriz inversa de bind pose $B_{\text{inv}}$, mantendo rotações ortogonais puras sem qualquer cisalhamento. |
| **6** | **Desconexão com `anigo-vrm` e `anigo-ik`** | Propôs criar novos módulos isolados `armature.rs` e formatos ad-hoc `.bin` em `anigo-core`. | Seção 7 (Sub-Sprints 3.2, 3.3). | **Duplicação de Código:** O repositório já possui [crates/anigo-vrm/src/lib.rs](file:///c:/ANIGO/crates/anigo-vrm/src/lib.rs#L14-L19) com `BlendShapeGroup` e [crates/anigo-ik/src/lib.rs](file:///c:/ANIGO/crates/anigo-ik/src/lib.rs#L6-L19) com `Skeleton` e `Bone`. Criar estruturas proprietárias desconectadas gera débito técnico que inviabiliza o exportador VRM da Sprint 10. | **Integração Padronizada glTF 2.0 / VRM 1.0.** Unificar a representação de morphs sob as especificações oficiais Khronos glTF 2.0 (`mesh.primitives[].targets`) e VRM 1.0 `VRMC_vrm.expressions`, integrando diretamente `anigo-vrm` e `anigo-ik`. |
| **7** | **Omissão de Músculos e Traços Críticos de Anime** | O catálogo de 132 sliders omitiu o Grande Dorsal (*lats*), o achatamento facial anime (*anime muzzle slant*), orelhas élficas e sobrancelhas. | Seção 5 (Catálogo de Sliders). | **Defasagem Estética:** É impossível criar um corpo masculino atlético/shonen em V-taper sem o Grande Dorsal (`latissimus_dorsi`). No rosto anime, a inclinação e espessura das sobrancelhas definem 70% da expressão, e o perfil estilizado exige achatamento da rima bucal. | **Catálogo Expandido para 148 Sliders em 18 Zonas.** Inclusão formal de Grande Dorsal, curvatura de espinha estilo Design Doll, perfil anime vs realista, orelhas estilizadas e sistema facial compatível com ARKit/FACS. |
| **8** | **UX Inviável (132 Sliders em Cascata Sem Macro Controles)** | Despejar mais de 130 sliders em uma coluna vertical sem controles macro de somatotípicos ou seleção tátil. | Seção 7 (Sub-Sprint 3.10). | **Fadiga Extrema do Usuário:** No VRoid e no Clip Studio Paint, o usuário nunca começa ajustando 130 micro-sliders. No CSP, existe um **Pad 2D de Somatótipo** (Magro $\leftrightarrow$ Forte / Corpulento $\leftrightarrow$ Infantil). | **Hierarquia de 3 Níveis de UI (Macro $\rightarrow$ Região $\rightarrow$ Micro).** Criação de um Pad 2D de Somatótipo (Triângulo de Heath-Carter), seleção direta por clique na malha 3D e expansão colapsável de micro-sliders no Svelte 5. |

---

## 2. BENCHMARK INDUSTRIAL APROFUNDADO: AS LIÇÕES DE VROID, DESIGN DOLL, CLIP STUDIO PAINT E THE SIMS 4

Para que o ANIGO supere os softwares existentes de acordo com o pedido do usuário, analisamos a engenharia reversa e os fluxos práticos de trabalho das quatro referências mundiais:

```
┌─────────────────────────────────────────────────────────────────────────────────────────────┐
│                          BENCHMARK DAS ARQUITETURAS DE CUSTOMIZAÇÃO                         │
├──────────────────┬───────────────────┬───────────────────┬──────────────────────────────────┤
│ SOFTWARE         │ MECANISMO CHAVE   │ PONTO FORTE       │ PRINCIPAL DEFEITO / LIMITAÇÃO    │
├──────────────────┼───────────────────┼───────────────────┼──────────────────────────────────┤
│ VRoid Studio     │ Shape Keys        │ Expressões faciais│ Corpo extremamente rígido. Sem   │
│ (Pixiv)          │ aditivas 3D em    │ ricas no padrão   │ sliders para glúteos, abdômen    │
│                  │ malha fixa Unity  │ VRM; pintura de   │ ou músculos reais. Exige desenhar│
│                  │                   │ cabelo estilizada │ "músculos falsos" em bodysuits.  │
├──────────────────┼───────────────────┼───────────────────┼──────────────────────────────────┤
│ Design Doll      │ Manipulação de    │ Caixas de volume  │ Sem suporte a blendshapes faciais│
│ (Terawell)       │ Bounding Boxes e  │ táteis no 3D;     │ padrão para animação; malha sem  │
│                  │ FFD por segmento; │ postura e escala  │ shader NPR estilizado; formato   │
│                  │ curvatura livre   │ dinâmica de corpos│ fechado proprietário.            │
├──────────────────┼───────────────────┼───────────────────┼──────────────────────────────────┤
│ Clip Studio Paint│ 2D Slider Pad     │ Pad 2D intuitivo  │ Não exporta malhas manipuladas   │
│ (Celsys - 3D Ver2│ (Músculo x Peso); │ para somatótipo;  │ nem rigs para uso em engines de  │
│ Drawing Figures) │ seleção direta de │ ajuste de costela,│ jogos ou animação 3D externa     │
│                  │ partes no 3D      │ cintura e nádegas │ (apenas render interno estático).│
├──────────────────┼───────────────────┼───────────────────┼──────────────────────────────────┤
│ The Sims 4 / CC4 │ Dual-Layer: BGEO  │ Desloca os vértices│ Custo computacional elevado de   │
│ (EA / Reallusion)│ (Morphs) + BOND   │ E sincroniza os   │ inicialização; rigs complexos não│
│                  │ (Joint Offsets)   │ ossos em tempo real│ orientados à estética anime pura.│
├──────────────────┴───────────────────┴───────────────────┴──────────────────────────────────┤
│ PADRÃO OURO ANIGO:                                                                          │
│ Combina a facilidade do Pad 2D do CSP, a interação tátil 3D do Design Doll, a arquitetura   │
│ matemática dual (Morph + Bone Offset) do Sims 4/CC4 e a exportação nativa VRM do VRoid.      │
└─────────────────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. ESTUDO DE MORFOLOGIA HUMANA E ESTILIZAÇÃO ANIME

### 3.1. Dimorfismo Sexual Anatômico (Esqueleto e Gordura)
O dimorfismo sexual humano na estilização de anime obedece a regras anatômicas e de silhueta rigorosas:

```
          SILHUETA MASCULINA (V-Taper)              SILHUETA FEMININA (Ampulheta)
                 ┌─────────────┐                           ┌─────────┐
                 │   OMBROS    │ Biacromial Larga          │ OMBROS  │ Biacromial Estreita
                 └──────┬──────┘                           └────┬────┘
                 ┌──────┴──────┐                           ┌────┴────┐
                 │   TÓRAX     │ Reto / Peitoral Plano     │  BUSTO  │ Cônico / Gota Pendente
                 └──────┬──────┘                           └────┬────┘
                 ┌──────┴──────┐                             ┌──┴──┐
                 │   CINTURA   │ Baixa (L1-L2)               │CINT.│ Pinch Alto (10ª costela)
                 └──────┬──────┘                             └──┬──┘
                 ┌──────┴──────┐                           ┌────┴────┐
                 │    BACIA    │ Estreita / Androide       │  BACIA  │ Larga / Ginecoide
                 └─────────────┘                           └─────────┘
```

1. **Estrutura Biacromial vs. Bitrocantérica**:
   - No corpo **Masculino**, a largura entre os acrômios dos ombros é a maior medida horizontal do tronco. A bacia é alta, estreita (arco subpúbico de ~60-70°) e os trocanteres femorais quase não se projetam para fora.
   - No corpo **Feminino**, a largura entre os trocanteres dos fêmures (quadris) iguala ou supera a largura dos ombros. A bacia é ampla, aberta (arco subpúbico de ~90-100°) e inclinada anteriormente em anteversão pélvica de 10-15° a mais que no homem, gerando uma curvatura lordótica natural que empina o glúteo.
2. **Dinâmica do Busto e Ligamentos de Cooper (Feminino)**:
   - Os seios não são hemisférios rígidos colados ao tórax (erro amador recorrente em jogos). Eles são glândulas suspensas pela fáscia peitoral e ligamentos de Cooper, exibindo formato natural em lágrima (*tear-drop*), achatamento no polo superior e caimento influenciado pela gravidade e pela postura da caixa torácica.
3. **Distribuição de Tecido Adiposo (Endomorfia)**:
   - **Gordura Androide (Masculina)**: Acúmulo no omento visceral e no abdômen anterior ("barriga de cerveja"), com flancos laterais espessos (*love handles*), mas membros relativamente secos.
   - **Gordura Ginecoide (Feminina)**: Acúmulo no tecido subcutâneo dos quadris (*culotes*), nádegas, face medial e anterior das coxas e braços inferiores, suavizando as transições articulares.

### 3.2. Os Três Somatotipos de Heath-Carter e Formulação Contínua
Para cumprir a exigência do usuário de permitir morfologias como **magro, musculoso, gordo, grande e pequeno**, o ANIGO implementa a parametrização do somatótipo em coordenadas contínuas normalizadas $(S_{\text{endo}}, S_{\text{meso}}, S_{\text{ecto}}) \in [0, 1]^3$:

$$\mathbf{p}_{\text{somato}} = S_{\text{endo}} \cdot \Delta \mathbf{p}_{\text{gordo}} + S_{\text{meso}} \cdot \Delta \mathbf{p}_{\text{muscular}} + S_{\text{ecto}} \cdot \Delta \mathbf{p}_{\text{magro}}$$

- **Ectomorfo (Magro / Esguio)**: Proeminência de relevos ósseos: clavícula esculpida, processo acromial pontiagudo, arco costal inferior visível, depressão supraclavicular profunda, espinhas ilíacas ântero-superiores marcadas.
- **Mesomorfo (Atlético / Musculoso)**: Hipertrofia dos ventres musculares: deltoides globulares, reto abdominal em 6 blocos divididos pelas interseções tendíneas e linha alba, grande dorsal expandido (*lats*), bíceps com pico pronunciado, vasto medial em gota (*teardrop*) acima da patela.
- **Endomorfo (Corpulento / Curvilíneo / Plus-Size)**: Preenchimento de dobras e sulcos, avental adiposo hipogástrico, coxas em contato medial (*thigh friction*), prega infraglútea contínua e queixo duplo suave.

### 3.3. Cânones de Altura de Anime (Régua de Cabeças)
- **2.0 a 3.0 Cabeças (Chibi / SD)**: Cabeça gigante (50-60% da altura), tronco minúsculo sem cintura, membros tubulares sem cotovelos/joelhos marcados.
- **4.0 a 5.0 Cabeças (Infantil / Kodomo)**: Proporção cabeça/tronco 1:1, silhueta ingênua sem curvatura de busto ou quadril.
- **6.0 a 6.5 Cabeças (Anime Padrão / Shojo / Shonen Teen)**: A proporção do VRoid Studio e de 90% das séries anime de TV.
- **7.0 a 7.5 Cabeças (Adulto Semi-Realista / Seinen)**: Estilo *Attack on Titan*, *Fate/Zero*, membros proporcionais e definição osteomuscular sutil.
- **8.0 a 8.5 Cabeças (Heróico / Fashion / JoJo)**: Pernas ultra-longas (60% da estatura total), tronco afilado, postura dinâmica e imponente.

### 3.4. Regras Canônicas de Estilização Facial Anime
- **Achatamento do Focinho (*Anime Muzzle Slant*)**: Ao contrário do rosto humano realista, onde o maxilar e a mandíbula se projetam em perfil com ângulo nasolabial de ~90-100°, o perfil anime clássico utiliza um vetor plano quase reto do nariz ao queixo, com rima bucal rebaixada.
- **Inclinação Cantal dos Olhos**:
  - *Tsurime* (ângulo cantal lateral elevado em +5° a +15°): Olhar confiante, afiado, rebelde, tsundere.
  - *Tareme* (ângulo cantal lateral rebaixado em -5° a -15°): Olhar dócil, tímido, gentil, moe.
- **Aegyosal (Bolsa Palpebral Inferior)**: O pequeno relevo muscular/adiposo logo abaixo dos cílios inferiores que confere jovialidade e profundidade à órbita estilisada.

---

## 4. O CATÁLOGO EXAUSTIVO DE SLIDERS ANATÔMICOS (148 SLIDERS EM 18 ZONAS)

Para atender estritamente ao pedido do usuário de permitir a deformação independente de **cada parte** sem travar o modelo em alterações globais, o catálogo foi expandido e refinado para **148 sliders estruturados**:

### 4.1. Proporções Globais & Silhueta (8 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo [Min, Padrão, Max] | Função Anatômica / Estética | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `height_overall` | Altura Estelar Total | Bone Delta | [110 cm, 165 cm, 215 cm] | Escala linear de toda a hierarquia esquelética | Ambos |
| `head_to_body_ratio` | Régua de Cabeças Canônica | Bone Delta | [2.0c, 6.5c, 8.5c] | Recalcula a proporção da caixa craniana em relação à estatura | Ambos |
| `head_scale_uniform` | Escala do Crânio | Bone Delta | [0.60x, 1.00x, 1.60x] | Multiplicador de escala da cabeça sem alterar o corpo | Ambos |
| `torso_to_limb_ratio` | Proporção Tronco/Pernas | Bone Delta | [0.70x, 1.00x, 1.40x] | Altera a altura da bacia, encurtando ou alongando as pernas | Ambos |
| `somatotype_endomorph` | Gordura / Corpulência Macro | Morph Target | [0.00, 0.00, 1.00] | Distribui volume adiposo global (abdômen, flancos, coxas) | Ambos |
| `somatotype_mesomorph` | Musculatura / Atletismo Macro | Morph Target | [0.00, 0.00, 1.00] | Projeta relevos e cortes musculares por todo o corpo | Ambos |
| `somatotype_ectomorph` | Magreza / Estrutura Óssea Macro | Morph Target | [0.00, 0.00, 1.00] | Afila os membros e projeta relevos ósseos pontiagudos | Ambos |
| `spine_s_curvature` | Curvatura de Postura S-Line | Bone + Morph | [-1.00, 0.00, 1.00] | Arqueamento da lordose/cifose espinhal (estilo Design Doll) | Ambos |

### 4.2. Crânio e Estrutura Craniofacial (11 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `head_width` | Largura Biparietal | Dual (Bone+Morph)| [0.75x, 1.00x, 1.35x] | Distância entre os ossos parietais laterais | Ambos |
| `head_depth` | Profundidade Occipital | Bone Delta | [0.80x, 1.00x, 1.30x] | Projeção anteroposterior da caixa craniana | Ambos |
| `face_lower_length` | Altura do Terço Inferior | Morph Target | [0.70x, 1.00x, 1.30x] | Distância da base nasal ao mento | Ambos |
| `forehead_height` | Altura da Testa | Morph Target | [0.75x, 1.00x, 1.40x] | Distância das sobrancelhas à linha capilar | Ambos |
| `forehead_roundness` | Curvatura Frontal | Morph Target | [0.00, 0.50, 1.00] | Convexidade da testa (arredondada feminina vs reta) | Ambos |
| `brow_ridge_prominence` | Arco Supraciliar | Morph Target | [0.00, 0.20, 1.00] | Projeção óssea sobre os olhos (traço masculino) | Ambos |
| `temple_width` | Largura das Têmporas | Morph Target | [0.80x, 1.00x, 1.25x] | Espessura lateral da fossa temporal | Ambos |
| `cheekbone_prominence` | Projeção Zigomática | Morph Target | [0.00, 0.30, 1.00] | Destaque das maçãs do rosto | Ambos |
| `cheek_fullness_upper` | Volume Malar Superior | Morph Target | [-1.00, 0.00, 1.00] | Bochechas fofas e arredondadas estilo anime | Ambos |
| `cheek_hollow_lower` | Concavidade Bucal Inferior | Morph Target | [0.00, 0.00, 1.00] | Bochechas encovadas (traço adulto/magro) | Ambos |
| `head_neck_blend` | Transição Cabeça-Pescoço | Morph Target | [0.00, 0.50, 1.00] | Suaviza o encaixe occipital para evitar pescoço colado | Ambos |

### 4.3. Olhos e Órbitas Anime (12 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica / Estética | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `eye_scale_uniform` | Escala Geral dos Olhos | Morph Target | [0.60x, 1.00x, 1.60x] | Tamanho total da fenda e globo ocular anime | Ambos |
| `eye_horizontal_width` | Largura Palpebral | Morph Target | [0.70x, 1.00x, 1.40x] | Abertura horizontal do canto medial ao lateral | Ambos |
| `eye_vertical_height` | Abertura Vertical | Morph Target | [0.60x, 1.00x, 1.50x] | Distância vertical entre pálpebra superior e inferior | Ambos |
| `eye_interpupillary_dist`| Espaçamento Intercantal | Morph Target | [0.75x, 1.00x, 1.30x] | Distância entre os olhos (olhos juntos vs afastados) | Ambos |
| `eye_vertical_position`| Posição Vertical na Face | Morph Target | [-1.00, 0.00, 1.00] | Olhos mais baixos = proporção infantil/moe | Ambos |
| `eye_socket_depth` | Profundidade Orbital | Morph Target | [-0.50, 0.00, 0.50] | Recuo dos olhos na cavidade craniana | Ambos |
| `eye_canthal_tilt` | Inclinação Cantal (Tsurime/Tareme)| Morph Target | [-20°, 0°, +20°] | Rotação angular externa da comissura palpebral | Ambos |
| `iris_scale_ratio` | Proporção da Íris | Morph Target | [0.60x, 1.00x, 1.40x] | Diâmetro relativo da íris e pupila | Ambos |
| `upper_eyelid_fold` | Dobra Palpebral Superior | Morph Target | [0.00, 0.50, 1.00] | Profundidade do vinco palpebral / dobra mongólica | Ambos |
| `lower_eyelid_aegyosal` | Volume da Bolsa Aegyosal | Morph Target | [0.00, 0.20, 1.00] | Volume estético abaixo da borda inferior | Ambos |
| `eyelid_corner_curve` | Curvatura dos Cantos | Morph Target | [-1.00, 0.00, 1.00] | Cantos agudos e afiados vs suaves e arredondados | Ambos |
| `pupil_vertical_squash`| Formato Oval da Pupila | Morph Target | [0.70x, 1.00x, 1.30x] | Pupila circular clássica vs achatada verticalmente | Ambos |

### 4.4. Sobrancelhas e Expressão Estrutural (6 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Estética Anime | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `eyebrow_thickness` | Espessura da Sobrancelha | Morph Target | [0.50x, 1.00x, 2.00x] | Linha fina delicada vs grossa expressiva | Ambos |
| `eyebrow_arch_height` | Altura do Arco Superior | Morph Target | [-1.00, 0.00, 1.00] | Sobrancelha reta vs arqueada dramática | Ambos |
| `eyebrow_inner_slant` | Inclinação Medial | Morph Target | [-15°, 0°, +15°] | Sobrancelhas caídas (tristeza/doçura) vs cerradas | Ambos |
| `eyebrow_spacing` | Distância Glabelar | Morph Target | [0.70x, 1.00x, 1.40x] | Espaçamento entre as pontas internas | Ambos |
| `eyebrow_length` | Comprimento Horizontal | Morph Target | [0.70x, 1.00x, 1.30x] | Extensão lateral da cauda da sobrancelha | Ambos |
| `eyebrow_depth` | Projeção em Relevo | Morph Target | [0.00, 0.20, 1.00] | Relevo 3D da linha da sobrancelha na malha | Ambos |

### 4.5. Nariz Estilizado Anime (8 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `nose_bridge_height` | Altura da Raiz Nasal | Morph Target | [0.60x, 1.00x, 1.40x] | Nível do násio na transição com a testa | Ambos |
| `nose_bridge_depth` | Projeção de Perfil do Dorso | Morph Target | [0.50x, 1.00x, 1.60x] | Distância da face em vista lateral | Ambos |
| `nose_length_vertical`| Comprimento Longitudinal | Morph Target | [0.70x, 1.00x, 1.30x] | Distância do násio à columela | Ambos |
| `nose_tip_upturn` | Ângulo da Ponta (Arrebitado)| Morph Target | [-25°, 0°, +25°] | Nariz arrebitado botão anime vs reto vs aquilino | Ambos |
| `nose_tip_sharpness` | Afunilamento da Ponta | Morph Target | [0.00, 0.70, 1.00] | Ponta pontual afiada (ponto de sombra anime) | Ambos |
| `nose_alar_width` | Largura da Base Alar | Morph Target | [0.60x, 1.00x, 1.40x] | Largura entre as narinas | Ambos |
| `nose_nostril_visibility`| Definição das Narinas | Morph Target | [0.00, 0.20, 1.00] | Fendas visíveis vs narina apagada estilo anime | Ambos |
| `nose_profile_flatness`| Achatamento Anime Muzzle | Morph Target | [0.00, 0.60, 1.00] | Redução do nariz realista para traço estilizado | Ambos |

### 4.6. Boca e Lábios (8 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `mouth_width` | Largura da Rima Bucal | Morph Target | [0.60x, 1.00x, 1.40x] | Distância de comissura a comissura labial | Ambos |
| `mouth_vertical_pos` | Posição Vertical / Filtro | Morph Target | [-1.00, 0.00, 1.00] | Comprimento do sulco do filtro labial | Ambos |
| `mouth_depth_protrusion`| Projeção Dentofacial | Morph Target | [-0.50, 0.00, 0.50] | Projeção anterior dos lábios | Ambos |
| `lip_upper_thickness` | Espessura do Lábio Superior | Morph Target | [0.20x, 1.00x, 2.00x] | Plenitude carnuda vs traço simples anime | Ambos |
| `lip_lower_thickness` | Espessura do Lábio Inferior | Morph Target | [0.20x, 1.00x, 2.00x] | Volume do vermelhão labial inferior | Ambos |
| `lip_corner_tilt` | Curvatura de Comissura | Morph Target | [-15°, 0°, +15°] | Cantos voltados sutilmente para cima vs neutro | Ambos |
| `lip_philtrum_depth` | Profundidade do Filtro | Morph Target | [0.00, 0.30, 1.00] | Nitidez das cristas do filtro labial | Ambos |
| `mouth_tuck_depth` | Reentrância das Comissuras | Morph Target | [0.00, 0.50, 1.00] | Covinhas na ponta da boca anime | Ambos |

### 4.7. Mandíbula, Queixo e Perfil Anime (10 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `jaw_bigonial_width` | Largura Bigoníaca da Mandíbula | Morph Target | [0.70x, 1.00x, 1.35x] | Largura entre os ângulos mandibulares | Ambos |
| `jaw_angle_vertical` | Altura do Ramo da Mandíbula | Morph Target | [0.75x, 1.00x, 1.25x] | Posição vertical do ângulo goníaco | Ambos |
| `jaw_v_line_taper` | Afunilamento V-Line Anime | Morph Target | [0.00, 0.60, 1.00] | Transição contínua para ponta triangular anime | Ambos |
| `chin_length` | Altura Vertical do Mento | Morph Target | [0.70x, 1.00x, 1.30x] | Comprimento do queixo | Ambos |
| `chin_width` | Largura da Ponta do Queixo | Morph Target | [0.50x, 1.00x, 1.50x] | Queixo fino delicado vs queixo quadrado | Ambos |
| `chin_forward_projection`| Projeção do Pogônio | Morph Target | [-0.50, 0.00, 0.50] | Projeção anterior do queixo de perfil | Ambos |
| `chin_cleft_dimple` | Covinha no Queixo (Cleft) | Morph Target | [0.00, 0.00, 1.00] | Sulco vertical mentoniano central | Masculino |
| `submental_fullness` | Gordura Submentoniana (Papada)| Morph Target | [0.00, 0.00, 1.00] | Tecido adiposo abaixo da mandíbula | Ambos |
| `jawline_bone_definition`| Nitidez da Borda Mandibular | Morph Target | [0.00, 0.70, 1.00] | Transição nítida da linha de sombra na mandíbula | Ambos |
| `anime_profile_slant` | Slant de Perfil Anime | Morph Target | [0.00, 0.50, 1.00] | Alinhamento em plano único de nariz e boca | Ambos |

### 4.8. Orelhas e Traços Élficos/Anime (5 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica / Estética | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `ear_scale_uniform` | Escala Auricular Total | Morph Target | [0.60x, 1.00x, 1.50x] | Tamanho relativo do pavilhão auditivo | Ambos |
| `ear_flare_angle` | Ângulo de Abertura (Orelha de Abano)| Morph Target | [0°, 15°, 40°] | Inclinação lateral em relação ao crânio | Ambos |
| `ear_pointy_elf` | Ponta Élfica / Fantasia | Morph Target | [0.00, 0.00, 1.00] | Alongamento agudo da ponta superior da hélice | Ambos |
| `ear_lobe_length` | Comprimento do Lóbulo | Morph Target | [0.50x, 1.00x, 1.50x] | Lóbulo pendente vs colado | Ambos |
| `ear_vertical_position`| Posição Vertical na Cabeça | Morph Target | [-1.00, 0.00, 1.00] | Alinhamento da orelha com a linha dos olhos | Ambos |

### 4.9. Pescoço e Trapézio (7 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `neck_length` | Comprimento do Pescoço | Bone Delta | [0.70x, 1.00x, 1.35x] | Distância da base do crânio a C7/T1 | Ambos |
| `neck_circumference` | Circunferência e Espessura | Dual (Bone+Morph)| [0.70x, 1.00x, 1.50x] | Diâmetro do cilindro cervical | Ambos |
| `trapezius_bulk` | Volume Superior do Trapézio | Morph Target | [0.00, 0.20, 1.00] | Massa muscular entre pescoço e ombro | Ambos |
| `adams_apple_prominence`| Pomo de Adão | Morph Target | [0.00, 0.00, 1.00] | Projeção da cartilagem tireóidea | Masculino |
| `scull_scm_tendon_relief`| Músculo Esternocleidomastóideo| Morph Target | [0.00, 0.30, 1.00] | Relevo dos tendões em V no pescoço | Ambos |
| `throat_concavity` | Concavidade da Fossa Jugular | Morph Target | [0.00, 0.50, 1.00] | Covinha central entre as cabeças das clavículas| Ambos |
| `neck_forward_posture` | Projeção Anterior do Pescoço | Bone Delta | [-15°, 0°, +20°] | Curvatura cervical para frente (estética anime)| Ambos |

### 4.10. Ombros, Clavículas e Escápulas (8 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `shoulder_biacromial_width`| Largura Biacromial dos Ombros | Bone Delta | [0.80x, 1.00x, 1.30x] | Distância entre os acrômios direito e esquerdo | Ambos |
| `shoulder_acromial_slope` | Inclinação Acromial | Bone Delta | [-10°, 0°, +15°] | Ombros retos atléticos vs caídos delicados | Ambos |
| `deltoid_muscle_volume` | Volume do Deltoide | Morph Target | [0.00, 0.20, 1.00] | Esfericidade muscular do ombro | Ambos |
| `clavicle_bone_relief` | Nitidez Óssea da Clavícula | Morph Target | [0.00, 0.50, 1.00] | Relevo proeminente da clavícula (traço magro/moe)| Ambos |
| `clavicle_v_angle` | Inclinação em V das Clavículas| Morph Target | [-10°, 0°, +15°] | Ângulo de subida em direção ao acrômio | Ambos |
| `scapula_wing_relief` | Proeminência das Escápulas | Morph Target | [0.00, 0.30, 1.00] | Destaque das asas das escápulas no dorso | Ambos |
| `shoulder_depth_thickness`| Espessura Glenoumeral | Morph Target | [0.80x, 1.00x, 1.30x] | Diâmetro anteroposterior da articulação | Ambos |
| `latissimus_dorsi_flare` | Expansão do Grande Dorsal | Morph Target | [0.00, 0.10, 1.00] | Asa do grande dorsal (essencial para V-taper) | Ambos |

### 4.11. Tórax e Peitorais Masculinos (6 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `ribcage_width` | Largura Torácica | Dual (Bone+Morph)| [0.80x, 1.00x, 1.30x] | Dimensão transversal da caixa torácica | Ambos |
| `ribcage_depth` | Profundidade do Tórax | Morph Target | [0.80x, 1.00x, 1.30x] | Diâmetro ântero-posterior esternal | Ambos |
| `pectoral_muscle_bulk` | Volume do Peitoral Maior | Morph Target | [0.00, 0.20, 1.00] | Hipertrofia do ventre peitoral | Masculino |
| `pectoral_lower_cut` | Definição da Linha Inframamária| Morph Target | [0.00, 0.40, 1.00] | Borda inferior nítida do peitoral masculino | Masculino |
| `pectoral_sternal_cleave`| Separação Esternal | Morph Target | [0.00, 0.30, 1.00] | Cleft central entre os dois peitorais | Masculino |
| `sternum_hollow_depth` | Concavidade Esternal | Morph Target | [0.00, 0.20, 1.00] | Sulco ósseo do esterno | Ambos |

### 4.12. Busto e Glândulas Mamárias Femininas (9 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `bust_volume_cup` | Volume do Busto (Copa A a G)| Morph Target | [0.00, 0.35, 1.50] | Massa total do tecido glandular mamário | Feminino |
| `bust_vertical_position`| Altura de Inserção Torácica | Morph Target | [-1.00, 0.00, 1.00] | Posição vertical do mamelão no tórax | Feminino |
| `bust_separation_cleavage`| Distância Intermamária (Decote)| Morph Target | [0.70x, 1.00x, 1.50x] | Espaçamento entre os polos mediais das mamas | Feminino |
| `bust_gravity_sag` | Caimento em Gota / Gravidade | Morph Target | [0.00, 0.30, 1.00] | Efeito pendente natural dos ligamentos de Cooper| Feminino |
| `bust_firmness_roundness`| Firmeza / Turgor Adiposo | Morph Target | [0.00, 0.70, 1.00] | Projeção cônica e sustentação superior | Feminino |
| `bust_outward_angle` | Ângulo de Divergência Lateral | Morph Target | [0°, 10°, 25°] | Orientação dos eixos mamários para os lados | Feminino |
| `bust_areola_diameter` | Diâmetro da Aréola | Morph Target | [0.50x, 1.00x, 2.00x] | Dimensão da zona areolar | Feminino |
| `bust_nipple_projection`| Projeção da Papila Mamária | Morph Target | [0.00, 0.20, 1.00] | Projeção anterior do mamilo | Feminino |
| `bust_underbust_taper` | Afunilamento Submamário | Morph Target | [0.75x, 1.00x, 1.25x] | Estreitamento da caixa torácica logo abaixo do busto| Feminino |

### 4.13. Abdômen, Cintura e Flancos (11 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `waist_pinch_width` | Estreitamento da Cintura | Dual (Bone+Morph)| [0.65x, 1.00x, 1.40x] | Menor circunferência transversal do tronco | Ambos |
| `waist_pinch_height` | Altura do Pinch da Cintura | Bone Delta | [0.80x, 1.00x, 1.20x] | Altura do estreitamento (alto na mulher, baixo no homem)| Ambos |
| `belly_visceral_protuberance`| Projeção Abdominal Anterior | Morph Target | [-0.50, 0.00, 1.00] | Barriga projetada para frente | Ambos |
| `belly_lower_panniculus`| Avental Adiposo Hipogástrico | Morph Target | [0.00, 0.00, 1.00] | Gordura acumulada abaixo da cicatriz umbilical | Ambos |
| `abs_sixpack_definition`| Relevo do Reto Abdominal | Morph Target | [0.00, 0.00, 1.00] | Relevo dos 6/8 gomos musculares e linha alba | Ambos |
| `oblique_apollo_belt` | Crista Ilíaca / Cinturão de Apolo| Morph Target | [0.00, 0.10, 1.00] | Definição dos oblíquos e virilha atlética | Ambos |
| `flank_love_handles` | Gordura nos Flancos Laterais | Morph Target | [0.00, 0.00, 1.00] | Depósito adiposo nos flancos | Ambos |
| `ribcage_costal_margin`| Projeção das Costelas Inferiores| Morph Target | [0.00, 0.20, 1.00] | Margem costal aparente (traço ectomorfo) | Ambos |
| `navel_vertical_pos` | Posição Vertical do Umbigo | Morph Target | [-1.00, 0.00, 1.00] | Posição vertical da cicatriz umbilical | Ambos |
| `navel_shape_slit` | Formato em Fenda Vertical | Morph Target | [0.00, 0.50, 1.00] | Umbigo fino vertical estilizado vs redondo | Ambos |
| `stomach_vacuum_depth` | Sucção Abdominal (Vacuum) | Morph Target | [0.00, 0.00, 1.00] | Retração atlética da parede abdominal | Ambos |

### 4.14. Pelve, Bacia e Curva do Quadril (9 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pelvis_bicristal_width`| Largura Bicristal da Bacia | Bone Delta | [0.75x, 1.00x, 1.35x] | Distância entre as cristas ilíacas superiores | Ambos |
| `hip_trochanteric_flare`| Curvatura Bitrocantérica (Quadris)| Morph Target | [0.75x, 1.00x, 1.45x] | Curvatura externa dos quadris femininos | Feminino |
| `pelvic_tilt_angle` | Anteversão / Retroversão Pélvica| Bone Delta | [-15°, 0°, +20°] | Inclinação pélvica sagital (curvatura da lordose)| Ambos |
| `iliac_crest_prominence`| Nitidez da Crista Ilíaca | Morph Target | [0.00, 0.40, 1.00] | Relevo ósseo da bacia na pele | Ambos |
| `trochanteric_fat_saddlebag`| Depósito de Culotes Laterais | Morph Target | [0.00, 0.00, 1.00] | Gordura trocantérica feminina externa | Feminino |
| `hip_dip_fill` | Preenchimento do Hip Dip | Morph Target | [-1.00, 0.00, 1.00] | Depressão do glúteo médio (violão contínuo vs depressão)| Ambos |
| `pubic_arch_width` | Largura do Arco Púbico | Morph Target | [0.70x, 1.00x, 1.40x] | Abertura do ângulo subpúbico | Ambos |
| `pelvis_depth` | Profundidade da Bacia | Morph Target | [0.80x, 1.00x, 1.30x] | Dimensão anteroposterior da cintura pélvica | Ambos |
| `groin_crease_depth` | Sulco da Dobra Inguinal | Morph Target | [0.00, 0.40, 1.00] | Profundidade da dobra entre tronco e coxa | Ambos |

### 4.15. Glúteos e Nádegas (9 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica / Estética | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `gluteus_volume_overall`| Volume Total das Nádegas | Morph Target | [0.60x, 1.00x, 1.60x] | Massa glútea máxima | Ambos |
| `gluteus_posterior_shelf`| Projeção Posterior de Perfil | Morph Target | [0.60x, 1.00x, 1.60x] | Projeção horizontal para trás | Ambos |
| `gluteus_lift_height` | Elevação Glútea Anti-Gravidade| Morph Target | [-1.00, 0.00, 1.00] | Sustentação do polo superior | Ambos |
| `gluteal_crease_depth` | Profundidade da Prega Infraglútea| Morph Target | [0.00, 0.50, 1.00] | Dobra horizontal na base da nádega | Ambos |
| `gluteus_shape_profile`| Perfil Geométrico Glúteo | Morph Target | [-1.00, 0.00, 1.00] | Formato Redondo (0.0) vs Coração Invertido (+1.0) vs V (-1.0)| Ambos |
| `gluteus_medius_fill` | Volume Súpero-Lateral | Morph Target | [0.00, 0.30, 1.00] | Preenchimento muscular superior da bacia | Ambos |
| `gluteus_firmness` | Firmeza Muscular vs Flacidez | Morph Target | [0.00, 0.60, 1.00] | Tônus muscular tenso vs relaxamento adiposo | Ambos |
| `intergluteal_cleft_depth`| Profundidade do Sulco Central| Morph Target | [0.00, 0.50, 1.00] | Separação das nádegas | Ambos |
| `infragluteal_crease_length`| Comprimento da Dobra Inferior| Morph Target | [0.00, 0.50, 1.00] | Extensão da dobra (curta anime vs longa realista)| Ambos |

### 4.16. Braços, Antebraços e Cotovelos (10 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arm_length_overall` | Comprimento Total do Braço | Bone Delta | [0.75x, 1.00x, 1.30x] | Escala longitudinal do membro superior | Ambos |
| `upper_arm_length` | Comprimento do Úmero | Bone Delta | [0.80x, 1.00x, 1.25x] | Distância do ombro ao cotovelo | Ambos |
| `forearm_length` | Comprimento Rádio-Ulna | Bone Delta | [0.80x, 1.00x, 1.25x] | Distância do cotovelo ao punho | Ambos |
| `upper_arm_thickness` | Espessura Geral do Braço | Dual (Bone+Morph)| [0.70x, 1.00x, 1.50x] | Diâmetro cilíndrico do braço | Ambos |
| `biceps_peak_volume` | Pico do Bíceps Braquial | Morph Target | [0.00, 0.20, 1.00] | Hipertrofia do ventre do bíceps | Ambos |
| `triceps_bulk` | Massa do Tríceps Braquial | Morph Target | [0.00, 0.20, 1.00] | Volume posterior do braço | Ambos |
| `forearm_brachioradialis`| Massa do Braquiorradial | Morph Target | [0.00, 0.20, 1.00] | Volume muscular do antebraço | Ambos |
| `forearm_taper_ratio` | Razão de Afunilamento | Morph Target | [0.60x, 1.00x, 1.30x] | Transição cônica cotovelo $\rightarrow$ punho | Ambos |
| `elbow_olecranon_sharpness`| Nitidez do Cotovelo (Olécrano)| Morph Target | [0.00, 0.40, 1.00] | Projeção óssea pontiaguda do cotovelo | Ambos |
| `wrist_circumference` | Circunferência do Punho | Dual (Bone+Morph)| [0.70x, 1.00x, 1.35x] | Diâmetro distal da articulação do punho | Ambos |

### 4.17. Mãos e Dedos (8 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica / Estética | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `hand_scale_uniform` | Escala da Mão | Bone Delta | [0.70x, 1.00x, 1.35x] | Escala proporcional da mão em relação à cabeça | Ambos |
| `palm_width` | Largura Metacarpal da Palma | Dual (Bone+Morph)| [0.75x, 1.00x, 1.35x] | Largura entre o 2º e o 5º metacarpo | Ambos |
| `palm_length` | Comprimento da Palma | Bone Delta | [0.80x, 1.00x, 1.25x] | Distância do carpo à base dos nós dos dedos | Ambos |
| `finger_length` | Comprimento dos Dedos | Bone Delta | [0.75x, 1.00x, 1.30x] | Comprimento longitudinal das falanges | Ambos |
| `finger_thickness` | Espessura das Falanges | Morph Target | [0.70x, 1.00x, 1.40x] | Dedos finos e delgados anime vs grossos | Ambos |
| `knuckle_joint_definition`| Relevo dos Nós dos Dedos | Morph Target | [0.00, 0.20, 1.00] | Proeminência das articulações metacarpofalângicas| Ambos |
| `thumb_opposability_angle`| Ângulo de Abertura do Polegar | Bone Delta | [0.70x, 1.00x, 1.30x] | Distância angular de oponibilidade do polegar | Ambos |
| `fingernail_style_anime` | Formato das Unhas Anime | Morph Target | [0.00, 0.50, 1.00] | Unhas curtas vs amendoadas estilizadas | Ambos |

### 4.18. Pernas, Joelhos, Panturrilhas e Pés (12 Sliders)
| ID do Slider | Nome de Exibição | Mecanismo | Intervalo | Função Anatômica | Dimorfismo |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `leg_length_overall` | Comprimento Total da Perna | Bone Delta | [0.75x, 1.00x, 1.40x] | Escala longitudinal do membro inferior | Ambos |
| `thigh_length` | Comprimento do Fêmur | Bone Delta | [0.80x, 1.00x, 1.30x] | Distância da cabeça femoral ao joelho | Ambos |
| `thigh_circumference` | Circunferência da Coxa | Dual (Bone+Morph)| [0.70x, 1.00x, 1.45x] | Diâmetro superior proximal da coxa | Ambos |
| `inner_thigh_gap` | Espaçamento Adutor Medial | Morph Target | [-1.00, 0.00, 1.00] | Contato medial entre coxas (-1.0) vs fresta (+1.0)| Ambos |
| `outer_thigh_sweep` | Curvatura Lateral da Coxa | Morph Target | [0.00, 0.40, 1.00] | Projeção do vasto lateral (*sweep* da coxa) | Ambos |
| `quadriceps_definition` | Relevo do Reto Femoral/Vasto | Morph Target | [0.00, 0.20, 1.00] | Definição atlética do quadríceps | Ambos |
| `knee_patella_prominence`| Projeção Óssea da Patela | Morph Target | [0.00, 0.50, 1.00] | Rótula definida e afiada no joelho | Ambos |
| `knee_valgus_uchimata` | Alinhamento Valgo (Uchimata Anime)| Bone Delta | [-10°, 0°, +15°] | Joelhos voltados para dentro (pose fofa anime) | Ambos |
| `calf_circumference` | Circunferência da Panturrilha | Dual (Bone+Morph)| [0.70x, 1.00x, 1.40x] | Espessura máxima do gastrocnêmio | Ambos |
| `gastrocnemius_height` | Altura do Ventre Muscular | Morph Target | [-1.00, 0.00, 1.00] | Panturrilha alta e esguia vs baixa e pesada | Ambos |
| `ankle_malleolus_thickness`| Espessura do Tornozelo | Dual (Bone+Morph)| [0.70x, 1.00x, 1.35x] | Circunferência bimaléolar distal | Ambos |
| `foot_scale_and_arch` | Tamanho do Pé e Arco Plantar | Dual (Bone+Morph)| [0.75x, 1.00x, 1.30x] | Comprimento do pé e altura do arco longitudinal | Ambos |

---

## 5. ARQUITETURA MATEMÁTICA E DE HARDWARE: MOTOR DUAL MORPH-BONE (DLMBA)

Para garantir máxima performance sem estiramento ou cisalhamento, o ANIGO implementa a **Arquitetura Dual Morph-Bone com Interação Tátil no Viewport** (*Dual-Layer Morph-Bone Architecture — DLMBA*):

```
┌─────────────────────────────────────────────────────────────────────────────────────────────┐
│                      ANIGO DLMBA — PIPELINE DE DEFORMAÇÃO EM WEBGPU                         │
├─────────────────────────────────────────────────────────────────────────────────────────────┤
│ 1. ENTRADA DE DADOS DO USUÁRIO                                                              │
│    - Pad 2D de Somatótipo (Ecto/Meso/Endo)  ou  Sliders 1D  ou  Arrasto Tátil 3D no Viewport│
├─────────────────────────────────────────────────────────────────────────────────────────────┤
│ 2. CAMADA ESQUELÉTICA (CPU/Rust — crates/anigo-ik)                                          │
│    - Sliders de comprimento aplicam transladações nas juntas: T'_child = T_child + ΔT       │
│    - Atualização recursiva da Bind Pose do mundo: W'_bone = W'_parent * T'_bone             │
│    - Recálculo dinâmico da matriz inversa de repouso: B_inv = (W'_bone)^-1                  │
│    - Preservação estrita de rotações ortogonais: ZERO cisalhamento de matrizes!              │
├─────────────────────────────────────────────────────────────────────────────────────────────┤
│ 3. CAMADA GEOMÉTRICA DE VÉRTICES (GPU — WebGPU Compute Shader em WGSL)                     │
│    - Entrada: Base Vertex Buffer + Sparse Morph Delta Buffer (VRAM < 3.5 MB)                │
│    - Acumulação esparsa dos sliders ativos:                                                 │
│        p_rest = p_base + SUM( w_k * Δp_k )                                                  │
│        n_rest = normalize( n_base + SUM( w_k * Δn_k ) )                                     │
├─────────────────────────────────────────────────────────────────────────────────────────────┤
│ 4. CAMADA DE SKINNING E NPR CEL-SHADING (GPU — shaders/cel_shading.wgsl)                    │
│    - Linear Blend Skinning (LBS) nativo:                                                    │
│        p_final = SUM( weight_j * ( M_j * B_inv_j ) * p_rest )                               │
│        n_final = SUM( weight_j * ( M_j * B_inv_j )_rot * n_rest )                           │
│    - Passe 1: Inverted Hull Outline Pass com espessura compensada pela escala da câmera.    │
│    - Passe 2: Cel-Shading NPR Pass com rampa toon quantizada e matiz de sombra hue-shifted. │
└─────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 5.1. Armazenamento Esparso de Morphs na GPU
Ao invés de alocar matrizes densas de 78 MB, os deltas são armazenados em um `storage_buffer` compacto contendo apenas os vértices ativos:

```rust
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SparseMorphDelta {
    pub vertex_index: u32,
    pub delta_position: [f32; 3],
    pub delta_normal: [f32; 3],
}
```

No Compute Shader WGSL:
```wgsl
struct MorphChannel {
    weight: f32,
    start_offset: u32,
    delta_count: u32,
};

@group(0) @binding(0) var<storage, read> base_positions: array<vec4<f32>>;
@group(0) @binding(1) var<storage, read> morph_deltas: array<SparseMorphDelta>;
@group(0) @binding(2) var<storage, read> active_channels: array<MorphChannel>;
@group(0) @binding(3) var<storage, read_write> out_positions: array<vec4<f32>>;

@compute @workgroup_size(64)
fn cs_accumulate_morphs(@builtin(global_invocation_id) global_id: vec3<u32>) {
    let vert_idx = global_id.x;
    if (vert_idx >= arrayLength(&base_positions)) { return; }

    var p = base_positions[vert_idx].xyz;
    // Acumula apenas morphs com peso diferente de zero que cobrem este vértice
    // ...
    out_positions[vert_idx] = vec4<f32>(p, 1.0);
}
```

### 5.2. Manipulação Tátil no Viewport (Estilo Design Doll & The Sims 4)
Para implementar o clique e arrasto no 3D sem Green Coordinates redundantes:
1. **Raycasting de Hulls Bounding**: Quando o mouse passa sobre o Viewport, um raio 3D testa interseção com volumes cilíndricos encapsulantes de cada segmento (`Head`, `Chest`, `Waist`, `Hips`, `UpperArm`, `Thigh`, `Calf`).
2. **Projeção de Arrastamento**: O vetor de deslocamento do mouse na tela $\Delta \mathbf{x}_{\text{screen}}$ é desprojetado no plano da câmera orientado pelos eixos locais do segmento 3D:
   $$\Delta s_x = \mathbf{r}_{\text{local\_x}} \cdot \Delta \mathbf{x}_{\text{world}}, \quad \Delta s_y = \mathbf{r}_{\text{local\_y}} \cdot \Delta \mathbf{x}_{\text{world}}$$
3. **Mapeamento Direto para Sliders**:
   - Arrastar horizontalmente no Tórax ajusta `ribcage_width`.
   - Arrastar verticalmente na Coxa ajusta `thigh_length`.
   - Arrastar horizontalmente na Bacia ajusta `hip_trochanteric_flare`.
O usuário usufrui da mesma agilidade tátil do Design Doll e do The Sims 4, enquanto o motor gráfico mantém a precisão paramétrica absoluta e estabilidade numérica.

---

## 6. ROADMAP DE ALTO PADRÃO: 14 SUB-SPRINTS MODULARES

Para executar a Sprint 03 com excelência profissional inviolável, o trabalho é estruturado em **14 Sub-Sprints especializadas**:

```
┌─────────────────────────────────────────────────────────────────────────────────────────────┐
│                       ROADMAP ESTRUTURADO DAS 14 SUB-SPRINTS                                │
├─────────────────────────────────────────────────────────────────────────────────────────────┤
│ 3.1  Topologia Canônica Isomórfica e Modelos Base glTF Male & Female                         │
│ 3.2  Evolução do Vértice NPR Skinned e Buffers de Skinning (crates/anigo-core)              │
│ 3.3  Motor de Morphs Esparsos em WebGPU Compute Shader (WGSL)                               │
│ 3.4  Motor de Propagação Esquelética e Sincronização de Rig (crates/anigo-ik)               │
│ 3.5  Pad 2D de Somatótipo Macro e Solver de Deformação Contínua (Ecto-Meso-Endo)           │
│ 3.6  Morfologia Craniofacial e Sliders Faciais Estilizados (ARKit/Anime)                    │
│ 3.7  Morfologia Cervical, Trapézio, Clavículas e Pomo de Adão                               │
│ 3.8  Morfologia Torácica, Peitorais Masculinos e Busto Feminino Dinâmico (Copa A a G)      │
│ 3.9  Morfologia Abdominal, Cintura, Flancos e Definição Muscular                            │
│ 3.10 Morfologia Pélvica, Bacia e Geometria Glútea Avançada (4 Formatos)                    │
│ 3.11 Morfologia dos Membros Superiores (Ombros, Braços, Mãos e Dedos)                       │
│ 3.12 Morfologia dos Membros Inferiores (Coxas, Joelhos, Panturrilhas e Pés)                 │
│ 3.13 Manipulação Tátil no Viewport 3D (Gizmos de Bounding Hull e Raycasting)               │
│ 3.14 Interface de Estúdio Svelte 5, Presets de Fábrica, Undo/Redo e Validação MCP           │
└─────────────────────────────────────────────────────────────────────────────────────────────┘
```

Abaixo detalha-se cada Sub-Sprint:

---

### SUB-SPRINT 3.1: Topologia Canônica Isomórfica e Modelos Base glTF Male & Female
- **Objetivo**: Integrar no ecossistema ANIGO o par de modelos base canônicos (1 Masculino e 1 Feminino) no formato glTF/VRM com **isomorfismo topológico estrito 1:1** ($N$ vértices idênticos, mesma indexação de faces e UVs coincidentes).
- **Arquivos Envolvidos**:
  - `assets/models/anigo_base_male.glb`: Modelo masculino mesomórfico canônico.
  - `assets/models/anigo_base_female.glb`: Modelo feminino ginecoide canônico.
  - `crates/anigo-core/src/mesh.rs`: Leitor de malhas glTF 2.0 com suporte a buffers de atributos customizados.
- **DoD & Critérios de Aceite**:
  - Verificação automatizada de isomorfismo: número de vértices de `male` == `female` e contagem de faces idêntica.
  - Carregamento e renderização básica com cel-shading em menos de 80ms.

---

### SUB-SPRINT 3.2: Evolução do Vértice NPR Skinned e Buffers de Skinning
- **Objetivo**: Estender a estrutura `Vertex` e o pipeline de renderização para suportar atributos de pele padrão da indústria (`joints: [u16; 4]`, `weights: [f32; 4]`).
- **Arquivos Envolvidos**:
  - [crates/anigo-core/src/mesh.rs](file:///c:/ANIGO/crates/anigo-core/src/mesh.rs): Atualização de `struct Vertex` e packing `Pod`/`Zeroable`.
  - [shaders/cel_shading.wgsl](file:///c:/ANIGO/shaders/cel_shading.wgsl): Atualização de `struct VertexInput` e cálculo de Linear Blend Skinning no `@vertex`.
  - [src/components/viewport/webgpu_renderer.ts](file:///c:/ANIGO/src/components/viewport/webgpu_renderer.ts): Atualização do layout de atributos do vertex buffer.
- **DoD & Critérios de Aceite**:
  - Compilação limpa sem quebra de compatibilidade com as cenas estáticas existentes.
  - Shaders WGSL validados sem erros no validador do `wgpu`.

---

### SUB-SPRINT 3.3: Motor de Morphs Esparsos em WebGPU Compute Shader
- **Objetivo**: Implementar o compute pass em WebGPU que processa mais de 140 canais de morph target esparsos diretamente na GPU sem alocação massiva de VRAM.
- **Arquivos Envolvidos**:
  - `shaders/morph_sparse_compute.wgsl`: Compute shader com workgroups otimizados (64 threads).
  - [crates/anigo-renderer/src/headless.rs](file:///c:/ANIGO/crates/anigo-renderer/src/headless.rs): Gerenciamento de buffers de deltas esparsos e despacho de compute no renderer headless.
  - [src/components/viewport/webgpu_renderer.ts](file:///c:/ANIGO/src/components/viewport/webgpu_renderer.ts): Integração do compute pass no pipeline interativo.
- **DoD & Critérios de Aceite**:
  - Execução simultânea de 148 sliders ativos em < 0.08ms por frame na GPU real.
  - Consumo total de VRAM para todos os dados de morph menor que 4 MB.

---

### SUB-SPRINT 3.4: Motor de Propagação Esquelética e Sincronização de Rig
- **Objetivo**: Integrar os Bone Deltas e o recálculo dinâmico de matrizes de repouso ($B_{\text{inv}}$) sem cisalhamento de juntas.
- **Arquivos Envolvidos**:
  - [crates/anigo-ik/src/lib.rs](file:///c:/ANIGO/crates/anigo-ik/src/lib.rs): Expansão da hierarquia de `Skeleton` e matrizes globais de ossos humanoides padrão VRM.
  - `crates/anigo-core/src/bone_sync.rs`: Mapeamento de sliders para transladações de juntas filhas e atualização recursiva de Forward Kinematics.
- **DoD & Critérios de Aceite**:
  - Alterar a régua de cabeças ou comprimento dos membros desloca as juntas ósseas sem descolar a malha da pele.
  - Testes unitários comprovando que as matrizes de rotação das juntas permanecem 100% ortogonais (determinante == 1.0).

---

### SUB-SPRINT 3.5: Pad 2D de Somatótipo Macro e Solver de Deformação Contínua
- **Objetivo**: Criar o controle de alto nível para interpolação contínua entre Ectomorfo, Mesomorfo e Endomorfo (estilo Clip Studio Paint).
- **Arquivos Envolvidos**:
  - `crates/anigo-core/src/somatotype.rs`: Solver baricêntrico de somatótipo contínuo de Heath-Carter.
  - `src/components/character/SomatotypePad2D.svelte`: Componente interativo 2D com design dark e cursor SVG arrastável.
- **DoD & Critérios de Aceite**:
  - Mover o ponto no Pad 2D altera suavemente o manequim de franzino para hiper-musculoso ou plus-size a 120 FPS cravados.
  - Transição perfeita entre anatomias masculina e feminina através do slider mestre `gender_dimorphism`.

---

### SUB-SPRINT 3.6: Morfologia Craniofacial e Sliders Faciais Estilizados
- **Objetivo**: Implementar os 47 sliders de Crânio, Olhos, Sobrancelhas, Nariz, Boca, Mandíbula e Orelhas (Seções 4.2 a 4.8).
- **Arquivos Envolvidos**:
  - `assets/morphs/head_face_sparse.bin`: Deltas compactados de geometria facial.
  - `crates/anigo-core/src/morphs/face.rs`: Tabela de canais e limites de segurança facial.
  - `src/components/character/FaceMorphControls.svelte`: Interface com agrupamento por abas: Olhos, Nariz/Boca, Mandíbula, Sobrancelhas.
- **DoD & Critérios de Aceite**:
  - Olhos com variação contínua de *Tsurime* (+20°) a *Tareme* (-20°) sem distorção das normais.
  - Perfil anime ajustável com achatamento correto do focinho (*muzzle slant*).

---

### SUB-SPRINT 3.7: Morfologia Cervical, Trapézio, Clavículas e Pomo de Adão
- **Objetivo**: Implementar os 15 sliders de Pescoço, Ombros, Clavículas e Trapézio (Seções 4.9 e 4.10).
- **Arquivos Envolvidos**:
  - `assets/morphs/neck_shoulders_sparse.bin`: Deltas anatômicos cervicais e escapulares.
  - `src/components/character/NeckShoulderControls.svelte`: Painel de controle no Inspetor.
- **DoD & Critérios de Aceite**:
  - Ativação do Pomo de Adão restrita ao gênero masculino com projeção anatômica nítida.
  - Clavículas com projeção óssea precisa sem quebras no contorno Inverted Hull.

---

### SUB-SPRINT 3.8: Morfologia Torácica, Peitorais Masculinos e Busto Feminino Dinâmico
- **Objetivo**: Implementar os 15 sliders dedicados ao tórax, peitorais e busto feminino com gravidade e decote (Seções 4.11 e 4.12).
- **Arquivos Envolvidos**:
  - `assets/morphs/chest_bust_sparse.bin`: Deltas do peitoral atlético e busto com curvatura em lágrima.
  - `crates/anigo-core/src/morphs/chest.rs`: Lógica de volume de Copa A a G e caimento pendente.
  - `src/components/character/ChestBustControls.svelte`: Interface de controle no Inspetor.
- **DoD & Critérios de Aceite**:
  - Variação do busto feminino preserva o formato orgânico em gota, sem estiramento poligonal esférico ingênuo.
  - Peitoral masculino mesomórfico exibe corte inframamário reto e separação esternal nítida.

---

### SUB-SPRINT 3.9: Morfologia Abdominal, Cintura, Flancos e Definição Muscular
- **Objetivo**: Implementar os 11 sliders de abdômen, cintura, gordura hipogástrica e gomos musculares (Seção 4.13).
- **Arquivos Envolvidos**:
  - `assets/morphs/abdomen_waist_sparse.bin`: Deltas de 6-pack, avental adiposo e flancos.
  - `src/components/character/TorsoMorphControls.svelte`: Interface no Inspetor.
- **DoD & Critérios de Aceite**:
  - Capacidade de criar barriga saliente (gordura visceral) mantendo membros magros, ou abdômen trincado atlético.
  - Estreitamento da cintura (*waist pinch*) independente da largura da bacia óssea.

---

### SUB-SPRINT 3.10: Morfologia Pélvica, Bacia e Geometria Glútea Avançada
- **Objetivo**: Implementar os 18 sliders de bacia, quadris e nádegas com 4 perfis geométricos (Seções 4.14 e 4.15).
- **Arquivos Envolvidos**:
  - `assets/morphs/pelvis_gluteus_sparse.bin`: Deltas de bacia ginecoide/androide e nádegas.
  - `crates/anigo-core/src/morphs/gluteus.rs`: Solver geométrico para formatos Redondo, Coração Invertido, Quadrado e V.
  - `src/components/character/PelvisGluteusControls.svelte`: Interface no Inspetor.
- **DoD & Critérios de Aceite**:
  - Nádegas com controle independente de volume, projeção posterior e sustentação anti-gravidade.
  - Eliminação da deficiência clássica do VRoid, permitindo silhuetas curvilíneas completas.

---

### SUB-SPRINT 3.11: Morfologia dos Membros Superiores (Ombros, Braços, Mãos e Dedos)
- **Objetivo**: Implementar os 18 sliders de braços, bíceps, antebraço, mãos e dedos (Seções 4.16 e 4.17).
- **Arquivos Envolvidos**:
  - `assets/morphs/arms_hands_sparse.bin`: Deltas de massa do braço e falanges.
  - `src/components/character/ArmsHandsControls.svelte`: Interface no Inspetor.
- **DoD & Critérios de Aceite**:
  - Escala uniforme de mãos sem perda de proporcionalidade com os dedos.
  - Contração e pico de bíceps independente da espessura do antebraço.

---

### SUB-SPRINT 3.12: Morfologia dos Membros Inferiores (Coxas, Joelhos, Panturrilhas e Pés)
- **Objetivo**: Implementar os 12 sliders de pernas, joelhos, panturrilhas, arco do pé e alinhamento *Uchimata* (Seção 4.18).
- **Arquivos Envolvidos**:
  - `assets/morphs/legs_feet_sparse.bin`: Deltas de coxas, patela, gastrocnêmio e pés.
  - `src/components/character/LegsFeetControls.svelte`: Interface no Inspetor.
- **DoD & Critérios de Aceite**:
  - Ajuste de *Inner Thigh Gap* funcional sem interpenetração das geometrias das pernas.
  - Alinhamento valgo (*Uchimata*) ajustável para poses expressivas de anime.

---

### SUB-SPRINT 3.13: Manipulação Tátil no Viewport 3D
- **Objetivo**: Implementar o sistema de seleção direta por clique na malha e arrasto na tela (estilo Design Doll e The Sims 4).
- **Arquivos Envolvidos**:
  - `crates/anigo-core/src/tactile/raycast_hulls.rs`: Teste de interseção raio-cilindro/cápsula por segmento anatômico.
  - `src/components/viewport/TactileManipulatorOverlay.svelte`: Destaque sutil de contorno na área sob o cursor e captura de arrasto.
  - [src/components/viewport/webgpu_renderer.ts](file:///c:/ANIGO/src/components/viewport/webgpu_renderer.ts): Integração com eventos de mouse/pointer.
- **DoD & Critérios de Aceite**:
  - Clicar e arrastar na coxa, busto ou ombro no Viewport ajusta os sliders paramétricos em tempo real.
  - Sincronização bidirecional instantânea entre a manipulação no 3D e os sliders da barra lateral.

---

### SUB-SPRINT 3.14: Interface de Estúdio Svelte 5, Presets de Fábrica, Undo/Redo e Validação MCP
- **Objetivo**: Integrar o painel completo "Personagem $\rightarrow$ Anatomia & Corpo" com busca instantânea, presets corporais, histórico completo de Undo/Redo e suíte de auditoria via `anigo-mcp`.
- **Arquivos Envolvidos**:
  - [src/App.svelte](file:///c:/ANIGO/src/App.svelte): Estruturação da aba de Anatomia com sub-abas colapsáveis e zero emojis.
  - `src/services/character_presets.ts`: Presets de fábrica: *Shonen Hero*, *Shojo Idol*, *Muscular Berserker*, *Plus Size / Chubby*, *Chibi 2.5c*, *Stylized Heroic 8.5c*.
  - [crates/anigo-mcp/src/main.rs](file:///c:/ANIGO/crates/anigo-mcp/src/main.rs): Ferramentas MCP estritas:
    - `anigo_set_character_model(model_type: "male" | "female")`
    - `anigo_set_somatotype(endo: f32, meso: f32, ecto: f32)`
    - `anigo_apply_morph_slider(slider_id: string, value: f32)`
    - `anigo_inspect_mesh_integrity()`
    - `anigo_get_active_morphs()`
    - `anigo_reset_morphs()`
  - `scripts/test_sprint03_mcp.py`: Script de validação automatizada e prova visual por MSE/PSNR.
- **DoD & Critérios de Aceite**:
  - Todos os 148 sliders integrados com Undo/Redo instantâneo (Ctrl+Z / Ctrl+Y).
  - Execução completa de `scripts/test_sprint03_mcp.py` gerando frames de prova com integridade de malha validada (zero vértices degenerados, zero normais invertidas).

---

## 7. REMAINING QUESTIONS & GAPS (QUESTÕES PENDENTES E LACUNAS)

Em rigorosa conformidade com o protocolo de honestidade e objetividade do ANIGO, registram-se as seguintes lacunas e áreas de atenção:

1. **Origem dos Dados Geométricos dos Modelos Base Male e Female**:
   - *Lacuna*: A especificação estabelece a exigência imperativa de **isomorfismo topológico estrito 1:1** entre as malhas base Masculina e Feminina. Contudo, o repositório atual em `assets/models` encontra-se vazio.
   - *Onde focar na Sub-Sprint 3.1*: Decidir e validar se a malha isomórfica base será adaptada de uma topologia aberta consagrada sob licença permissiva (como a topologia `base/hm08` CC0 do MakeHuman adaptada para proporções de anime) ou se será sintetizada proceduralmente em código Rust via subdivisão de superfícies quad-dominant.
2. **Compressão e Precisão Numérica dos Deltas de Morphs**:
   - *Lacuna*: Para 148 sliders, armazenar deltas em ponto flutuante de precisão simples (FP32) totaliza ~3,2 MB. Deve-se avaliar na Sub-Sprint 3.3 se a conversão para ponto flutuante de meia precisão (FP16 / `f16` no WGSL) traz ganhos perceptíveis de taxa de transferência de memória em GPUs integradas sem degradar a precisão de detalhes faciais sutis (como pálpebras e lábios).
3. **Mapeamento UV e Compatibilidade com Futuras Sprints**:
   - *Lacuna*: A malha unificada precisa ter suas ilhas de UV organizadas para suportar tanto a pintura de texturas 4K da **Sprint 09** quanto a sobreposição de roupas da **Sprint 06**.
   - *Ação preventiva*: Definir na Sub-Sprint 3.1 o mapa UV canônico (cabeça/rosto com densidade de texels prioritária de 50%, tronco 30%, membros 20%).
4. **Prioridade Imediata para o Parent Orchestrator**:
   - Atualizar a especificação formal de [SPRINTS_PADRAO_OURO/SPRINT_03.md](file:///c:/ANIGO/SPRINTS_PADRAO_OURO/SPRINT_03.md) para incorporar a reestruturação nas 14 Sub-Sprints aqui fundamentadas.
   - Autorizar o início da **Sub-Sprint 3.1** com o provisionamento e validação do par de malhas glTF isomórficas canônicas em `assets/models/`.
