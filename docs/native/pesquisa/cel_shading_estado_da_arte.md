# Motor cel ANIGO: o que falta e roadmap (pesquisa out/2026)

Escopo: li cel_shading, inverted_hull, face_sdf, anime_eye, postprocess_dof (morph_sparse_compute é só deformação), render_graph.rs, trechos de headless.rs e render_contract_v1.json. Nada foi alterado em /home/user/anigo.
Limite da pesquisa: o conteúdo de 2025-2026 que consegui verificar é pouco. Os talks (GDC/CEDEC) de miHoYo/Hi-Fi Rush/Strive não têm slides abertos. Marquei [V] = li a fonte; [S] = só trecho/snippet de busca; [C] = conhecimento próprio, não reverificado.

## 1. O que já existe (não repetir)
- Cel: half-lambert, 1/2/3 degraus analíticos, sombra com hue-shift OKLab, shade_tex/second shade (MToon), AO e shadow-shift por vertex color (R/G), rim fresnel com back-light, matcap, emissão, spec com "angel ring" por jitter, face SDF (theta da luz), olho (parallax + 2 highlights procedurais).
- Hull invertido: extrusão em clip space (espessura constante em pixels), máscara por vertex color B, textura de largura, skinning igual ao cel.
- DoF (CoC físico, bokeh círculo/hexágono), morph esparso em compute. MSAA 4x, alvo Rgba8Unorm, depth24plus.
- Grafo canônico: depth_prepass, face_shadow_sdf, opaque_cel, hair_cloth, outline, postprocess. O contrato congela passes, bindings e uniforms. O plano tem aliasing de slots e testes.

## 2. Lacunas e bugs prováveis achados na leitura (alto valor, baixo custo)
1. Tonemap ACES + linear→sRGB dentro do fragment do cel. A cena sai em LDR/sRGB 8 bits antes do pós, então bloom, grading e DoF operam em gama. ACES também desloca matiz e dessatura as cores chapadas da paleta anime. Mover para um alvo HDR (Rgba16Float) e tonemap/grade num passe final, com tonemap neutro (AgX ou PBR Neutral) opcional por material. Exige visão humana.
2. `ramp_sample` é amostrada e descartada (comentário "kept for validation"). A rampa 2D (linha por material) não é usada. É o mecanismo central de Genshin/Star Rail e UTS [S]. Ou implementar, ou remover a textureSample.
3. Face SDF sem espelhamento: threshold depende só de cos(theta). Falta escolher entre uv e uv espelhado conforme o sinal de sin(theta) (luz à esquerda ou à direita), senão a sombra é simétrica. Usa `camera.model[0/2]` do personagem, não o osso da cabeça. O fator fixo 0.72 ignora a cor de sombra do material. Tudo isso é matemática testável em Rust.
4. DoF: `textureSample` depois de `return` condicional por pixel (radius_px < 0.5) viola uniformidade de derivadas. Tint/Chrome rejeitam e naga avisa. Trocar por `textureSampleLevel(...,0.0)`. O commit "skip dof on GL" sugere que já houve dor aqui.
5. Hull: usa as normais de malha. Em arestas duras (queixo, roupa) o casco abre buracos. Sem normal suavizada, sem compensação de FOV/distância (espessura é fixa em pixels, o oposto do traço de anime, que afina ao longe), cor única (sem tinta por material), sem taper.
6. Tangente do cabelo/spec = cross(N, up). Não é a direção do fio, então o angel ring oscila. `ao_intensity` (params3.w) não é usado. Não existe nenhum shadow map: cabelo não projeta sombra no rosto, e Hi-Fi Rush, Star Rail e Genshin têm essa sombra.
7. Canais de vertex color esgotados (R AO, G shadow shift, B outline, A spec/rim). Novos dados (normal suave, ID, taper) pedem UV1 ou textura estilo "ILM/lightmap". O layout é congelado em 72 B no contrato, no compute de morph e no TS.

## 3. Catálogo de técnicas
Colunas: fonte | custo GPU | encaixe no grafo ANIGO | validação (A = sem GPU; B = golden/SSIM em software; H = olho humano).

### Contorno
| Técnica | Fonte | Custo | Grafo | Valid. |
|---|---|---|---|---|
| Normal suavizada assada (UV1 ou vertex color, espaço tangente) | Ameye [V], Delt06 outline wiki [S]: normal em UV2, flag clip-space | zero em runtime | import da malha (CPU) + `outline` lê o atributo | A (hull de cubo sem frestas, testável em Rust) |
| Largura por distância/FOV (afinar ao longe), taper G-pen, pico por vertex/textura | GGXrd: largura variável por vertex color, apagável [V]; lilToon: largura por vertex color e distância [S] | ~0 | uniform do `outline` (params2.z livre) | A (função pura + golden), H para o gosto |
| Cor de linha por material (tinta = base escurecida/saturada, não preto) | GGXrd, UTS [S] | ~0 | uniform + `outline` | H |
| Linhas internas por UV "beams" alinhados a eixo, sem serrilhado em close | GGXrd [V] | ~0 | textura estática no `opaque_cel` | H |
| Arestas internas/crease por pós: Sobel/Roberts em depth+normal+ID, com modulação por profundidade e ângulo rasante | Ameye [V] (Roberts 4 amostras, Sobel 9, limiar × depth, máscara de fresnel); OpenMoonRay ToonDisplayFilter [S] | 1 passe fullscreen, 1-2 buffers extras | novos recursos `normal_id` (MRT) e passe `edge_post` entre outline e postprocess | A (kernel em Rust vs WGSL), B, H |
| ID de objeto/material como fonte de descontinuidade | Ameye "custom discontinuity source" [V] | MRT | atenção: ID não se resolve em MSAA; usar textureLoad por amostra (`texture_multisampled_2d`) ou pré-passe 1x | A para layout; B |
| Jump Flood para linhas largas/glow | Golus [S] | log2(N) passes | opcional, só para destaque de seleção | B |
Decisão: manter o hull como linha principal (GGXrd preferiu hull a pós por controle e preview [V]) e acrescentar um pós só para creases e linhas de ID. Aresta, vinco e sobreposição de materiais é o que o hull não faz.

### Sombreamento
| Técnica | Fonte | Custo | Grafo | Valid. |
|---|---|---|---|---|
| Rampa 2D por material (linha = material, coluna = NdotL), com versão dia/noite | UTS, Star Rail [S]; slot já existe | 1 sample | `opaque_cel` (reativar `toon_ramp`) | A (ler rampa em Rust), H |
| Mapa "ILM/lightmap" (threshold, spec mask, spec shape, ramp row em canais) | GGXrd usa vertex color como offset de threshold [V]; Genshin/HoYo reverse-eng. [S] | 1 sample | `opaque_cel` + novo binding | A (layout), H |
| Normais customizadas e transferência de normais (cabeça por esfera/proxy) | GGXrd editou normais "em toda feature", rosto à mão [V] | offline | ferramenta no import, em anigo-vrm/core | A (teste: normais do rosto variam suavemente) |
| Luz própria por personagem, desacoplada da cena | GGXrd [V]: vetor de luz dedicado por personagem | zero | uniform de luz já existe | H |
| Shadow map de 1 luz + degrau estilizado (PCF duro/ordered), "hair shadow" no rosto | Hi-Fi Rush GDC 2024 [S, tópicos: toon light, shadow maps, face shadow]; Star Rail: self-shadow e per-object [V] | 1 passe depth + 1 sample | novo `shadow_map` antes de `opaque_cel`; reaproveita o pipeline de depth_prepass | A (matriz de luz ajustada ao bounding box, testável), B, H |
| CSM | só se houver cenário grande | N passes | fora do escopo de personagem | B |
| Sombra externa fina (outer shadow, 2º NdotL deslocado) | ArtStation Genshin breakdown [S] | 1 termo | `opaque_cel` | H |
| Half-lambert, hue-shift, tint de sombra | já existem | | | |

### Especular, rim, pele
| Técnica | Fonte | Custo | Grafo | Valid. |
|---|---|---|---|---|
| Angel ring estável: tangente do fio (cartões do crate anigo-hair) + deslocamento vertical em espaço de vista | UTS AngelRing [S] | ~0 | atributo de tangente, `opaque_cel` | A (função da tangente), H |
| Rim dependente de luz + rim por profundidade (offset de depth em screen-space, borda uniforme) | Blender Genshin recreation: "rim parece contorno, espessura uniforme" [S] | 2-4 depth taps | `opaque_cel` precisa de depth 1x; mais prático no `edge_post` | H |
| SSS fake de pele: sombra com tinta avermelhada | GGXrd: sombra de pele tem tinta vermelha por transmissão [V] | ~0 | já coberto por shade_color/hue-shift; só fornecer presets | H |
| Matcap | já existe | | | |

### Cabelo, olhos
| Técnica | Fonte | Custo | Grafo | Valid. |
|---|---|---|---|---|
| Cabelo translúcido sobre olho/sobrancelha (hair-through): stencil, ou máscara de olho + cabelo com dither | Fórum Unity: sobrancelha grava stencil, cabelo é excluído [S]; HoyoToon suporta stencil de cabelo [S]; Star Rail "transparent front hair" [V] | 1 passe pequeno | `hair_cloth` depois de `opaque_cel`. Stencil exige depth24plus-stencil8 (mudar `depth_format` no contrato, 4 B/px igual). Alternativa sem stencil: alvo R8 `eye_mask` | A (grafo/ordem), H |
| Sombra de pálpebra/cílios sobre íris, íris SDF, highlights por textura | já há parallax e highlights procedurais | | `opaque_cel` | H |
| Linha do nariz por ângulo de vista | [C] | ~0 | máscara por vertex + `dot(V, face_forward)` | H |

### Pós e qualidade
| Técnica | Fonte | Custo | Grafo | Valid. |
|---|---|---|---|---|
| Bloom estilizado (threshold suave + dual-Kawase, tinta quente) | Star Rail traz bloom+tonemap próprios [V] | ~6 passes de meia res | cadeia `post_a/post_b` (já existem no contrato) | A (pesos somam 1, energia), H |
| Color grading por LUT 3D 32³ | [C] | 1 sample | `postprocess` | A (LUT identidade = identidade) |
| Halftone/hatch, grain, aberração cromática | [C] | 1 passe | `postprocess` | B/H |
| Céu/nuvens anime | [C] | | cena, fora do personagem | H |
| MSAA 4x (já) | SMAA paper [V]: MSAA é o padrão-ouro mas pesa em deferred | | hull e geometria saem limpos | |
| SMAA/FXAA só para a camada de linhas de pós | Toon RP wiki [S] | 3 passes | depois de `edge_post` | B |
| TAA | Toon RP wiki [S]: jitter alto → flicker; Blender NPR [V]: filtros contíguos sofrem instabilidade temporal | histórico + velocity | NÃO recomendado agora: linhas finas e skinning limitado sem vetor de movimento | |
| Supersampling no export | Godot docs [S] | N× pixels | `headless`: renderizar 2-3× e reduzir | A (box filter), B |
| AOVs para compositing (flat, sombra, linha, spec, ID) | Blender NPR migrou para compositing multi-estágio [V] | MRT | passes escrevem em anexos separados | A (contrato), B |

## 4. Referências principais
- GGXrd, Motomura, GDC 2015 [V]: https://www.ggxrd.com/Motomura_Junya_GuiltyGearXrd.pdf. Pontos: vertex color como offset de threshold, luz por personagem, normais editadas, textura de tinta de sombra (base × tint), hull com largura por vertex color, linhas internas por UV beams, ~40k triângulos, sem normal map, "Kill everything 3D".
- Hi-Fi Rush, GDC 2024 [S]: https://gdcvault.com/play/1034251/3D-Toon-Rendering-in-Hi e https://80.lv/articles/the-making-of-hi-fi-rush-s-3d-toon-rendering-style. Só tópicos (deferred toon, comic shader, toon light, shadow maps, GI, face shadow). Slides atrás de login.
- Ameye, "5 ways to draw an outline" [V]: https://ameye.dev/notes/rendering-outlines/. Golus, Jump Flood [S]: https://bgolus.medium.com/the-quest-for-very-wide-outlines-ba82ed442cd9.
- Delt06 URP toon (normais suavizadas em UV2, clip-space) [S]: https://github.com/Delt06/urp-toon-shader/wiki/Outline
- StarRailNPRShader (arquivado 21/03/2025, v2.10.3) [V]: https://github.com/stalomeow/StarRailNPRShader e https://srshader.stalomeow.com. Lista: cabelo frontal translúcido, self-shadow e per-object shadow, bloom e tonemap próprios, normais suavizadas no import. GPL-3.0: ler como referência, não copiar código.
- URPSimpleGenshinShaders (SDF de rosto, instabilidade de sombra projetada em close) [S]: https://github.com/NoiRC256/URPSimpleGenshinShaders. UTS3 (AngelRing, rampas, 3 cores) [S]: https://docs.unity3d.com/Packages/com.unity.toonshader@0.15/manual/index.html. lilToon [S]: https://github.com/lilxyzw/lilToon.
- Blender NPR (2024-2026) [V]: https://projects.blender.org/blender/blender/issues/120403 e https://devtalk.blender.org/t/eevee-npr-prototype-feedback/37098. Conclusão da equipe: shading no material, filtros em compositing multi-estágio. Isso valida AOVs + passes de pós.
- SMAA, Jimenez et al. [S]: https://www.iryoku.com/smaa/downloads/SMAA-Enhanced-Subpixel-Morphological-Antialiasing.pdf. Toon RP AA [S]: https://github.com/Delt06/toon-rp/wiki/Anti%E2%80%90Aliasing
- Ecossistema: o crates.io mostra wgpu/naga na série 30.x (ago/2026) e o projeto está no wgpu 24. Não atualizar no meio do roadmap.

## 5. Restrições do ANIGO que moldam o roadmap
- Contrato único (render_contract_v1.json) + headless Rust + viewport TS (webgpu_renderer.ts) + fallback GLSL WebGL2. Todo passe/binding novo muda os quatro lugares e o `check_wgsl.mjs`. Um passe novo = JSON + grafo + headless + TS + (decidir) GLSL.
- Não há GPU nem naga no ambiente de dev (check_wgsl.mjs diz isso). O `build.rs` instala um shim do naga-cli. Proposta: `naga` como dev-dependency na versão do wgpu 24 e um `cargo test` que parseia e valida todos os `.wgsl` (inclusive uniformidade de derivadas). Isso pega o item 2.4.
- Não há ICD Vulkan nesta máquina (/usr/share/vulkan/icd.d vazio). Se o CI tiver Mesa lavapipe, o headless roda em software e permite golden com SSIM/tolerância. Não verifiquei.

## 6. Roadmap em ordem
Legenda: [A] valida sem GPU; [B] precisa de renderização (software ok); [H] precisa de olho humano local.

Fase 0, fundação e correções (1-2 PRs)
1. Teste cargo que valida todos os WGSL com naga; corrigir DoF para `textureSampleLevel`. [A]
2. Núcleo de matemática em Rust (anigo-core::npr) com goldens iguais ao WGSL: face SDF com espelhamento e mapeamento theta→threshold, largura de contorno por distância/FOV/taper, rampa 2D, janela de PCF. Um golden por função, no estilo do `circle_of_confusion` que já existe nos 3 lugares. [A]
3. Ou ativar a rampa 2D, ou remover a amostra morta; usar `ao_intensity`; cor de sombra no fator do rosto. [A+H]

Fase 1, linha (maior impacto visual por esforço)
4. Normal suavizada assada no import (espaço tangente, soldagem por posição), nova entrada no hull. Teste: cubo/cilindro sem frestas. [A]
5. Largura por distância/FOV + taper + cor de linha por material (uniform do outline). [A para função, H para ajuste]
6. Passe `edge_post` (Roberts/Sobel em depth+normal+ID) com modulação por profundidade e ângulo rasante; buffer MRT `normal_id`. Decisão de ID: pré-passe 1x ou textureLoad por amostra no MSAA. [A para kernel, B, H]

Fase 2, luz e sombra
7. HDR + tonemap neutro no passe final, mantendo paridade com o frame congelado (flag no contrato). [B, H]
8. Rampa 2D por material + mapa ILM (threshold/spec/ramp row). [A layout, H]
9. `shadow_map` de uma luz ajustado ao personagem; sombra do cabelo no rosto com offset; degrau estilizado. Reaproveitar o pipeline de depth_prepass. [A para matriz/limites, B, H]
   **Atualização (workspace Iluminação):** feito de forma ADITIVA e desligada por padrão — passe de profundidade da luz que reusa o `vs_main` do cel, bindings 18–20, PCF + degrau estilizado, auto-sombra (cabelo/cabeça no rosto) pelo mesmo mapa; goldens intactos. Ver `docs/native/ILUMINACAO_UI.md` §3 (o que falta: gosto/bias/GPU real, cabelo real, CSM, luzes extras).
10. Normais de rosto transferidas de proxy (ferramenta offline). [A+H]

Fase 3, cabelo e olho
11. Tangente de fio (anigo-hair) → angel ring estável. [A+H]
12. Hair-through: decidir stencil (muda `depth_format`) vs. máscara `eye_mask`; sombra de pálpebra e highlights por textura. [A para ordem do grafo, H]

Fase 4, pós e saída
13. Bloom estilizado, LUT 3D, grain/CA/halftone na cadeia `post_a/post_b`. [A para energia/LUT identidade, H]
14. SMAA só na camada de linhas, SSAA no export, AOVs (flat/sombra/linha/spec/ID) para compositing em camadas. [A contrato, B]
15. Só depois: TAA (precisa de vetor de movimento com skinning/morph), JFA para destaques.

## 7. O que dá para fechar sem GPU e o que não dá
- Sem GPU: itens 1, 2, 4, 5 (função), 6 (kernel), 8 (layout), 9 (matrizes), 13 (LUT/energia), contrato, plano do grafo (render_graph.rs já testa aliasing e ordem; adicionar `shadow_map`, `edge_post`, `bloom` com testes de dependência), tamanhos de uniform.
- Software GPU (lavapipe) se disponível: imagem de referência com tolerância para linhas, sombras e bloom. Isso prova ausência de buracos e de vazamento, não que ficou bonito.
- Olho humano obrigatório: espessura e gosto da linha, limiar de crease, paleta e hue-shift, ponto do ramp, aparência de bloom e tonemap, hair-through, flicker no movimento. Sugestão: uma cena fixa (3 poses × 3 luzes × 2 distâncias de câmera) com captura automática para revisão em lote, em vez de ajustar slider por slider.
