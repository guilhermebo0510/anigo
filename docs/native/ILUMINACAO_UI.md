# Workspace Iluminação — UI nativa (`anigo-studio`)

**Estado:** Fase 1 (MVP) e a maior parte da Fase 2 de [`pesquisa/iluminacao_anime.md`](pesquisa/iluminacao_anime.md) implementadas e validadas no sandbox
(Xvfb + Vulkan por software/lavapipe, sem GPU real, sem mouse/teclado reais de um artista). Cobre a parte de Iluminação de T-2.11 e fecha T-3.8
(shadow map de 1 luz + sombra do cabelo/cabeça no rosto) no renderer, **desligada por padrão**.

Legenda: **[T]** coberto por teste automático que rodou; **[S]** visto em screenshot ou em entrada real (xdotool) no Xvfb; **[N]** não validado.

## 1. O que existe

Tudo ligado a dados reais (sem mockups): a luz é a `StylizedLight` canônica do `anigo-core` (a primeira luz do projeto), editada por `Command` através do
`CoreSession` — a MESMA sessão do Personagem e do Cenário (Ctrl+Z/Ctrl+Y do app inteiro desfazem a luz). O preview usa o cel canônico do
`anigo-renderer` sobre o personagem **e** o cenário do projeto (nós do Cenário), então o que se vê é o que o render final produz.

| Área | O que faz | Validação |
|---|---|---|
| **Gizmo de direção** (viewport) | Esfera de direção (paralelos −60…60°, 6 meridianos, horizonte destacado) centrada no personagem, tamanho constante em pixels, **disco do sol** arrastável, raio de luz com seta, **arcos de azimute e elevação** (protractor) e a marca "frente" (az 0°). LMB no disco escolhe a direção sobre a esfera (hemisfério da câmera; sair da esfera e voltar inverte o hemisfério, o que leva a luz para trás do personagem). **Shift** = passos de 15°. **Ctrl+arrastar** em qualquer ponto gira a luz pelo deslocamento. Orbit MMB/Alt+LMB, pan Shift+MMB, zoom na roda, **F** enquadra o personagem. | [T] matemática (az/el, escolha na esfera, hemisfério, geometria); [S] arrasto real com xdotool |
| **Preview ao vivo + um comando** | O arrasto (disco ou slider) aplica `CoreSession::preview` a cada quadro (fora do histórico) e vira **UM** `SetLight` ao soltar; voltar ao valor inicial não suja o histórico. | [T] 40 previews = 1 passo; undo exato; no-op; [S] smoke: 1 passo, Ctrl+Z restaura a viewport (0 px) |
| **Sol** | Azimute, elevação, **cor** (picker), **temperatura em Kelvin** (1500–12000 K, régua de corpo negro; escreve a COR da luz), intensidade 0–3, "Restaurar sol". | [T] Kelvin→RGB (Tanner Helland); [S] |
| **Ambiente / hemisfério** | Intensidade, cor do **céu** e do **chão** (`ambient_sky/ground`), régua do hemisfério. No shader o ambiente multiplica o tom da sombra (é o "fill" do rig). | [S] `iluminacao_ambiente`; [T] pixels |
| **Cor e dureza da sombra** | Tinta e saturação da sombra (`shadow_color`/`shadow_saturation`, multiplicam a cor de sombra do material); **limiar** e **penumbra** do terminador (`shadow_threshold`/`shadow_smoothness` do **material do personagem**). | [T] tinta avermelha a sombra; [S] |
| **Rim** | Intensidade, abertura e cor do rim do **material do personagem** (rim Fresnel + contraluz do shader). | [T] rim acende a silhueta só com contraluz e nunca escurece |
| **Rigs de luz** | 7 presets — três pontos, contraluz/rim, dia, golden hour, noite, interior, estúdio — cada um um `Command::Batch` (`SetLight` + `SetMaterialParams` do rim) = **um** passo de desfazer; não tocam nas configurações de sombra projetada. | [T] catálogo, contraste 3:1 do três-pontos, undo exato de cada rig; [S] 4 rigs fotografados; [S] smoke clica o botão |
| **Luz presa à câmera** | Modo de **preview** (studio light): a luz acompanha a câmera com ângulos RELATIVOS a ela (padrão −35°/37°); o projeto não muda. "Aplicar ao projeto" grava a direção resultante (um `SetLight`). Ligar não faz a luz "pular" (parte da direção atual). | [T] relativo↔mundo ida e volta; [S] `iluminacao_camera` |
| **Sombra projetada** (shadow map de 1 luz) | Liga/desliga, **força**, **dureza**, **bias** (m), **bias de normal** (texels), **resolução** (512–4096), **cobertura** (m; automática = o conteúdo, no máximo 7 m em torno do alvo da câmera). A sombra herda a cor de sombra do terminador. A sombra do cabelo/cabeça no rosto e do braço no tronco vêm do mesmo mapa (auto-sombra). Persistida em `StylizedLight.shadow` (omitido do JSON no padrão). | [T] ver §3; [S] `iluminacao_sombra`, `_sombra_suave`, `_rosto` |
| **Luzes extras** | Ponto / Spot / Retangular **desabilitadas, com explicação** (o contrato de render tem uma luz direcional principal). | [S] `iluminacao_extras` |
| **Cena de prova** | "Adicionar piso de prova" (plano sob o personagem) e "Adicionar volume de cabelo (prova)" (elipsoide-franja sobre a testa) — cada um um passo de desfazer. O cabelo é um **proxy**: o cabelo do workspace Personagem ainda não é malha de cena. | [S] |
| **Undo/Redo** | Ctrl+Z / Ctrl+Y / Ctrl+Shift+Z e os botões do topo, roteados para o `CoreSession` (`route_undo_redo`/`can_undo_redo`); histórico de 400 passos como o Cenário. Descrições traduzidas ("Luz", "Terminador / rim", "Rig de luz"). | [T] roteamento; [S] Ctrl+Z/Y reais |

## 2. Atalhos

| Ação | Atalho |
|---|---|
| Girar a luz | arrastar o disco do sol (Shift = passos de 15°) |
| Girar a luz sem pegar o disco | Ctrl+arrastar na viewport |
| Órbita / pan / zoom | MMB ou Alt+LMB / Shift+MMB / roda |
| Enquadrar o personagem | F (ou o botão da barra) |
| Desfazer / refazer | Ctrl+Z / Ctrl+Y ou Ctrl+Shift+Z |

O shelf lateral tem 4 ferramentas (Sol, Sombras, Ambiente e céu, Rigs de luz): cada uma abre e rola o painel até a seção.

## 3. Sombra projetada (renderer) — o que foi feito e como se prova

Avaliação: **coube SEM quebrar o contrato** porque é aditiva e desligada por padrão. O desenho:

* **Dados** (`anigo-core::scene::LightShadow`): `enabled, strength, hardness, bias, normal_bias, resolution, distance`; vive em `StylizedLight.shadow`
  (`#[serde(default, skip_serializing_if = default)]`: JSON canônico e fixtures de projeto **não mudam**); `SetLight.shadow: Option<LightShadow>` valida faixas
  (NaN/fora de faixa/resolução inválida são recusados), tem inverso exato e só carrega o campo no inverso quando o comando o tocou (logs antigos idênticos).
  Contrato TS (`commands.v1.ts`) atualizado e `npm test` verde.
* **Matemática pura** (`anigo-core::shadow`): projeção ORTOGRÁFICA ajustada a uma esfera envolvente dos cantos de todos os nós visíveis (raio constante sob
  rotação), **snap de texel** (a borda não cintila), olho recuado até o projetor mais próximo da luz, foco opcional (distância), offset de normal e degrau estilizado.
* **Renderer** (`headless.rs`): passe só-profundidade da luz (`shadow_pipeline`, 1×, `depth24plus`, sem culling, bias de rasterização `2/2.0`) que **reusa o `vs_main` do
  cel** (mesmos buffers e skinning; a câmera do bind group é a da luz); depois o cel amostra o mapa: bindings **18** (uniform `ShadowUniform`, 96 B), **19**
  (`texture_depth_2d`) e **20** (`sampler_comparison`) do grupo `cel`. PCF 5×5 de `textureSampleCompareLevel` (raio 1,5–4,5 texels conforme a dureza) →
  `shadow_step` (smoothstep em torno de 0,5) → `toon_factor *= 1 − sombra·força`: a sombra projetada **herda a cor de sombra do terminador** (base × tinta, hue-shift).
* **Desligada por padrão = quadro idêntico**: sem `enabled`, o uniform é neutro (`params.x = 0`), o mapa é uma profundidade 1×1 e o shader não amostra nada.

| Prova | Onde |
|---|---|
| Hashes dos quadros congelados **iguais** | `tests/golden_frames.rs` (3 quadros) verde antes e depois |
| Desligada não muda um byte (mesmo mexendo nos outros parâmetros) | `tests/shadow_map.rs::shadow_off_is_the_default_and_changes_nothing` |
| Ligada escurece o chão sob o cubo e só ali; nunca clareia | `shadow_darkens_the_floor_under_the_cube_and_only_there` |
| A sombra anda com o azimute, alonga com a elevação | `shadow_follows_the_light_azimuth`, `shadow_elevation_changes_the_shadow_length` |
| Força 0 = sem sombra; força 0,5 escurece menos | `strength_scales_the_shadow_and_zero_removes_it` |
| 4 resoluções, determinismo, dureza (borda suave tem mais meio-tom), cena vazia | `every_resolution_*`, `hard_shadow_has_a_sharper_edge_than_soft`, `a_degenerate_scene_*` |
| Paridade WGSL×Rust (`shadow_project`, `shadow_receiver_pos`, `shadow_step`) numa GPU | `tests/shader_math_parity.rs::shadow_math_matches_the_rust_math` |
| Matriz da luz: todos os pontos dentro de [0,1]², profundidade em [0,1], estável a movimento sub-texel, raio independe da direção, foco, projetor fora do foco | `anigo-core::shadow::tests` |
| Contrato (uniform `shadow` 96 B, bindings 18–20, seção `shadow_map`), shaders ⇄ contrato | `node scripts/gen_contract_fixtures.mjs`, `npm run fixtures:check`, `npm run check:wgsl`, `tests/wgsl_validation.rs` (naga) |

PNGs: `docs/native/img/iluminacao_sombra.png` (sombra dura do personagem, da caixa e do orbe no piso), `iluminacao_sombra_suave.png`, `iluminacao_rosto.png` × `iluminacao_rosto_sem_sombra.png`
(o bloco de cabelo projeta na testa/olhos; sem o mapa a mesma cena está toda iluminada) e a cabeça projetando no pescoço/ombros em `iluminacao_luz_el_78.png`.

### Limites da sombra (honestos)
* **Um** mapa de **uma** luz; sem CSM, sem contact shadows, sem luzes extras (ver §5).
* Nós **fora do frustum da câmera** são culados antes do passe e **não projetam** (um pilar logo fora do quadro não faz sombra); o ajuste da matriz considera todos os nós visíveis, mas só os
  desenháveis entram no mapa.
* A **cobertura automática** é limitada a 7 m em torno do alvo da câmera: cenas grandes perdem sombra além disso (escolha "Cobertura" para mudar). Um piso de 18 m sem limite daria 1 cm/texel e uma escada visível.
* **Backend GL** (Mesa llvmpipe via GL) não cria o passe (a UI desabilita o controle com aviso); o app web **Svelte/Tauri legado** só ancora os neutros nos bindings 18–20 (o shader é compartilhado) e **não** implementa a sombra. **[N]** o viewport Svelte não foi executado aqui.
* O offset de normal usa a normal interpolada do vértice: malhas de poucos polígonos (o manequim) mostram a borda da sombra facetada. O cabelo real (cartões do `anigo-hair`) ainda precisa de validação com malha de verdade.
* Rosto: a sombra por shadow map em close pode ficar instável com a câmera muito perto (o README do Genshin shader descreve o mesmo [S]); sem controle "sem auto-sombra no rosto" ainda.

## 4. Arquitetura e arquivos

```text
anigo-core    scene.rs (LightShadow em StylizedLight) · shadow.rs (matemática pura) · command.rs (SetLight.shadow)   [T]
anigo-renderer cel_shading.wgsl (ShadowUniform + bindings 18–20 + bloco "Sombra projetada") · headless.rs (passe de sombra)
              uniforms.rs (ShadowUniform) · contrato (scripts/render_contract_fixture.mjs → contracts/fixtures)
anigo-studio  lighting/        módulos puros e testados: mod (az/el, Kelvin, câmera-relativa, escolha na esfera),
                               gizmo (geometria), rigs (catálogo + Batch)
              app/lighting_ui/ UI: mod (estado, edição ao vivo = gesture_edit/gesture_finish), viewport (entrada, gizmo,
                               overlays, HUD), panels (rigs, sol, sombras, ambiente, rim, extras, cena de prova), script (capturas)
              i18n_lighting.rs  pt/en (`light.*`, `tool.rigs`)
              celview.rs        assinatura de redesenho agora cobre TODOS os campos da luz e do material (rim/terminador)
```

Hooks em arquivos compartilhados (mínimos e aditivos): `app.rs` (módulo, campo `light_ui`, despacho de painel/viewport/atalhos, status, tecla do shelf, `switch_workspace`),
`app/automation.rs` (cenários), `app/character_ui.rs` (`route_undo_redo`/`can_undo_redo` incluem Iluminação), `app/scene_ui/{mod,viewport}.rs`
(`scene_render` virou `pub(crate)` + campo `light_dir_override` para a luz presa à câmera), `i18n.rs`, `lib.rs`, `workspace.rs` (ferramenta `rigs`), `celview.rs` (`shadow_supported` + assinatura).

## 5. Luzes extras (ponto/spot/retangular): desenho, NÃO implementado
O contrato de render é de **uma** luz direcional principal. Luzes extras exigem: `LightUniform[]` + contagem no shader, um loop de iluminação que some o terminador de cada luz (hoje há um
único `L`), um mapa por luz com sombra (ou só sombra para a principal) e mudança de contrato v2 (hashes, fixtures, TS, GLSL). Por isso a UI mostra Ponto/Spot/Retangular **desabilitados com tooltip**,
como o Cenário. O desenho está na §6 da pesquisa (Fase 3).

## 6. Capturas e smoke

Cenários (`anigo-studio --workspace iluminacao --scenario <nome> --screenshot x.png`): `iluminacao-base`, `iluminacao-luz-<az>-<el>-<intensidade×100>` (negativo com `n`),
`iluminacao-rig-<id>` (7 rigs), `iluminacao-sombra`, `iluminacao-sombra-suave`, `iluminacao-rosto`, `iluminacao-rosto-sem`, `iluminacao-camera`, `iluminacao-ambiente`, `iluminacao-extras`.
Cada um monta o conjunto (piso, parede, orbe, caixa) e aplica luz/rig/sombra pelos **mesmos comandos da UI**. Referências em `docs/native/img/iluminacao_*.png`, geradas por
`scripts/native_screenshots.sh`; a trilha `native` do CI gera as mesmas e roda `scripts/native_lighting_smoke.sh`.

`scripts/native_lighting_smoke.sh` (xdotool, **somente dentro de um Xvfb próprio** `:92`; estado do egui em `XDG_DATA_HOME` temporário; teclas com modificador em sequência explícita `keydown/key/keyup`;
`windowfocus` explícito). O app imprime `lighttrace ...` por mudança de estado (`ANIGO_TRACE_LIGHT=1`):
1. arrasta o **disco do sol** (posição medida pelo trace) → az 45° → −66°, el 35° → 42°, **exatamente +1 passo de undo**, a viewport muda (71 k px);
2. Ctrl+Z → az/el e **viewport idênticos ao antes (0 px)**; Ctrl+Y → idênticos ao arrastado (0 px);
3. clica o botão do rig **golden hour** (posição medida pelo trace) → luz quente (R−B > 0,3), rim 0,8 → 1,4, +1 passo;
4. Ctrl+Z desfaz o rig inteiro (luz **e** rim juntos), viewport idêntica (0 px).

Um defeito real que o smoke e as capturas acharam (e que os testes unitários não pegavam): o painel direito **crescia a cada quadro** até a largura máxima porque os sliders usavam toda a largura disponível;
corrigido reservando espaço para o campo de valor. Outro, achado na leitura do `celview`: a assinatura de redesenho só misturava direção e intensidade da luz, então editar **cor, ambiente, tinta ou rim não redesenhava**
o quadro parado — agora toda a luz e os parâmetros de terminador/rim do material entram na assinatura (vale também para o Cenário).

### Prova de que a luz responde (pixels, `anigo-renderer/tests/light_response.rs`, personagem canônico)
* **Azimute** −75°→75°: o centroide da área iluminada anda monotonicamente com o lado da luz (o terminador cruza o corpo).
* **Elevação** 5°→88°: a área iluminada de frente encolhe sempre e a sombra sob a cabeça/ombros cresce.
* **Intensidade** 0,25→1,6: o brilho médio cresce monotonicamente (comprimido pelo ACES do shader — achado T-3.7).
* **Ambiente** 0,15→1,4 clareia a sombra; a **tinta** vermelha avermelha a sombra.
* **Rim**: com contraluz (az 170°) e rim 2,5 centenas de pixels da silhueta acendem e nenhum escurece; com luz de frente o rim acende bem menos.

## 7. Limites conhecidos e decisões
* **Uma** luz direcional + ambiente hemisférico + rim do material: um rig não tem fill/rim como fontes reais (§ rigs). Os números dos rigs são ponto de partida artístico, não calibrados em GPU real.
* O **terminador e o rim** são editados no **material do personagem** (comandos `SetMaterialParams`); os objetos do cenário usam o material padrão (limiar 0,5, penumbra 0,02, rim 0,8) — não há ainda material por objeto para esses parâmetros.
* O ambiente **multiplica** o tom da sombra (não soma luz): ambiente ≈ 1,5 dá ~3:1 de contraste luz/sombra para o material padrão; as sombras do material padrão são escuras e frias (azul-marinho), é o look do shader atual (achado P2-04 da auditoria).
* **Persistência**: a câmera do workspace, o gizmo visível e o modo de luz presa à câmera **não** são salvos na sessão (só o projeto: luz, material, nós). A tecla F e a vista frontal reenquadram.
* **Tonemap/HDR (T-3.7) não foi feito**: o shader aplica ACES e `exp2(EV)` dentro do fragment; trocar por um alvo HDR + tonemap neutro muda o formato de alvo do contrato e exige olho humano. A intensidade 0–3 já é o teto do shader (`clamp`).
* **Temperatura** é uma aproximação artística (Tanner Helland, 1000–40000 K), escreve a cor; o K do slider é a última escolha e não é recalculado da cor.
* **Sombra do cabelo** só com malha de verdade; o botão "volume de cabelo" é um proxy.
* Os cenários de captura montam o conjunto por comandos; os screenshots só saem quando as malhas assentam.

## 8. O que NÃO foi validado (precisa de máquina local, GPU real e uma pessoa)
* **Sensação** do arrasto do disco (tamanho da esfera de 100 px, raio de captura de 15 px, sensibilidade do Ctrl+arrastar 0,35°/px, passos de 15° com Shift) — só exercitado por xdotool com passos discretos.
* **Gosto visual**: cores/intensidades dos 7 rigs, cor de sombra azul-marinho, dureza/bias padrão da sombra, aparência das bordas facetadas do manequim, cores do gizmo.
* **Desempenho** do passe de sombra (2048²/4096² + PCF 25 amostras) em GPU real e cenas grandes; lavapipe não mede isso.
* O **viewport Svelte/Tauri legado** com o shader compartilhado (bindings 18–20 ancorados no TS): o código foi editado mas não executado.
* **Backend GL**: o passe é desligado (caminho não exercitado). Windows/macOS: só Linux foi exercitado.
* Cabelo/roupa **reais** projetando no rosto; câmera muito perto do rosto (instabilidade de sombra).
* Persistência entre execuções (câmera/gizmo), e o texto en dos painéis revisado por um falante.

## 9. Como rodar
```bash
cargo test -p anigo-core -p anigo-session -p anigo-studio -p anigo-renderer
cargo run -p anigo-studio -- --workspace iluminacao                  # app
cargo run -p anigo-studio -- --workspace iluminacao --scenario iluminacao-rig-golden_hour --screenshot /tmp/i.png
bash scripts/native_lighting_smoke.sh ./target/debug/anigo-studio /tmp/lighting_smoke   # Xvfb, xdotool, ImageMagick
ANIGO_TEST_OUT=/tmp/out cargo test -p anigo-renderer --test shadow_map --test light_response   # PNGs das provas
```
