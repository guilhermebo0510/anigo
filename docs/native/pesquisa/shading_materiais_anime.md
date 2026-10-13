# Pesquisa: authoring de materiais toon/anime e UX de editores de material — e o que levar ao workspace "Shading" do ANIGO (out/2026)

Legenda (a mesma de `ue5_level_editor.md`): **[V]** = documentação/PDF oficial lido nesta sessão (conteúdo carregado, não só snippet);
**[S]** = visto só em trecho de busca ou em fonte secundária (blog, ArtStation, vídeo); **[C]** = conjectura/conhecimento prévio, NÃO verificado agora.
Estado em 2026-10-12.

Limitações (honestidade): o Firecrawl limitou a taxa (429) várias vezes; a documentação do lilToon só foi lida na página "Shadow" (ja_JP) e as páginas de
rim/outline do lilToon deram 404 na URL que tentei, então o que digo sobre rim/outline do lilToon vem do que as outras fontes (MToon, UTS) confirmam [V] ou de
memória [C]. Não há slides abertos do miHoYo: tudo sobre Genshin é **engenharia reversa de terceiros** [S]. O manual do UTS lido é o **0.5.0-preview** (o
mesmo conjunto de parâmetros do UTS2/UTS3). Nada daqui foi validado em GPU real.

Contexto local lido no repo [V] (molda a proposta):
- `StylizedMaterial` (anigo-core) já tem o essencial toon: `base/shade/outline_color`, `shadow_threshold/smoothness`, `toon_steps`, `hue_shift`, especular (`specular_color/size/softness/offset/spec_intensity/spec_power`), rim (`rim_color/intensity/spread`), `outline_*` (largura, opacidade, suavidade, bias), MToon (emissão, 2ª sombra, matcap, flags de textura), face SDF (`face_sdf_enabled`, `face_shadow_offset/smoothness`), olho anime e o grupo `NprParams` (rampa 2D, contorno com normais suavizadas/falloff/taper/tinta, espelho do SDF facial) — **todos os recursos de Fase 1 do cel estão desligados por padrão**.
- `MaterialPatch` + `Command::SetMaterialParams{material_id, patch}` (undo exato por `inverse_of`) e `CoreSession::{apply, preview, commit_preview}` (arrasto sem poluir histórico) — é o mecanismo de "Material Instance" já existente. Não existe `AddMaterial`/`SetNodeMaterial`: um projeto tem hoje 1 material (`canonical_default`) e `NodeSlot.overrides` (3 campos) para instâncias do Cenário.
- `ToonMaterial` (anigo-assets, `.anigomat.json`): superconjunto estruturado (blocos base/shade/ilm/spec/rim/matcap/outline/face_sdf/emission), texturas por papel com espaço de cor, **sem** o grupo `npr`; `to_stylized()` perde rampa/ILM/máscaras. Materiais são ativos da Biblioteca (`AssetKind::Material`), com miniatura CPU (esfera) hoje.
- Renderer: a textura de rampa é **uma só por renderer** (256×4 do contrato, 4 linhas fixas); os slots de textura MToon (bindings 6–15) ancoram sempre o neutro 1×1; só o mapa de face SDF é ancorado de verdade (renderer-global, `set_face_sdf_map`). Não existe transparência/recorte (`alpha`) em `StylizedMaterial`.

---------------------------------------------------------------------------------------------------

## 1. Como cada sistema autora um material toon

### 1.1 MToon / VRM 1.0 [V] — `VRMC_materials_mtoon-1.0` (vrm-specification, lido na íntegra)
- **Duas cores e um limiar:** `baseColorFactor` × textura e `shadeColorFactor` × `shadeMultiplyTexture`; `shading = dot(N,L) + shadingShiftFactor (+ shadingShiftTexture·scale)`; `shading = linearstep(-1+toony, 1−toony, shading)`; `color = lerp(shade, base, shading) · lightColor`. Ou seja: **deslocamento (threshold)** e **toony (largura da penumbra)** são os dois botões centrais, defaults `shift = 0`, `toony = 0.9`.
- **Rim:** matcap (`matcapFactor`, `matcapTexture`) **+** rim paramétrico `pow(saturate(1 − N·V + lift), fresnelPower)` × `parametricRimColorFactor` (defaults cor 0, power 5, lift 0), × `rimMultiplyTexture` (máscara por UV), × `lerp(1, lighting, rimLightingMixFactor)` — o rim pode ser ou não afetado pela luz.
- **Contorno:** `outlineWidthMode` (none / worldCoordinates em metros / screenCoordinates em fração da altura da tela), `outlineWidthFactor`, `outlineWidthMultiplyTexture` (apagar o contorno onde não se quer), `outlineColorFactor`, `outlineLightingMixFactor` (a cor da linha pode herdar a sombra/luz). A spec recomenda respeitar a largura do artista, e menciona o comportamento "afina com a distância" do VRM0 como opcional.
- **Emissão, GI (`giIntensityFactor`), `transparentWithZWrite`, `renderQueueOffsetNumber`, UV animation** (scroll/rotação com máscara).
- Tradução ANIGO: já mapeado 1:1 em `ToonMaterial` (blocos `shade`, `rim`, `outline`, `emission`, `matcap`, `uv_anim`).

### 1.2 Unity Toon Shader (UTS2/UTS3), manual 0.5.0-preview [V]
Menus numerados que viram **grupos recolhíveis**: 1 Basic Shader Settings · 2 Basic Three Colors and Control Maps (Base/1st Shade/2nd Shade + mapas) · 3 Shading Step and Feather · 4 HighColor · 5 RimLight · 6 MatCap · 7 AngelRing · 8 Emissive · 9 Outline · 10 LightColor Contribution · 11 Environmental Lighting.
- **Step & Feather** (a unidade de authoring de sombra): `BaseColor_Step` (limite base/sombra), `BaseShade_Feather` (suavidade), `ShadeColor_Step` e `1st2nd_Shades_Feather` (limite e suavidade entre 1ª e 2ª sombra). Dois workflows: `DoubleShadeWithFeather` (Position Maps fixam a sombra ao modelo) e `ShadingGradeMap` (mapa de grau de sombra).
- **HighColor:** cor, `Power` (tamanho), `Specular Mode` (gloss) vs círculo duro, blend Aditivo/Multiplicativo, máscara de sombra sobre o highlight.
- **RimLight:** cor, `Power`, `Inside Mask`, `FeatherOff` (corta o desfoque → rim duro), `LightDirection Mask` (rim só do lado da luz) e **rim antipodal** (do lado oposto), máscara por UV.
- **Outline:** método de casco invertido por material; modo `Normal Direction` vs `Position Scaling`; **`Baked Normal for Outline`**: normais suavizadas assadas em textura para dar contorno macio a objetos de aresta dura — exatamente o `outline_smooth_normals` do ANIGO; `Outline Sampler` (textura de força), `Offset Outline with Camera Z`.
- Ideia de UX a copiar: **Feather = 0 vira "cel duro", cresce vira "ilustração"**.

### 1.3 lilToon — página "Shadow" (ja_JP) [V]; resto [C]
- Até **3 sombras** com cor individual, cada uma com **blur (`ぼかし`)** próprio (0 = anime duro → aumenta para ilustração → realista), **`境界の色`/`境界の幅` = cor e largura da borda da sombra** (efeito de SSS: faixa colorida na transição), **AO Map** com remapeamento min/max (onde a textura é escura a sombra aparece mais fácil), **máscara de força** por canal R (apagar sombra no rosto) ou "achatar normais", influência da luz ambiente na cor da sombra.
- Fórmula publicada: `indirectCol = lerp(main, shadowTex.rgb, shadowTex.a) · shadowColor`; opcionalmente `· main` (mais contraste); `lerp(indirect, main, envLight·envStrength)`.
- Tradução ANIGO: `shade_color`, `mtoon_second_shade_*` (2ª sombra), `ao_intensity`; **borda colorida da sombra e 3ª sombra não existem** no `StylizedMaterial` (ficam como "avançado/futuro").

### 1.4 Guilty Gear Xrd / Arc System Works — Motomura, GDC 2015 [V] (PDF + texto lido por inteiro)
- Princípio: **"Kill everything 3D"**; cada elemento é "escolha intencional, não resultado de cálculo"; **feedback instantâneo** no software de modelagem foi "key".
- O shader cel é um `step` com **3 controles: limiar, vetor de luz, normal**. O **limiar** é deslocado por um canal de **vertex color** (oclusão pintada à mão; pode ser 0 = sempre sombra). **Luz própria por personagem** (sem iluminação global). **Normais editadas à mão** (rosto principalmente).
- **Cor de sombra = base × textura de "tint"** (dois blocos de cor): a sombra mostra o quão "sólido" é o material (pele ganha tinta avermelhada por transmissão) — "o artista escolhe a cor de luz e de sombra livremente". Isto fundamenta a UX "cor de sombra independente + derivar de HSV da base".
- **Contorno por casco invertido** com largura variável por vertex color (podendo sumir); linhas internas por UVs alinhados a eixo. Preferiram hull a pós-processo **porque dá para pré-visualizar no viewport de modelagem**.
- Implicação de UX: o painel precisa de **preview ao vivo e fiel ao render final**, e de controles de intenção (limiar, cores, linha) — não de dezenas de sliders físicos.

### 1.5 Genshin / miHoYo [S] (engenharia reversa de terceiros; **não há fonte primária aberta**)
- **ILM / lightmap por material** (canais: máscara/intensidade de especular, deslocamento de sombra, forma do especular, índice de linha da rampa/ID; o canal A costuma ser o ID da linha da rampa) [S: GabrielToonShader manual cita "ILM Texture", vídeos de breakdown]. O ANIGO já tem o bloco `ilm` em `ToonMaterial` (canais configuráveis), sem consumo no renderer.
- **Shadow ramp** por material (rampa 2D com linhas dia/noite): a cor do gradiente dentro/fora da sombra dá a "SSS artificial" — faixa amarelada no lado iluminado e avermelhada dentro da sombra [S: bjayers.com]. Para o ANIGO = **rampa 2D por material** (Fase 1 do cel) com um editor de gradiente por linha.
- **Face SDF:** textura onde R cobre ângulos 0–180° e G 180–360° (ou um mapa espelhado) [S: Adrian Mendez]; criação com 8 frames de sombra (SDF generator) [S: vídeo 2AM]. ANIGO: `face_sdf_enabled`, `face_sdf_mirror_center` (espelhamento) e `set_face_sdf_map`.
- **Sombra externa fina** (2º `NdotL` deslocado, mais duro, tom parecido) [S] ≈ 2ª sombra do MToon.
- **Contorno:** linha fina escura, às vezes **colorida pelo material** [S] ≈ tinta por material (`outline_ink_*`). Rim "tipo contorno" de espessura uniforme é pós-processo, não Fresnel [S] — não coberto pelo `rim_*` (Fresnel).
- **Metal:** mistura de difusa com **matcap**/gradiente por `N·(V+L)` [S].

### 1.6 Blender (Shader to RGB, Color Ramp) [V parcial — trechos da doc 5.x]
- O nó **Shader to RGB** (EEVEE) converte o resultado de um BSDF em cor para usar com **Color Ramp**: "a color ramp on the output of a diffuse BSDF can be used to create a flexible toon shader" [V]; a doc avisa que quebra a PBR (sem transmissão/refração). O **Color Ramp** é o editor de gradiente de referência: paradas (stops) arrastáveis com cor, interpolação (linear/constante/ease), posição numérica. → serve de modelo para o editor de rampa por linha (stops, interpolação constante = degrau duro).
- Pipeline típico do Genshin no Blender [S]: Diffuse → Shader to RGB → ColorRamp (sombra) + Solidify invertido (contorno).

### 1.7 UX de editores de material (Unreal Material Instance, Blender) [V parcial]
- **UE Material Instance:** o instance expõe só os **parâmetros que o material-pai marcou**, **agrupados por `Group`** (todos os parâmetros com o mesmo grupo caem na mesma categoria do editor) [V]; cada parâmetro tem **checkbox de override** (valor herdado × sobrescrito) e botão de reset; o editor tem **preview em esfera/malha** com troca de primitiva [V/C]. Tradução: grupos recolhíveis, **"modificado" destacado** (como o painel do Personagem), **reset por parâmetro e por grupo**, preview com troca de forma (esfera / objeto).
- **Blender:** material com preview em esfera, painéis colapsáveis por grupo, gradiente embutido, **Material Presets** e copiar/colar de material entre slots [C].
- Conclusões de UX: (1) agrupar por **intenção** (Base/Sombra, Rampa, Especular, Rim, Matcap, Contorno, Face, Emissão), não por uniform; (2) mostrar o que foi **alterado em relação ao padrão**; (3) arrastar = pré-visualiza ao vivo, soltar = 1 passo de undo; (4) A/B de antes/depois; (5) presets com miniatura renderizada com o **mesmo renderer** do viewport.

---------------------------------------------------------------------------------------------------

## 2. O que isso pede do ANIGO — gaps encontrados

| # | Gap | Solução proposta | Onde |
|---|---|---|---|
| G1 | `ToonMaterial.to_stylized()` perde `npr` (rampa/contorno/SDF) → preset não guarda o que o Shading edita | campo aditivo `npr: Option<NprParams>` em `ToonMaterial` (+ `to_stylized`/`from_stylized`) | anigo-assets |
| G2 | Rampa só tem as 4 linhas fixas do contrato; **sem editor de gradiente por linha** | gradiente por linha **opcional** (paradas pos/valor) em `NprParams`, textura 256×4 montada por material no renderer (mesmo formato; WGSL e contrato intactos); linha sem parada = linha do contrato | anigo-core `npr`, anigo-renderer |
| G3 | `MaterialPatch` só aplica campo a campo: faltam patch "completo" (colar/preset/reset de grupo) e diff | `MaterialPatch::from_material` e `MaterialPatch::diff` (puros, testados) | anigo-core |
| G4 | Instâncias do Cenário só têm 3 overrides; Modelagem só 3 cores por objeto | `NodeOverrides.material: Option<MaterialPatch>` (aditivo, omitido quando vazio, via `SetNodeProps`); Modelagem: `StylizedMaterial` opcional por objeto no `Document` (histórico por snapshot) | core `level.rs`, studio |
| G5 | Texturas por material (base/sombra/matcap/ILM/emissão) **não são amostradas** no renderer (slots neutros) | **não fazer agora** (exige binding por material + contrato). A UI mostra o slot e seu estado e só liga o que o renderer realmente usa (mapa SDF facial); referências de textura vão no preset `.anigomat.json` | docs |
| G6 | Transparência/recorte (`alpha_mode`, `alpha_cutoff`) existe em `ToonMaterial.base`, mas não em `StylizedMaterial` nem no renderer | seção da UI mostra o modo/limiar do preset (dado do `.anigomat.json`) **desabilitado com explicação**; sem renderizar | docs |
| G7 | Miniatura de ativo `Material` na Biblioteca é uma esfera CPU | `CelThumbnailRenderer` passa a renderizar o material numa esfera pelo renderer canônico (shader ball) | studio `library_ui/cel_thumb.rs` |

---------------------------------------------------------------------------------------------------

## 3. PROPOSTA de escopo (fases) e divisão nuvem × local

### Fase MVP (nuvem: tudo)
1. Workspace `Shading` nativo: painel esquerdo **Alvo** (lista de materiais do projeto/personagem, nós do Cenário, objetos da Modelagem), painel direito **Parâmetros agrupados**, centro **viewport** com o objeto/personagem e um **shader ball** (esfera) renderizados pelo `CelPreview` canônico.
2. Grupos reais ligados ao `StylizedMaterial`: **Base/Sombra** (cor base, cor de sombra, limiar, suavidade, degraus, hue-shift, 2ª sombra, AO), **Especular**, **Rim**, **Contorno** (largura, cor, opacidade, suavidade, bias + NPR: normais suavizadas, falloff, distância de referência, piso, taper, tinta), **Rampa 2D** (mistura + linha), **Face SDF** (liga, offset, suavidade, espelho), **Matcap**, **Emissão**, olho.
3. Edição **sem poluir histórico:** arrasto → `CoreSession::preview`; soltar → `commit_preview` (1 passo). Undo/redo no `CoreSession`; Ctrl+Z/Y roteados.
4. Reset por parâmetro / por grupo / total; indicador "modificado".
5. Luz presa à câmera (liga/desliga), ângulo da luz livre para ver o terminador.

### Fase Completa (nuvem: tudo; gosto = local)
6. **Editor de gradiente por linha** da rampa (paradas arrastáveis, interpolação constante/linear, 4 linhas, reset, pré-visualização da faixa) → G2.
7. **Paleta:** swatches do projeto, derivar a cor de sombra da base por HSV (saturação/valor/hue), hue-shift, harmonias (complementar, análoga, tríade, tétrade), salvar/aplicar paleta do projeto.
8. **Presets** de material (`.anigomat.json` como ativo da Biblioteca, com miniatura em shader ball) → G1, G7; presets embutidos (Pele anime, Cabelo, Tecido, Metal, Pano de sombra dura/macia).
9. **Copiar/colar** material entre slots; **A/B** lado a lado (snapshot "antes" × atual) → G3.
10. Fontes de material para os três workspaces (Personagem: material do projeto; Cenário: overrides completos por instância; Modelagem: material por objeto) → G4.
11. Seletor de **mapa de face SDF** da Biblioteca (textura → R8 → `set_face_sdf_map`) e gerador procedural de demonstração.

### Fase Avançada (fora desta entrega; documentada)
12. Texturas por material (base/sombra/matcap/ILM/rampa-textura/emissão) no renderer → G5; ILM (canais de threshold/spec/forma/ID).
13. Alpha mode/cutoff, `transparent_with_zwrite` → G6; 3ª sombra e borda colorida da sombra (lilToon); rim antipodal e rim por profundidade (Genshin).
14. Material Instance de verdade (herança de um "material-pai" com overrides por parâmetro); `AddMaterial`/`SetNodeMaterial` no `Command`.
15. Pintura de vertex color (limiar/AO/largura de linha, GGXrd) — pertence ao workspace Personagem/Modelagem.

### Divisão nuvem × local
- **Nuvem (feito e testado aqui):** matemática pura (diff/patch, rampa por paradas, HSV/harmonias), serialização de preset, comandos com undo exato, renderer em Vulkan por software (lavapipe) com PNGs mostrando que cada controle muda a imagem (rampa, rim, contorno, face SDF a vários ângulos), smoke xdotool em Xvfb próprio, i18n pt/en, testes.
- **Local (precisa de pessoa/GPU real):** **gosto** dos defaults e dos presets, sensação do arrasto e dos controles de gradiente, legibilidade do painel em telas reais/HiDPI, desempenho com GPU real (shader ball + objeto + A/B no mesmo quadro), comportamento de Ctrl+Z/Y com teclado real, cor (sRGB/linear) comparada a referências de anime, fidelidade das paletas/harmonias à sua percepção, e qualquer comparação com o render final em hardware diferente.

---------------------------------------------------------------------------------------------------

## 4. Referências
- VRMC_materials_mtoon-1.0 [V]: https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md
- Unity Toon Shader 0.5.0-preview manual [V]: https://docs.unity3d.com/Packages/com.unity.toonshader@0.6/manual/instruction.html
- lilToon, "Shadow" (ja_JP) [V]: https://lilxyzw.github.io/lilToon/ja_JP/color/shadow.html
- Motomura, "Guilty Gear Xrd's Art Style: The X Factor Between 2D and 3D", GDC 2015 [V]: http://ggxrd.com/Motomura_Junya_GuiltyGearXrd.pdf (publicado em https://www.arcsystemworks.com/guilty-gear-xrds-art-style-the-x-factor-between-2d-and-3d-talk-from-gdc-2015-is-now-available-online/)
- Genshin: Adrian Mendez, character shader breakdown [S] https://adrianmendez.artstation.com/projects/wJZ4Gg ; Ben Ayers, "Recreating the Genshin Impact Shader" [S] https://bjayers.com/blog/9oOD/blender-npr-recreating-the-genshin-impact-shader ; 2AM, SDF de rosto [S] https://www.youtube.com/watch?v=-Ot5TzGMwrA ; GabrielToonShader (ILM) [S] https://github.com/gabriel01913/GabrielToonShader/blob/main/Manual/GTShader_Manual.md
- Blender Shader To RGB [V parcial]: https://docs.blender.org/manual/en/latest/render/shader_nodes/color/shader_to_rgb.html
- Unreal, material instances e grupos de parâmetros [V parcial]: https://dev.epicgames.com/documentation/unreal-engine/creating-and-using-material-instances-in-unreal-engine e https://dev.epicgames.com/documentation/unreal-engine/material-parameter-expressions-in-unreal-engine
- Cel no ANIGO: `docs/native/CEL_ENGINE_FASE1.md`, `docs/native/pesquisa/cel_shading_estado_da_arte.md`.
