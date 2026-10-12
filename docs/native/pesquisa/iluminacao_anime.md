# Iluminação para anime 3D / cel: pesquisa e proposta do workspace Iluminação nativo (out/2026)

Escopo: o que um estúdio de anime 3D espera de um workspace de iluminação (rig de luz, direção da luz controlando o terminador e o face SDF, sombra projetada estilizada, ambiente, mobilidade, tipos de luz) e como os editores de referência (Unreal, Blender, Unity) expõem isso. Fecha com uma PROPOSTA de escopo em fases e a divisão nuvem × local.

Legenda de confiança: **[V]** = fonte oficial/primária lida nesta pesquisa; **[S]** = secundário (blog, fórum, vídeo, trecho de busca); **[C]** = conjectura/conhecimento próprio, não reverificado. Nada aqui foi inventado para parecer fonte.

Relação com o que já existe no repo: `AUDITORIA_SHADING_ILUMINACAO.md` (P1-05: "sistema de luz é uma única direcional: sem shadow map, sem ambiente real, sem SDF facial"), `pesquisa/cel_shading_estado_da_arte.md` (itens T-3.7 HDR/tonemap e T-3.8 shadow map de 1 luz + sombra do cabelo no rosto, ambos ainda ⬜) e `CEL_ENGINE_FASE1.md` (face SDF com espelhamento já feito, T-3.4). O ambiente hemisférico (céu/chão), a tinta de sombra da luz e a saturação já existem no núcleo (`StylizedLight`), no uniform (`LightUniform`) e no shader; o que faltava era a UI e a sombra projetada.

## 1. Como os jogos/estúdios de anime 3D iluminam

### 1.1 Guilty Gear Xrd (Motomura, GDC 2015) [V]
Fonte: https://www.ggxrd.com/Motomura_Junya_GuiltyGearXrd.pdf (PDF lido, 34 páginas).
- "Each character has his or her own dedicated light vector that lights their idle pose in the best way." A luz é **por personagem**, não uma luz de cena realista: o artista escolhe a direção que dá o melhor desenho de sombra para aquele modelo.
- "This lighting doesn't change frame by frame in the battle scene, but in cut scenes, it animated along with the character to get the best result each and every frame." Em cutscene a luz é **animada** com o personagem (luz como parte da direção de arte, não do mundo).
- "We used a channel from the Vertex Colors as an offset on the Threshold. This made it possible for the artist to make certain areas on the mesh get darker more easily." O terminador é `N·L > limiar`, e o limiar é deslocado por vértice. No ANIGO isso é o canal G (`shadow_shift`) do vértice de 72 B, já suportado.
- "While the Tint texture defines how dark it gets when shaded. Multiplying the two textures, we get the shaded color." A cor da sombra é **base × tinta** (a tinta de sombra tem matiz próprio, p.ex. avermelhada na pele). No ANIGO: `shade_color` do material × `light.shadow_color` (tinta da luz) com hue-shift OKLab.
- A palestra não trata de sombra projetada (o texto lido não menciona "cast shadow"); no Xrd a luz serve ao terminador, e o "shadow-melting" do Zato é efeito, não luz [S: reportagem da 4Gamer republicada].
Consequência de produto: a UI de luz principal precisa ser **direta e artística** (direção, cor de luz, tinta de sombra, limiar), com o "vetor de luz" arrastável, e não um editor de luzes físicas.

### 1.2 Genshin Impact / Honkai Star Rail [S]
- Face SDF (já implementado no motor, T-3.4): "Genshin Impact's facial shadow texture stores shadow coverage information for left-side lighting at angles 0° ~ 180° on the xz plane. All we have to do is take the shadow texture, flip it if light is coming from the right, and compare its colour value with `FdotL` (i.e. forward xz direction • light xz direction)" [S: README de NoiRC256/URPSimpleGenshinShaders]. Conclusão para o workspace: **a direção da luz comanda o rosto**; mexer no azimute tem de mudar o desenho da sombra do rosto em tempo real, e o gizmo precisa mostrar o azimute relativo à frente do personagem.
- O mesmo README lista como limitação que sombras projetadas por outros objetos no rosto "may disappear or become unstable when the camera is close" porque o shader soma um offset de posição de shadow map para esconder sombra suja, e lista como desejo "auto 2D hair shadow on face (very important, it is very difficult to produce good looking shadow result using shadowmap)". Ou seja: a sombra do cabelo no rosto por shadow map é possível mas **frágil**; precisa de bias/offset por normal, filtro estilizado e controle de força, e idealmente de um modo "sem auto-sombra no rosto" [S].
- StarRail NPR Shader (stalomeow) declara suporte a "self shadow and per-object shadow, up to 16 on the same screen" [V: página oficial do projeto; só essa frase é confirmada, a página não detalha parâmetros]. Roda self-shadow como recurso próprio, o que confirma que o jogo trata sombra projetada em personagem como uma camada separada do terminador.
- Rim "sharp const width" (Blue Protocol/Genshin) [S, README acima]: rim de largura constante em pixels, diferente do Fresnel. O ANIGO tem Fresnel + backlight (shader atual); rim por profundidade ficou como Fase avançada.

### 1.3 Hi-Fi Rush [S]
Tópicos da palestra GDC 2024 (slides atrás de login): deferred toon, "toon light", shadow maps, GI e face shadow. Não há conteúdo lido além do índice; só confirma que shadow maps + um modelo de luz próprio de toon coexistem em produção.

### 1.4 Regra de três pontos [S]
Fonte: https://2822digitalcinematography.wordpress.com/three-point-lighting-the-basic-setup/ e StudioBinder. Key = luz principal, de um lado da câmera, cria modelagem; fill = lado oposto, mais suave, regra geral 3:1 (key:fill); back/rim = atrás do sujeito, destaca cabelo e ombros e separa do fundo; ângulo da key entre 15° e 70°, mais comum 45° [S]. Em anime 3D com UMA direcional: key = a direcional; **fill = o ambiente hemisférico** (levantar o ambiente reduz o contraste da sombra; no shader atual o ambiente MULTIPLICA o tom da sombra — `sombra = cor_de_sombra × tinta × hemisfério × ambiente` —, e para o material padrão do personagem um contraste luz/sombra de ~3:1 pede `ambient ≈ 1,5`, conta conferida por teste em `lighting/rigs.rs`); rim = o rim Fresnel do material com `backlight` dependente de `dot(L, -V)` (já no shader: `rim_backlight = max(dot(L,-V)*0.6+0.4, 0)`), logo um preset "contraluz/rim" é uma direção com `L` atrás do sujeito e `rim_intensity` alto. Isso é o que dá para fazer honestamente com o contrato atual.

## 2. Tipos de luz e mobilidade nos editores de referência

### 2.1 Unreal Engine [V]
Fonte: https://dev.epicgames.com/documentation/unreal-engine/directional-lights-in-unreal-engine
- Mobilidade: **Static** ("cannot be changed in game", baked), **Stationary** (sombras/bounce de geometria estática baked; "allows for the light to change color and intensity in game, but, it does not move"), **Movable** ("totally dynamic and allows for dynamic shadowing"; o mais caro).
- Propriedades da Directional Light: **Source Angle** (padrão 0,5357°, o ângulo do sol; controla a largura da penumbra), **Source Soft Angle**, **Indirect Lighting Intensity**, **Contact Shadow Length** ("Length of screen space ray trace for sharp contact shadows"), **Dynamic Shadow Distance** ("how far Cascaded Shadow Map dynamic shadows will cover... measured from the camera"), **Num Dynamic Shadow Cascades**, Atmosphere Sun Light, Light Shaft Occlusion/Bloom.
Lição: três parâmetros de sombra definem a UX — **distância** (onde a sombra existe), **suavidade** (ângulo da fonte) e **contato** (sombra fina de contato). Para um personagem de anime, "distância" vira "raio de cobertura em torno do alvo da câmera" e "suavidade" vira a dureza do degrau.

### 2.2 Blender [V]
Fonte: https://docs.blender.org/manual/en/latest/render/lights/light_object.html
- Tipos: **Point** (Power em Watts, Radius, Soft Falloff), **Sun** (Strength em W/m², **Angle**), **Spot** (Power, Radius, Soft Falloff, Spot **Angle**, **Blend**, Show Cone), **Area** (Shape: Rectangle/Square/Disk/Ellipse, Size X/Y). Todos têm **Temperature**, Color, Exposure, Normalize.
- A página lida não descreve opções de sombra do EEVEE; de uma busca anterior [S]: o EEVEE expõe "Contact Shadows" por luz e o tamanho físico da luz alarga a penumbra ("Increasing a light's physical size in Blender widens the penumbra") [S: Varsity Tutors].
Lição: **Temperature** (Kelvin) ao lado de Color é padrão; o Sun tem um único controle de "dureza" (Angle).

### 2.3 Unity [V]
Fonte: https://docs.unity3d.com/Manual/class-Light.html
- Tipos: Directional, Point, Spot, Area (Rectangle/Disc). Modo: Realtime, Mixed, Baked.
- Sombra: **Shadow Type** (No, Hard, Soft — "Soft shadows also tend to reduce the blocky aliasing effect from the shadow map"), **Strength** (0–1, "how dark the shadows are"), **Bias** (0–2, padrão 0,05, "false self-shadowing artifacts"), **Normal Bias** (0–3, padrão 0,4), **Near Plane**, Cookie.
Lição: os parâmetros que o usuário precisa mexer para matar acne/"peter panning" são exatamente **bias** e **normal bias**, mais **strength** e hard/soft. O ANIGO expõe os quatro.

### 2.4 O que isso significa para o ANIGO
Todos os três têm Directional + Point + Spot + Area; no anime 3D de produção (Xrd, Genshin) a luz é **uma direcional por personagem**. O contrato congelado do ANIGO tem uma luz. Luzes extras (ponto/spot/rect) exigiriam: array de luzes no uniform, N sombras e mudança de todos os cálculos de terminador (que hoje usam um único `L`). Decisão: **não** nesta fase; a UI mostra os tipos como desabilitados com explicação (como o Cenário já faz).

## 3. Técnica de sombra projetada para toon

### 3.1 Shadow map ortográfico para uma direcional [V]
Fonte: https://learnopengl.com/Advanced-Lighting/Shadows/Shadow-Mapping
- Luz direcional = projeção **ortográfica** + `lookAt` a partir de um ponto ao longo da direção da luz, olhando o centro da cena.
- **Acne**: causada pela resolução do mapa; bias simples `current - bias > closest` e **bias por inclinação** `max(0.05 * (1 - dot(N, L)), 0.005)`.
- **Peter panning**: o bias descola a sombra; mitigação clássica: desenhar o mapa com **front-face culling**.
- **PCF**: média de 3×3 (ou mais) amostras de comparação vizinhas; **fora do frustum** da luz a sombra deve ser 0.
### 3.2 Estabilidade (sem tremer) [S]
Fonte: Microsoft Learn "Common Techniques to Improve Shadow Depth Maps" e GameDev.net: mover a projeção da luz em **incrementos de um texel** (`min /= worldUnitsPerTexel; floor; *= worldUnitsPerTexel`) mantém as bordas da sombra sem cintilar quando a câmera/objeto se mexe; ajustar a caixa a uma **esfera envolvente** (e não à caixa alinhada) torna o tamanho do texel constante sob rotação [S: Alex Tardif, catlike coding].
### 3.3 Estilização [S/C]
Sombra projetada em toon não é gradiente: o fator PCF (0..1) passa por um **degrau** (`smoothstep(0,5−h, 0,5+h)`) e entra no **mesmo fator** que o terminador (`toon_factor *= 1 − sombra`), para que a sombra projetada tenha a mesma **cor de sombra** do terminador (base × tinta, hue-shift). Isso é o que o GGXrd faz para o terminador ("Multiplying the two textures, we get the shaded color") estendido à sombra projetada [C: a extensão é decisão de projeto, não citação]. "Dureza" = largura do `smoothstep`; "Força" = o quanto a sombra projetada escurece (Unity: Strength).
### 3.4 Sombra do cabelo/cabeça no rosto [S]
Duas abordagens na indústria: (a) shadow map com offset/bias agressivo no rosto (a que o README do Genshin shader descreve e critica pela instabilidade em close) e (b) sombra 2D de cabelo por deslocamento em espaço de tela ou por SDF. Nesta fase: **(a)**, porque o mesmo mapa serve ao corpo todo (cabelo, cabeça no pescoço, braço no tronco); (b) fica como avançado.

## 4. Convenções de UX de editores de luz

- **Gizmo de direção** [S/C, convenção observada em Blender/Unreal/Unity, não verificada em doc nesta pesquisa]: a luz direcional é uma seta/arco no viewport partindo do alvo; arrastar rotaciona. No ANIGO: uma **esfera de direção** (esfera wireframe com meridianos/paralelos centrada no personagem) com um **disco de sol** no ponto `centro + L·raio`; arrastar o disco muda azimute/elevação; Shift = passo de 15°; a seta de luz aponta do sol ao centro.
- **Preview ao vivo**: a mudança aparece enquanto se arrasta e vira **um** comando no soltar (mesmo padrão do gizmo do Cenário: `CoreSession::preview` + `commit_preview`).
- **Luz presa à câmera**: padrão de visualizadores de modelo ("studio light"); tem de ser opção explícita, porque ela mente sobre a luz do projeto. No ANIGO: modo de **preview** com deslocamento relativo à câmera + botão **"Aplicar ao projeto"** que grava a direção resultante (um comando, com undo).
- **Presets de rig**: lista nomeada com miniatura/ícone; aplicar é **um** passo de undo (Batch).
- **Temperatura**: slider em Kelvin com faixa 1000–40000 K (a faixa do algoritmo de Tanner Helland [V], https://tannerhelland.com/2012/09/18/convert-temperature-rgb-algorithm-code.html) convertida em RGB; o slider escreve a COR da luz (não é um segundo estado).

## 5. Tonemap/exposição (T-3.7) [S/C]
O shader aplica ACES e `exp2(EV)` dentro do fragment (`tonemap_aces`, linha final do `cel_shading.wgsl`); isso dessatura e desloca matiz das cores chapadas do anime. A correção estrutural (alvo HDR + passe final com tonemap neutro Khronos PBR Neutral) muda o formato de alvo do contrato e exige olho humano; **fora do escopo desta entrega**. O que dá para fazer aditivamente: nada que mude o quadro congelado por padrão. Mantido ⬜ e documentado. A leitura da página do Khronos PBR Neutral nesta pesquisa só trouxe a especificação em equações (sem o GLSL de referência), então não há código copiado nem validado aqui.

## 6. PROPOSTA de escopo em fases

### Fase 1 — MVP (FEITO; tudo validado na nuvem)
- Workspace **Iluminação** nativo: viewport com o personagem + o cenário do projeto (cel canônico), e painel com a **luz principal** (sol): gizmo de direção arrastável (azimute/elevação, esfera de direção), cor, intensidade, **temperatura (K)**, ambiente (intensidade + cor de céu + cor de chão), **tinta e saturação de sombra**, dureza do terminador (material do personagem), rim.
- **Presets de rig**: 3 pontos, contraluz/rim, dia, golden hour, noite, interior, estúdio — cada um um `Batch` de `SetLight` (+ `SetMaterialParams` do rim) = um passo de undo.
- **Luz presa à câmera** (preview) e "Aplicar ao projeto".
- Persistência por comandos do núcleo com undo/redo; arrasto com preview + um comando ao soltar; Ctrl+Z/Y roteados.
- Cenários de captura `iluminacao-*` com variação de azimute/elevação/intensidade e prova de que terminador/rim respondem; smoke xdotool.

### Fase 2 — Completo (FEITO, aditivo e desligado por padrão; resultado, provas e limites em `ILUMINACAO_UI.md` §3)
- **Shadow map de 1 luz**, ADITIVO e DESLIGADO por padrão: passe de profundidade da luz (ortográfico, ajustado ao conteúdo com snap de texel), PCF 3×3/5×5 com degrau estilizado, bias constante + por inclinação + offset por normal, força, distância, resolução 1024/2048/4096; cor de sombra = a do terminador. Bindings 18–20 do grupo `cel` com neutros 1×1 quando desligado → quadros congelados byte a byte idênticos.
- **Sombra do cabelo/cabeça no rosto** como consequência (auto-sombra no mesmo mapa), com controle de força.
- Testes: layout do uniform no contrato (`gen_contract_fixtures.mjs`), `check:wgsl`, paridade WGSL×Rust das funções puras (degrau, offset por normal, projeção), matriz da luz (todos os pontos do conteúdo dentro de [−1,1]² e profundidade em [0,1]), golden desligado = golden anterior, PNGs com sombra visível.

### Fase 3 — Avançado (NÃO feito; desenho aqui)
- Luzes extras (ponto/spot/retângulo) e **light linking**: exige `LightUniform[]` + loop no shader + um mapa por luz; mudança de contrato v2.
- **CSM** e **contact shadows** (UE: "Contact Shadow Length" [V]) para cenas grandes; sombra 2D de cabelo por SDF (§3.4-b).
- **Volumes de post/look** por cena/plano (exposição, tonemap neutro, LUT) — depende de T-3.7/T-3.10.
- **Sky** procedural / IBL estilizado; rim por profundidade (§1.2).
- Luz por **plano/shot** (animável, como o Xrd em cutscene [V]): depende do workspace Animação.

## 7. Divisão nuvem × local
| Item | Nuvem (Xvfb + lavapipe) | Local (GPU real + pessoa) |
|---|---|---|
| Matemática (direção↔az/el, temperatura→RGB, matriz de luz, PCF, bias) | testes unitários + paridade WGSL×Rust | — |
| Comandos/undo/redo/persistência | testes do núcleo e do studio | — |
| Contrato/WGSL (`fixtures:check`, `check:wgsl`, naga) | verde | — |
| Quadro congelado desligado | hashes de `golden_frames` iguais | repetir em GPU real |
| Aparência (PNGs de azimute/elevação/intensidade/sombra) | inspecionados | **gosto**: dureza, bias, cor de sombra |
| Sensação do gizmo e do arrasto | xdotool com passos discretos | mouse real, taxa de atualização |
| Desempenho do passe de sombra | não mede (lavapipe) | 2048²/4096² em GPU real |
| Viewport Svelte/Tauri legado | **não executável aqui** | validar que o binding 18–20 não quebrou o app web |
