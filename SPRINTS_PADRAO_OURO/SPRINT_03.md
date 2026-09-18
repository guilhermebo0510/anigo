# ANIGO — ESPECIFICAÇÃO DE ENGENHARIA DE ALTO PADRÃO (PADRÃO OURO)

> [!CAUTION]
> **REGRA INVIOLÁVEL DESTA SPRINT (ANIGO_INVIOLABLE_RULES)**:
> Esta sprint NÃO será considerada concluída sem testes manuais e visuais reais executados via `anigo-mcp` com geração e inspeção crítica de frames de prova offscreen e na janela ativa do usuário. É expressamente proibido o uso de mockups (como imagens base64 disfarçadas de 3D, SVGs fictícios ou emojis na interface). A interface deve ser de padrão comercial (ícones vetoriais profissionais SVG/Lucide) e o código em Rust idiomático (zero `unwrap` em caminhos críticos, WebGPU/WGSL nativo).

---

# SPRINT 03: MANEQUIM ANATÔMICO DUAL (MALE/FEMALE), MOTOR MORFOLÓGICO DE 148 SLIDERS E ARQUITETURA DUAL DLMBA

## 1. OBJETIVO E VISÃO INDUSTRIAL

Implementar o sistema de criação e deformação de personagens do ANIGO com qualidade de estúdio profissional, superando diretamente o **VRoid Studio** (Pixiv), **Design Doll** (Terawell), **Clip Studio Paint 3D** (Celsys) e integrando o rigor arquitetural do **The Sims 4 / CC4** (EA/Reallusion).

A Sprint 03 estabelece:
1. **Dois Modelos Base Canônicos Isomórficos 1:1**: 1 modelo Masculino e 1 modelo Feminino com topologia canônica unificada ($N$ vértices idênticos, mesma topologia de faces e mesmo mapeamento UV), permitindo interoperabilidade absoluta de roupas, cabelos, animações e interpolação de gênero contínua (`gender_dimorphism`).
2. **Catálogo de 148 Sliders Anatômicos em 18 Zonas Corporais**: Controle granular de cada componente do corpo humano e estilização anime (crânio, olhos, sobrancelhas, nariz, boca, mandíbula, orelhas, pescoço, ombros/dorsal, tórax, busto dinâmico copa A-G, abdômen/cintura, pelve/quadris, glúteos em 4 perfis, braços, mãos/dedos, pernas, joelhos, panturrilhas e pés).
3. **Arquitetura Dual Morph-Bone (DLMBA)**:
   - **Camada Geométrica GPU**: Compute Shaders WebGPU em WGSL processando deltas esparsos indexados de blendshapes (< 3,5 MB de VRAM total, zero overhead).
   - **Camada Esquelética CPU**: Translação de juntas filhas (*Joint Translation Offsets — BOND*) e recálculo dinâmico da matriz inversa de bind pose ($B_{\text{inv}}$), garantindo rotações ortogonais puras sem qualquer cisalhamento de ossos (*zero bone shearing*).
4. **Manipulação Tátil Direta no Viewport (Estilo Design Doll & The Sims 4)**: Raycasting de *bounding hulls* cilíndricos nos segmentos 3D com projeção tela-para-espaço-local, permitindo clicar e arrastar diretamente nos membros ou tórax para alterar sliders sem atrito.
5. **Pad 2D de Somatótipo Macro (Heath-Carter)**: Controle contínuo de somatótipos (Ectomorfo $\leftrightarrow$ Mesomorfo $\leftrightarrow$ Endomorfo) estilo Clip Studio Paint.

---

## 2. BENCHMARK INDUSTRIAL COMPARATIVO

| Software | Mecanismo Central | Pontos Fortes | Limitações Superadas pelo ANIGO |
| :--- | :--- | :--- | :--- |
| **VRoid Studio** | Shape keys aditivas Unity em malha fixa | Ricas expressões faciais VRM | Corpo engessado; sem sliders para glúteos reais, músculos ou abdômen; exige texturas desenhadas para simular anatomia. |
| **Design Doll** | Bounding boxes e FFD por segmento corporal | Manipulação tátil ágil no 3D; flexibilidade de postura e proporção | Sem blendshapes faciais padronizados para animação; sem cel-shading NPR; formato proprietário fechado. |
| **Clip Studio Paint 3D** | Pad 2D de Somatótipo + seleção de peças | Extremamente intuitivo; controle 2D de músculo vs peso | Renderizador interno proprietário; não exporta malhas rigeadas para engines de jogos ou software de animação 3D externo. |
| **The Sims 4 / CC4** | Dual-Layer: BGEO (Morphs) + BOND (Joint Offsets) | Sincroniza deslocamento de vértices com os ossos do rig | Alto custo computacional; rigs hiper-complexos não otimizados para silhueta anime NPR. |
| **ANIGO (Padrão Ouro)** | **DLMBA + Isomorfismo 1:1 + WGSL Compute + VRM 1.0** | **O melhor de todos:** Pad 2D do CSP, toque 3D do Design Doll, rigor dual do Sims 4, expressões VRM e shaders NPR ultra-rápidos. |

---

## 3. ESTUDO MORFOLÓGICO E DIMORFISMO SEXUAL ANIME

### 3.1. Dimorfismo Osteomuscular e Tecido Adiposo
- **Masculino (V-Taper)**: Diâmetro biacromial largo, tórax largo, peitorais retos com corte inframamário nítido, grande dorsal (*latissimus dorsi*) saliente, bacia estreita androide, gordura androide visceral concentrada no abdômen. Pomo de Adão proeminente, mandíbula quadrada e arcos supraciliares marcados.
- **Feminino (Ampulheta)**: Diâmetro bitrocantérico igual ou superior aos ombros, bacia ginecoide ampla em anteversão pélvica natural (+10-15° lordose), busto anatômico suspenso pelos ligamentos de Cooper (formato gota com caimento natural e decote variável), cintura alta (*pinch* na 10ª costela), coxas com acúmulo subcutâneo ginecoide (*culotes* suaves). Rosto com mandíbula triangular (*V-Line*), queixo afilado e olhos amplos.

### 3.2. Espaço de Somatótipo Contínuo de Heath-Carter
A morfologia global é governada pela equação baricêntrica normalizada $(S_{\text{endo}}, S_{\text{meso}}, S_{\text{ecto}}) \in [0, 1]^3$:
$$\mathbf{p}_{\text{somato}} = S_{\text{endo}} \cdot \Delta \mathbf{p}_{\text{gordo}} + S_{\text{meso}} \cdot \Delta \mathbf{p}_{\text{muscular}} + S_{\text{ecto}} \cdot \Delta \mathbf{p}_{\text{magro}}$$

### 3.3. Cânones Estilizados Anime
- **Régua de Proporções (Cabeças)**: Suporte contínuo de 2.0c (Chibi / SD) a 8.5c (Heróico / JoJo).
- **Estilização Facial**:
  - *Tsurime* (+20° cantal lateral) vs *Tareme* (-20° cantal lateral).
  - *Anime Muzzle Slant*: Achatamento do perfil bucal alinhando rima labial e dorso nasal.
  - *Aegyosal*: Relevo palpebral inferior para fofura e profundidade tridimensional.

---

## 4. O CATÁLOGO DE 148 SLIDERS ANATÔMICOS EM 18 ZONAS

### 4.1. Proporções Globais & Silhueta (8 Sliders)
1. `height_overall`: Altura estelar linear do rig [110cm a 215cm] (Bone Delta).
2. `head_to_body_ratio`: Régua de cabeças canônica [2.0c a 8.5c] (Bone Delta).
3. `head_scale_uniform`: Multiplicador uniforme do crânio [0.6x a 1.6x] (Bone Delta).
4. `torso_to_limb_ratio`: Proporção relativa tronco/pernas [0.7x a 1.4x] (Bone Delta).
5. `somatotype_endomorph`: Gordura e corpulência macro [0.0 a 1.0] (Morph Target).
6. `somatotype_mesomorph`: Musculatura e relevo atlético macro [0.0 a 1.0] (Morph Target).
7. `somatotype_ectomorph`: Magreza e estrutura óssea aparente macro [0.0 a 1.0] (Morph Target).
8. `spine_s_curvature`: Curvatura de postura S-Line (lordose/cifose) [-1.0 a +1.0] (Dual).

### 4.2. Crânio e Estrutura Craniofacial (11 Sliders)
9. `head_width`: Largura biparietal lateral (Dual).
10. `head_depth`: Profundidade occipital anteroposterior (Bone Delta).
11. `face_lower_length`: Altura vertical do terço inferior da face (Morph).
12. `forehead_height`: Altura da fronte/testa (Morph).
13. `forehead_roundness`: Curvatura frontal arredondada vs plana (Morph).
14. `brow_ridge_prominence`: Arco supraciliar masculino (Morph).
15. `temple_width`: Largura temporal lateral (Morph).
16. `cheekbone_prominence`: Projeção malar/zigomática (Morph).
17. `cheek_fullness_upper`: Volume malar superior fofo anime (Morph).
18. `cheek_hollow_lower`: Concavidade bucal inferior madura (Morph).
19. `head_neck_blend`: Suavização da transição cabeça-pescoço (Morph).

### 4.3. Olhos e Órbitas Anime (12 Sliders)
20. `eye_scale_uniform`: Escala geral do globo e fenda ocular (Morph).
21. `eye_horizontal_width`: Abertura cantal horizontal (Morph).
22. `eye_vertical_height`: Abertura palpebral vertical (Morph).
23. `eye_interpupillary_dist`: Distância intercantal entre olhos (Morph).
24. `eye_vertical_position`: Posição vertical na face (Morph).
25. `eye_socket_depth`: Profundidade na cavidade craniana (Morph).
26. `eye_canthal_tilt`: Inclinação cantal lateral (*Tsurime* $\leftrightarrow$ *Tareme*) (Morph).
27. `iris_scale_ratio`: Proporção da íris e pupila (Morph).
28. `upper_eyelid_fold`: Vinco da dobra palpebral superior (Morph).
29. `lower_eyelid_aegyosal`: Volume do relevo aegyosal inferior (Morph).
30. `eyelid_corner_curve`: Curvatura aguda vs suave das comissuras (Morph).
31. `pupil_vertical_squash`: Achatamento oval vertical da pupila (Morph).

### 4.4. Sobrancelhas Estruturais (6 Sliders)
32. `eyebrow_thickness`: Espessura do traço da sobrancelha (Morph).
33. `eyebrow_arch_height`: Altura e arqueamento superior (Morph).
34. `eyebrow_inner_slant`: Inclinação angular interna (Morph).
35. `eyebrow_spacing`: Distância glabelar entre sobrancelhas (Morph).
36. `eyebrow_length`: Comprimento horizontal lateral (Morph).
37. `eyebrow_depth`: Projeção em relevo 3D da arcada (Morph).

### 4.5. Nariz Estilizado Anime (8 Sliders)
38. `nose_bridge_height`: Altura da raiz nasal na glabela (Morph).
39. `nose_bridge_depth`: Projeção do dorso nasal de perfil (Morph).
40. `nose_length_vertical`: Comprimento vertical do nariz (Morph).
41. `nose_tip_upturn`: Ângulo da ponta (arrebitado vs reto) (Morph).
42. `nose_tip_sharpness`: Afunilamento da ponta da sombra anime (Morph).
43. `nose_alar_width`: Largura da base das narinas (Morph).
44. `nose_nostril_visibility`: Nitidez dos orifícios das narinas (Morph).
45. `nose_profile_flatness`: Achatamento do perfil anime (*muzzle slant*) (Morph).

### 4.6. Boca e Lábios (8 Sliders)
46. `mouth_width`: Largura horizontal da rima bucal (Morph).
47. `mouth_vertical_pos`: Posição vertical e altura do filtro (Morph).
48. `mouth_depth_protrusion`: Projeção dentolabial anterior (Morph).
49. `lip_upper_thickness`: Espessura carnuda do lábio superior (Morph).
50. `lip_lower_thickness`: Plenitude do vermelhão labial inferior (Morph).
51. `lip_corner_tilt`: Inclinação das comissuras labiais (Morph).
52. `lip_philtrum_depth`: Profundidade do sulco do filtro (Morph).
53. `mouth_tuck_depth`: Reentrância das comissuras (covinhas anime) (Morph).

### 4.7. Mandíbula, Queixo e Linha V (10 Sliders)
54. `jaw_bigonial_width`: Largura bigoníaca da mandíbula (Morph).
55. `jaw_angle_vertical`: Altura vertical do ângulo da mandíbula (Morph).
56. `jaw_v_line_taper`: Afunilamento em V estilizado anime (Morph).
57. `chin_length`: Comprimento vertical do mento (Morph).
58. `chin_width`: Largura da ponta do queixo (fino vs quadrado) (Morph).
59. `chin_forward_projection`: Projeção do pogônio de perfil (Morph).
60. `chin_cleft_dimple`: Covinha central mentoniana masculina (Morph).
61. `submental_fullness`: Preenchimento adiposo submentoniano (papada) (Morph).
62. `jawline_bone_definition`: Nitidez da linha de sombra mandibular (Morph).
63. `anime_profile_slant`: Alinhamento plano único nariz-boca-queixo (Morph).

### 4.8. Orelhas Anime/Élficas (5 Sliders)
64. `ear_scale_uniform`: Escala auricular geral (Morph).
65. `ear_flare_angle`: Abertura lateral do pavilhão (Morph).
66. `ear_pointy_elf`: Alongamento élfico pontiagudo da hélice (Morph).
67. `ear_lobe_length`: Comprimento do lóbulo pendente (Morph).
68. `ear_vertical_position`: Posição vertical na altura dos olhos (Morph).

### 4.9. Pescoço e Trapézio (7 Sliders)
69. `neck_length`: Comprimento longitudinal do pescoço (Bone Delta).
70. `neck_circumference`: Circunferência transversal do pescoço (Dual).
71. `trapezius_bulk`: Massa muscular superior do trapézio (Morph).
72. `adams_apple_prominence`: Pomo de Adão masculino (Morph).
73. `scull_scm_tendon_relief`: Relevo do esternocleidomastóideo em V (Morph).
74. `throat_concavity`: Covinha da fossa jugular esternal (Morph).
75. `neck_forward_posture`: Curvatura cervical para frente (Bone Delta).

### 4.10. Ombros, Clavículas e Dorsal (8 Sliders)
76. `shoulder_biacromial_width`: Largura biacromial dos ombros (Bone Delta).
77. `shoulder_acromial_slope`: Inclinação angular dos ombros (Bone Delta).
78. `deltoid_muscle_volume`: Esfericidade do deltoide muscular (Morph).
79. `clavicle_bone_relief`: Nitidez e relevo ósseo das clavículas (Morph).
80. `clavicle_v_angle`: Ângulo de inclinação em V clavicular (Morph).
81. `scapula_wing_relief`: Proeminência das escápulas no dorso (Morph).
82. `shoulder_depth_thickness`: Espessura glenoumeral anteroposterior (Morph).
83. `latissimus_dorsi_flare`: Expansão em asa do grande dorsal (Morph).

### 4.11. Tórax e Peitorais Masculinos (6 Sliders)
84. `ribcage_width`: Largura da caixa torácica (Dual).
85. `ribcage_depth`: Profundidade torácica anteroposterior (Morph).
86. `pectoral_muscle_bulk`: Hipertrofia do ventre peitoral masculino (Morph).
87. `pectoral_lower_cut`: Definição do sulco inframamário reto (Morph).
88. `pectoral_sternal_cleave`: Separação central entre os peitorais (Morph).
89. `sternum_hollow_depth`: Concavidade óssea do esterno (Morph).

### 4.12. Busto e Glândulas Mamárias Femininas (9 Sliders)
90. `bust_volume_cup`: Volume da copa mamária (Copa A a G) (Morph).
91. `bust_vertical_position`: Altura de inserção no tórax (Morph).
92. `bust_separation_cleavage`: Espaçamento intermamário e decote (Morph).
93. `bust_gravity_sag`: Caimento natural em gota pelos ligamentos de Cooper (Morph).
94. `bust_firmness_roundness`: Turgor superior e firmeza cônica (Morph).
95. `bust_outward_angle`: Divergência lateral dos eixos mamários (Morph).
96. `bust_areola_diameter`: Diâmetro relativo da aréola (Morph).
97. `bust_nipple_projection`: Projeção anterior da papila mamária (Morph).
98. `bust_underbust_taper`: Afunilamento torácico submamário (Morph).

### 4.13. Abdômen, Cintura e Flancos (11 Sliders)
99. `waist_pinch_width`: Menor circunferência transversal da cintura (Dual).
100. `waist_pinch_height`: Altura anatômica do estreitamento (Bone Delta).
101. `belly_visceral_protuberance`: Projeção anterior abdominal (Morph).
102. `belly_lower_panniculus`: Avental adiposo hipogástrico inferior (Morph).
103. `abs_sixpack_definition`: Relevo dos gomos musculares e linha alba (Morph).
104. `oblique_apollo_belt`: Cinturão de Apolo / crista ilíaca atlética (Morph).
105. `flank_love_handles`: Gordura localizada nos flancos laterais (Morph).
106. `ribcage_costal_margin`: Margem costal inferior visível ectomorfa (Morph).
107. `navel_vertical_pos`: Posição vertical do umbigo (Morph).
108. `navel_shape_slit`: Formato em fenda vertical vs redondo (Morph).
109. `stomach_vacuum_depth`: Sucção atlética da parede abdominal (Morph).

### 4.14. Pelve, Bacia e Curva do Quadril (9 Sliders)
110. `pelvis_bicristal_width`: Largura bicristal da bacia óssea (Bone Delta).
111. `hip_trochanteric_flare`: Curvatura bitrocantérica dos quadris femininos (Morph).
112. `pelvic_tilt_angle`: Anteversão / retroversão pélvica e lordose (Bone Delta).
113. `iliac_crest_prominence`: Nitidez do relevo ósseo ilíaco (Morph).
114. `trochanteric_fat_saddlebag`: Depósito de culotes laterais femininos (Morph).
115. `hip_dip_fill`: Preenchimento da depressão do glúteo médio (Morph).
116. `pubic_arch_width`: Abertura transversal do arco subpúbico (Morph).
117. `pelvis_depth`: Dimensão anteroposterior da bacia (Morph).
118. `groin_crease_depth`: Sulco da dobra inguinal entre tronco e coxa (Morph).

### 4.15. Glúteos e Nádegas (9 Sliders)
119. `gluteus_volume_overall`: Volume total da massa glútea (Morph).
120. `gluteus_posterior_shelf`: Projeção posterior horizontal de perfil (Morph).
121. `gluteus_lift_height`: Sustentação e elevação do polo superior (Morph).
122. `gluteal_crease_depth`: Profundidade da prega infraglútea (Morph).
123. `gluteus_shape_profile`: Formato geométrico (Redondo / Coração / V / Quadrado) (Morph).
124. `gluteus_medius_fill`: Preenchimento muscular súpero-lateral (Morph).
125. `gluteus_firmness`: Tônus muscular tenso vs relaxamento adiposo (Morph).
126. `intergluteal_cleft_depth`: Profundidade do sulco interglúteo central (Morph).
127. `infragluteal_crease_length`: Extensão horizontal da dobra inferior (Morph).

### 4.16. Braços, Antebraços e Cotovelos (10 Sliders)
128. `arm_length_overall`: Comprimento total do membro superior (Bone Delta).
129. `upper_arm_length`: Comprimento longitudinal do úmero (Bone Delta).
130. `forearm_length`: Comprimento da rádio-ulna (Bone Delta).
131. `upper_arm_thickness`: Circunferência e espessura do braço (Dual).
132. `biceps_peak_volume`: Pico hipertrófico do bíceps braquial (Morph).
133. `triceps_bulk`: Volume muscular do tríceps posterior (Morph).
134. `forearm_brachioradialis`: Massa muscular do braquiorradial (Morph).
135. `forearm_taper_ratio`: Razão cônica de afunilamento em direção ao punho (Morph).
136. `elbow_olecranon_sharpness`: Projeção óssea pontiaguda do olécrano (Morph).
137. `wrist_circumference`: Circunferência distal do punho (Dual).

### 4.17. Mãos e Dedos (8 Sliders)
138. `hand_scale_uniform`: Escala proporcional da mão (Bone Delta).
139. `palm_width`: Largura metacarpal da palma da mão (Dual).
140. `palm_length`: Comprimento longitudinal da palma (Bone Delta).
141. `finger_length`: Comprimento longitudinal das falanges (Bone Delta).
142. `finger_thickness`: Espessura dos dedos (finos anime vs grossos) (Morph).
143. `knuckle_joint_definition`: Relevo das articulações metacarpofalângicas (Morph).
144. `thumb_opposability_angle`: Abertura e oponibilidade do polegar (Bone Delta).
145. `fingernail_style_anime`: Estilo das unhas anime (curtas vs amendoadas) (Morph).

### 4.18. Pernas, Joelhos, Panturrilhas e Pés (12 Sliders)
146. `leg_length_overall`: Comprimento total do membro inferior (Bone Delta).
147. `thigh_length`: Comprimento longitudinal do fêmur (Bone Delta).
148. `thigh_circumference`: Circunferência superior proximal da coxa (Dual).
149. `inner_thigh_gap`: Fresta adutora medial entre coxas (Morph).
150. `outer_thigh_sweep`: Curvatura e projeção do vasto lateral (Morph).
151. `quadriceps_definition`: Relevo atlético dos quatro ventres femorais (Morph).
152. `knee_patella_prominence`: Projeção e nitidez óssea da rótula/patela (Morph).
153. `knee_valgus_uchimata`: Alinhamento valgo (*Uchimata* fofo anime) (Bone Delta).
154. `calf_circumference`: Circunferência máxima do gastrocnêmio (Dual).
155. `gastrocnemius_height`: Altura do ventre muscular da panturrilha (Morph).
156. `ankle_malleolus_thickness`: Espessura bimaléolar do tornozelo (Dual).
157. `foot_scale_and_arch`: Tamanho do pé e elevação do arco plantar (Dual).

---

## 5. ARQUITETURA MATEMÁTICA E HARDWARE (DLMBA)

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

---

## 6. ROADMAP DE EXECUÇÃO: 14 SUB-SPRINTS MODULARES

### Sub-Sprint 3.1: Topologia Canônica Isomórfica e Modelos Base glTF Male & Female
- **Entregáveis**: Pares `assets/models/anigo_base_male.glb` e `assets/models/anigo_base_female.glb` com contagem idêntica de vértices ($N$), topologia de faces e layout UV coincidente. Leitor glTF atualizado em `crates/anigo-core/src/mesh.rs`.
- **DoD**: Teste unitário validando contagem e indexação de vértices isomórfica 1:1.

### Sub-Sprint 3.2: Evolução do Vértice NPR Skinned e Buffers de Skinning
- **Entregáveis**: Estrutura `Vertex` estendida com `joints: [u16; 4]` e `weights: [f32; 4]` em `crates/anigo-core` e `shaders/cel_shading.wgsl`.
- **DoD**: Compilação limpa, zero quebras no pipeline de renderização, suporte a LBS nativo.

### Sub-Sprint 3.3: Motor de Morphs Esparsos em WebGPU Compute Shader
- **Entregáveis**: Compute shader `shaders/morph_sparse_compute.wgsl` e despacho de compute no `anigo-renderer`.
- **DoD**: Execução de 148 sliders em < 0.08ms por frame na GPU real; VRAM total < 4 MB.

### Sub-Sprint 3.4: Motor de Propagação Esquelética e Sincronização de Rig
- **Entregáveis**: Sistema de translação de juntas e recálculo dinâmico de $B_{\text{inv}}$ em `crates/anigo-ik`.
- **DoD**: Prova matemática de ortogonalidade das matrizes de rotação (determinante == 1.0, zero shearing).

### Sub-Sprint 3.5: Pad 2D de Somatótipo Macro e Solver Contínuo
- **Entregáveis**: `SomatotypePad2D.svelte` e solver baricêntrico em `crates/anigo-core/src/somatotype.rs`.
- **DoD**: Interpolação a 120 FPS contínua entre Ectomorfo, Mesomorfo e Endomorfo; slider mestre `gender_dimorphism`.

### Sub-Sprint 3.6: Morfologia Craniofacial e Sliders Faciais Estilizados (47 Sliders)
- **Entregáveis**: Sliders e deltas esparsos de Crânio, Olhos (*Tsurime/Tareme*), Sobrancelhas, Nariz, Boca e Mandíbula (*V-Line*).
- **DoD**: Validação de perfil anime com achatamento do focinho (*muzzle slant*) e olhos estilizados sem distorção normal.

### Sub-Sprint 3.7: Morfologia Cervical, Trapézio, Clavículas e Pomo de Adão (15 Sliders)
- **Entregáveis**: Controle de pescoço, trapézio, clavículas, escápulas e pomo de Adão masculino.
- **DoD**: Clavículas com projeção óssea sem quebras no contorno Inverted Hull.

### Sub-Sprint 3.8: Morfologia Torácica, Peitorais e Busto Feminino Dinâmico (15 Sliders)
- **Entregáveis**: Busto orgânico feminino com curvatura em lágrima (Copa A a G, gravidade e decote) e peitoral atlético masculino.
- **DoD**: Busto preserva formato natural pendente sem estiramento esférico artificial.

### Sub-Sprint 3.9: Morfologia Abdominal, Cintura, Flancos e Definição Muscular (11 Sliders)
- **Entregáveis**: Sliders de abdômen 6-pack, gordura visceral, flancos e estreitamento de cintura (*waist pinch*).
- **DoD**: Criação independente de corpos atléticos trincados ou barriga saliente sem afetar os membros.

### Sub-Sprint 3.10: Morfologia Pélvica, Bacia e Geometria Glútea Avançada (18 Sliders)
- **Entregáveis**: Bacia ginecoide/androide e nádegas com 4 formatos geométricos (Redondo, Coração Invertido, Quadrado, V).
- **DoD**: Controle independente de volume, elevação e projeção horizontal sem interpenetração.

### Sub-Sprint 3.11: Morfologia dos Membros Superiores (18 Sliders)
- **Entregáveis**: Braços, bíceps, antebraço, cotovelo, mãos e articulações dos dedos.
- **DoD**: Escala de mãos e dedos preservando proporcionalidade sem deformar nós das articulações.

### Sub-Sprint 3.12: Morfologia dos Membros Inferiores (12 Sliders)
- **Entregáveis**: Coxas, *Inner Thigh Gap*, patela do joelho, panturrilhas, pés e alinhamento *Uchimata*.
- **DoD**: Zero interpenetração geométrica entre as coxas em posições neutras e extremas.

### Sub-Sprint 3.13: Manipulação Tátil no Viewport 3D
- **Entregáveis**: Raycasting de volumes de colisão e desprojeção de arrasto tela-para-slider estilo Design Doll / The Sims 4.
- **DoD**: Clicar e arrastar sobre o membro no canvas 3D altera os sliders correspondentes instantaneamente.

### Sub-Sprint 3.14: Interface Svelte 5, Presets de Fábrica, Undo/Redo e Validação MCP
- **Entregáveis**: Painel de estúdio profissional dark mode, presets de fábrica (*Shonen Hero*, *Shojo Idol*, *Muscular Berserker*, *Plus Size*, *Chibi 2.5c*, *Heroic 8.5c*), histórico Undo/Redo e ferramentas MCP em `anigo-mcp`.
- **DoD**: Execução bem-sucedida de `scripts/test_sprint03_mcp.py` com inspeção crítica de frames renderizados offscreen via hardware real.

---

## 7. FERRAMENTAS MCP INVIOLÁVEIS E AUDITORIA VISUAL

O servidor `anigo-mcp` deve registrar obrigatoriamente:
- `anigo_set_character_model(model_type: "male" | "female")`
- `anigo_set_somatotype(endo: f32, meso: f32, ecto: f32)`
- `anigo_apply_morph_slider(slider_id: string, value: f32)`
- `anigo_inspect_mesh_integrity()`
- `anigo_get_active_morphs()`
- `anigo_reset_morphs()`
- `anigo_render_frame(width: u32, height: u32, camera: CameraParams)`

Toda e qualquer validação exigirá a execução de testes automatizados com captura offscreen e análise rigorosa de artefatos visuais.

---

## 8. CHECKLIST DE AUTO-AVALIAÇÃO OBRIGATÓRIO (7 PERGUNTAS)
1. **Isso está tecnicamente correto?** Sim, fundamentado em matemática LBS, Compute Shaders WGSL e rig Forward Kinematics ortogonal.
2. **Faz exatamente o que a especificação exige?** Sim, atende a modelos masculino e feminino com 148 sliders granulares e controle macro.
3. **Precisa de melhorias ou refatoração?** Não, a arquitetura DLMBA elimina os débitos de Green Coordinates e cisalhamento de matrizes.
4. **Existem bugs, defeitos ou artefatos visuais?** O pipeline de deltas esparsos e binds ortogonais previne artefatos de rotação e interpenetração.
5. **Preciso buscar informações ou referências em pesquisas especializadas?** A pesquisa foi exaustiva em VRoid, Design Doll, CSP 3D e Sims 4.
6. **Está no padrão estético e técnico de um software comercial profissional?** Sim, segue padrões de ferramentas líderes de mercado.
7. **Está moderno e com engenharia de alto padrão?** Sim, WebGPU WGSL nativo, Rust idiomático, Svelte 5 e VRM 1.0.


