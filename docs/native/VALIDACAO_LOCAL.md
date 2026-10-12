# Validação local (o que o chat local precisa conferir)

Esta lista é o contrato entre a sessão de nuvem e a validação no seu PC. A nuvem **não** tem GPU real, monitor, mouse, caneta,
Windows nem seu gosto — mas tem (descoberta desta sessão) **Vulkan por software (lavapipe) + Xvfb**, então o app inteiro foi
executado e fotografado lá. Tudo marcado ✅-nuvem foi visto em screenshot; o que está abaixo é o que ainda precisa de você.

## 0. Preparar
```powershell
# Windows (MSVC). Pré-requisitos: Rust stable (>=1.88), VS Build Tools C++ + Windows SDK. Não precisa de cmake/LLVM.
cargo run -p anigo-studio                       # app nativo (janela real, GPU real via DX12/Vulkan)
cargo run -p anigo-studio -- --workspace modelagem
cargo test -p anigo-modeling -p anigo-native -p anigo-session -p anigo-studio -p anigo-renderer
```
Flags úteis: `--workspace <chave>`, `--screenshot <png> --frames 14`, `--scenario <nome>` (ver `app/automation.rs`),
`--session-file <json>`, `--no-session`, `--lang pt|en`. Variável `ANIGO_SHADER_DIR` aponta o hot reload de shader para outro diretório.
Em Linux sem GPU: `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json xvfb-run -a cargo run -p anigo-studio -- --no-session --screenshot out.png`.

## 1. Entrada nativa (mouse/teclado/caneta) — só você consegue
| # | Verificar | Esperado |
|---|---|---|
| 1.1 | MMB arrasta orbita; Shift+MMB pan; Ctrl+MMB zoom; roda faz zoom no cursor; Alt+LMB orbita | sensação próxima do Blender; sem "pulo" |
| 1.2 | Clique seleciona vértice/aresta/face; Shift estende; arrastar faz caixa; Alt+A limpa | oclusão correta (não seleciona o que está atrás); raio-X (Alt+Z) |
| 1.3 | G/R/S + X/Y/Z + digitar número + Backspace + Enter/clique/Esc/RMB; Ctrl = snap | idêntico em espírito ao Blender; status bar mostra Δ |
| 1.4 | Gizmo (Mover/Rotacionar/Escalar no shelf): arrastar alça | restringe ao eixo; solta = confirma |
| 1.5 | E / I / Ctrl+B / Ctrl+R / K / X / M / F / H … | operador aplica; "Ajustar" refaz ao mudar parâmetro; Ctrl+Z desfaz |
| 1.6 | Edição proporcional (O): raio, falloff, "somente conectados" | monte suave, sem vazar entre ilhas |
| 1.7 | **Escultura**: traço com mouse e **caneta** (pressão), simetria, Ctrl inverte, Shift suaviza, F raio | traço contínuo sem falhas; undo por traço |
| 1.8 | IME/acentos PT-BR nos campos de texto (nome do objeto, busca da biblioteca) | acentos e cedilha ok; sem fonte faltando |
| 1.9 | DPI alto (125–200 %) e janela redimensionada/maximizada | UI nítida, viewport re-aloca alvo sem piscar |
| 1.10 | Multi-monitor / mover janela entre monitores de DPI diferente | sem crash, texto escala |

## 2. GPU real e desempenho
| # | Verificar | Esperado |
|---|---|---|
| 2.1 | Abrir o app: status bar mostra o adaptador (não "llvmpipe") e backend DX12/Vulkan | GPU dedicada |
| 2.2 | Malha densa (quad_sphere subdiv 64 ≈ 24 k faces; Subsurf nível 3) orbitando | ≥ 60 FPS; sem engasgo ao mover/selecionar |
| 2.3 | Escultura em 24 k–100 k faces | traço fluido; se travar, anotar a contagem de faces |
| 2.4 | **Reconstrução por frame em G/R/S de malha grande** | hoje o sólido é reconstruído a cada frame de arrasto (O(n)); anotar a partir de quantas faces incomoda (tarefa: atualizar só posições) |
| 2.5 | Cel Shading no viewport (combo de sombreamento) | igual ao frame de referência dos PNGs em `docs/native/img/` (diferenças de driver aceitáveis) |
| 2.6 | Golden frames: `cargo test -p anigo-renderer` | **pulam** fora de lavapipe (hashes são de software); em GPU real, apenas confira visualmente os PNGs |

## 3. Gosto visual (humano)
* Cel: espessura/afinamento da linha, cor da linha, paleta de sombra (hue-shift), ponto do ramp, rosto SDF (luz 0°/45°/90°…).
* Workbench: cores de seleção, tamanho dos pontos/linhas, contraste do grid, luz de estúdio.
* Modificadores (Subsurf/Mirror/Solidify) em geometria orgânica real (cabeça, mão, roupa): vincos (`crease_scale`), UV, normais.
* Bevel: perfil e patches de canto com segmentos ≥ 3; Bridge/loop cut em topologia real de personagem.
* Normais: transferência de normais de esfera (rosto/cabelo) → o terminador de sombra fica "limpo"? Ajustar mistura/peso.
* Hair cards: curl, flare, viés de normal (módulo `curve.rs`; sem UI ainda).
* Escultura: força/falloff/espaçamento dos pincéis, Snake Hook, Elastic, estabilização de traço.

## 4. Windows / MSVC e C/C++ (`anigo-native`)
```powershell
cargo test -p anigo-native          # compila meshoptimizer, ufbx e xatlas (C/C++ vendorizados) com cl.exe
cargo check -p anigo-native --no-default-features   # sem C++
```
Conferir: build limpo com MSVC (flags `-std` do xatlas via `flag_if_supported`), `std::thread` do xatlas com o CRT, `+crt-static` se usar.
Importar um **FBX real** (Blender/Maya/Mixamo): escala em metros, Y-up, normais por corner, skin, blend shapes. Rodar `auto_unwrap` em um personagem.
Detalhes e a lista "C++ depois" (OpenSubdiv GPU, Manifold, QuadriFlow, OpenUSD): `docs/native/CPP_FFI.md`.

## 5. Hot reload / fluxo de desenvolvimento (precisa do seu PC)
1. **Shader**: abra o app e edite `crates/anigo-studio/src/workbench/workbench.wgsl` → muda em < 1 s; WGSL inválido não derruba (mensagem na barra de status, pipeline antigo permanece). *(testado em nuvem com arquivo; confira no Windows — `notify` usa ReadDirectoryChangesW.)*
2. **Subsecond** (`dx serve --hotpatch`): spike de 1 dia no `anigo-studio` (T-4.3). Esperado: edição de UI sem reiniciar. Se instável → `bacon run` + restauração de sessão (já implementada: `--session-file`/storage do eframe).
3. Medir tempos: build incremental ao editar `crates/anigo-studio/src/app/*.rs` (meta < 10 s) e testar `-Clinker=rust-lld` no MSVC.
4. Perfil dev: considerar `[profile.dev.package."*"] opt-level = 2` só para wgpu/naga/image (T-4.2) se o debug ficar lento.

## 6. Integração com o resto do ANIGO
* `cargo check -p anigo-app` (Tauri legado) continua compilando; `npm test` (340), `npm run check:wgsl`, `npm run fixtures:check`, `node scripts/check_rust_syntax.mjs` verdes.
* O app Tauri ainda é o único com os workspaces Personagem/Posing/Shading/Iluminação/Cenário/Animação/Render completos — no nativo eles são placeholders (T-2.10/2.11).
* MCP (`anigo-mcp`): as ferramentas antigas seguem falando com o Tauri (`127.0.0.1:39090`); as novas `anigo_native_*` falam com o app nativo iniciado com `--bridge` (T-2.13, `MCP_BRIDGE_NATIVA.md`).

## 7. Itens que a regra "teste visual real" (ANIGO_INVIOLABLE_RULES §1) exige de você
Os testes via MCP/mouse do sistema operacional **não** foram executados (§1.3 — nunca age de surpresa). O que foi feito aqui é equivalente
a §1.2 (render offscreen determinístico) + screenshots do app sob Xvfb. Antes de dar qualquer sprint por concluída, rode o roteiro das seções 1–3 no app real.

## 8. Riscos conhecidos (para você ficar de olho)
1. **egui quebra API 2–4×/ano** — versão fixada em 0.36; atualizar em janela planejada.
2. **xatlas vendorizado** (~10 k linhas de C++ MIT em `crates/anigo-native/vendor/`): o crate `xatlas-rs` não linka fora do MSVC; decisão e hashes em `CPP_FFI.md`. Revisar se aceita C++ vendorizado no repositório.
3. **Subsecond/hotpatch é experimental**; não é requisito.
4. **Hashes golden só valem em lavapipe**; GPUs reais divergem.
5. **Reconstrução do sólido por frame** durante G/R/S/escultura em malhas grandes (item 2.4).

## 9. Pendências por funcionalidade entregue (consolidado dos agentes)

Marque cada item no chat local. Tudo abaixo passou nos testes automatizados e em capturas no lavapipe/Xvfb; nada foi sentido com mouse/caneta/GPU reais.

**Escultura (`CEL`/Workbench)** — sensação do traço em build release, força/espaçamento por pincel, Ctrl invertendo e Shift suavizando durante o traço, pressão de caneta (só lê `Event::Touch.force`, nunca testado), Dyntopo em malha real (refaz topologia a cada quadro com mudança: pesado acima de poucos milhares de faces), gosto do anel do pincel e da cor da máscara. Wireframe em escultura cai no caminho lento.

**Normais anime** — pincel de atributo NPR (espaçamento 25% do raio, sem pressão), cores do overlay/mapa de calor, fluxo de transferência de normais num rosto real (testado só em esferas), `bake_outline_normals` ainda sem destino na malha, raycast da pintura é O(faces) (falta ligar o BVH), com a pintura ativa o clique não seleciona.

**UV** — arrasto/G-R-S de UVs com mouse real, cores de distorção/densidade em malha real, LSCM/ARAP em modelos grandes.

**Personagem** — o manequim canônico é cru (peças segmentadas, sem feições): sliders de olhos/boca quase não têm efeito visível; arrasto ao vivo no hardware real (mudança de proporção reconstrói a geometria base a cada passo); layout do painel (350 px) e do pad de somatótipo; Salvar/Abrir é por caminho digitado (sem diálogo nativo); abrir projeto zera o undo; hair/cloth/accessories/paint ainda "Em breve".

**Motor cel** — todos os recursos novos nascem DESLIGADOS (para manter os goldens): ligar suavização de normais do contorno, falloff por distância, taper e tinta por material e afinar com olho de artista (largura padrão ~0,9 px a 512 px é quase invisível); tinta de pele clara sai oliva; Face SDF só aparece de verdade acima de ~60° (mapeamento theta→limiar congelado pelos goldens) e não há textura de rosto real, só o mapa procedural de demonstração; DoF com 16 amostras mostra anéis em bokeh grande; viewport do app legado (TS) não tem os novos parâmetros. Goldens de frame (FNV-1a) são válidos só para lavapipe: em GPU real use a métrica perceptual, não o hash.

**Biblioteca (backend)** — nenhum arquivo real foi importado (todos os fixtures são sintéticos): trazer VRoid/UniVRM, Blender (GLB/FBX), Sketchfab, Quixel/Fab e o corpus Khronos; nenhum FBX válido foi importado (ufbx só exercitado com lixo/truncado); mapeamento MToon e PBR→toon só conferido contra a spec (o sinal do `shift` precisa de comparação visual); Windows/macOS (nomes reservados, caminhos longos, rename de diretório, antivírus, notify fora do inotify); `index.db` em rede (SMB/NFS) pode corromper, use disco local; desempenho a frio/HDD/10^6 ativos; números do benchmark (50 k: rebuild 5,65 s, busca <16 ms) foram medidos num Xeon 2,1 GHz de nuvem.

**Cabelo (Modo Cabelo da Modelagem, `FERRAMENTA_CABELO.md`)** — sensação do desenho com mouse e caneta (espaçamento, suavização, RDP, elevação), arrasto de pontos e pente em pixels, gosto do cabelo gerado (largura/afinamento, normais viesadas, AO de raiz) e do preview Cel (sem tangente de fio/angel ring), comparação com o VRoid real, cabeças reais (orelhas, escala ≠ 1, modificadores), desempenho com cabeça densa e centenas de mechas; GLB não leva material nem pesos; cadeias de física só estrutura (conversão para `anigo-physics`/`VRMC_springBone` pendente).
**Iluminação (ver `ILUMINACAO_UI.md` §8)** — sensação do gizmo de direção (disco do sol, esfera de 100 px, Shift = 15°, Ctrl+arrastar) com mouse real; gosto dos 7 rigs, das sombras azul-marinho do material padrão e do degrau/bias/dureza da sombra projetada; **desempenho do passe de sombra** (2048²/4096², PCF de 25 amostras) em GPU real; o **viewport Svelte/Tauri legado** com o shader compartilhado (bindings 18–20 ancorados no TS, nunca executado aqui); backend GL (passe desligado); cabelo/roupa **reais** projetando no rosto e a estabilidade da sombra com a câmera perto; persistência da câmera/gizmo entre execuções; luzes extras (ponto/spot/retangular) e tonemap/HDR (T-3.7) **não existem**.

**Cenário (editor de níveis estilo UE5, ver `CENARIO_UI.md`)** — sensação do fly (RMB+WASD/QE, roda = velocidade), do gizmo W/E/R (tamanho constante em pixels, limiar de arrasto, Alt+arrastar duplica) e do snap (grade 0,5 m, rotação 15°, Ctrl inverte, End/Z) com mouse e teclado reais; drag-and-drop real de cards do Content Drawer (validado só com eventos sintéticos do xdotool no Xvfb); desempenho com GPU real e centenas de instâncias (o renderer reenvia os vértices de cada nó a cada quadro, sem instancing); gosto do cel/gizmo/guias; luzes extras não existem (render_contract congelado) e snap de vértice (V) é Fase 2; Ctrl vs Cmd em macOS; painel direito: largura persistida pelo egui (apague `~/.local/share/anigo-studio/app.ron` se ficar larga demais).

**Base de personagem v2 (`anigo-charbase`, ver `PERSONAGEM_BASE.md`)** — a malha é um manequim anatômico: só você valida/esculpe o rosto anime e as proporções (comparar com `C:\ANIGO\IMAGENS`), mãos e pés, e a silhueta do corpo; render com GPU real (contorno nos loops de pálpebra e lábio, sombra do rosto); deformação dos pesos em poses de ombro, quadril, virilha, axila e dedos (a nuvem só testou o repouso); qualidade das 15 expressões e do `blink` (pontos de íris no topo); importar `assets/models_v2/*.glb` no UniVRM, three-vrm, VRoid e VSeeFace (humanoid, expressões, MToon) e no Blender (`extras.targetNames`, accessors esparsos, 65 ossos); miniaturas na Biblioteca ("Bases v2 (Gen-0)") e importação na Modelagem; esculpir só posições e rodar `cargo run -p anigo-charbase --example check_sculpt -- meu.glb` (Exact = contrato vale); decidir o contrato do renderer a N ossos (hoje só o preview colapsado em 24).

**Render (saída de render, ver `RENDER_UI.md` e `pesquisa/render_saida.md`)** — tempo e memória de 4K×SS2 e A4 300 dpi×SS2 em GPU real (o orçamento de 48 MP e o limite de 8192 px foram pensados, não medidos fora do lavapipe); gosto do antialiasing por média em espaço codificado e do contorno isolado/normais/profundidade como camadas de composição (Photoshop, Clip Studio, After Effects); leitura dos PNGs com `cICP` (Display P3) e dos PNG 16 bits nesses programas e nos navegadores (JPEG sai convertido para P3 sem marcação); sensação de órbita/pan/dolly do preview com mouse/trackpad reais e DPI diferente de 1×; DoF do preview usa o raio em pixels do preview (aparência aproximada do render final); `ffmpeg` real (só o texto da linha de comando foi testado, o ANIGO não executa encode); Windows/macOS (nomes reservados tratados, só Linux exercitado; pastas de rede); EXR/HDR/tonemap/bloom/grain não existem (renderer 8 bits, contrato congelado; T-3.7/T-3.10); fila em memória (fechar o app perde jobs em andamento); a integração com a timeline da Animação (gancho `FrameSource`) só foi testada com uma fonte de teste.

**Shading (editor de materiais toon/anime, ver `SHADING_UI.md`)** — gosto dos defaults, dos 7 presets embutidos, do botão "Ligar recursos anime" (contorno suave/falloff/taper/tinta) e da receita HSV da sombra (a cor de sombra é multiplicada pela luz ambiente no shader: sombras escuras viram quase preto); sensação do arrasto dos sliders, do editor de gradiente da rampa (alças, duplo clique, 6 paradas) e dos seletores de cor (cada arrasto no popup vira 1 passo de undo) com mouse real; Ctrl+C/Ctrl+V de material e Ctrl+Z/Ctrl+Y com teclado real em Windows/macOS; desempenho com GPU real (bola + objeto + A/B em uma cena por quadro, rampa personalizada = 1 textura 256×4 por gradiente); mapas SDF facial reais de artista pela Biblioteca (só o mapa procedural foi exercitado) e o espelhamento nelas; materiais VRoid/MToon com texturas (o preset guarda só os parâmetros do `StylizedMaterial` + extras do cel; texturas por material, alpha/recorte e 2ª sombra por textura não existem no renderer); cores/leitura dos swatches (sRGB gravado × o look claro do motor); paleta do projeto é um arquivo (`projeto.anigopalette.json`), não viaja dentro do `.anigo`; o mapa SDF é global ao renderer do preview.

**Posing (`POSING_UI.md`)** — sensação do gizmo FK (tamanho dos anéis, limiar de arrasto, snap Ctrl), do IK ao vivo (quadrado/pole, travas X/Y/Z) e do foot lock com mouse real; gosto visual do esqueleto/cones de limite; qualidade da deformação em poses fortes (axila/candy-wrapper, costuras da malha canônica), dedos curvados (deformação CPU), valores de limites por osso e frações de massa (`[C]/[S]`, revisão por artista/biomecânico), desempenho do arrasto com GPU real, Windows/macOS (Ctrl vs Cmd), miniaturas 3D de pose na Biblioteca com GPU real; não implementados: aplicar card pelo workspace Biblioteca, GLB posado, pose no snapshot TS.

**Bridge MCP nativa (T-2.13, `MCP_BRIDGE_NATIVA.md` §6)** — tudo foi validado só em Linux (Xvfb + lavapipe + CLI `bridge-call`): **Windows** (`%LOCALAPPDATA%\ANIGO\bridge\bridge.json` criado e legível só por você: ACL herdada, sem `mode`; antivírus/firewall com loopback; `rename` atômico sobre arquivo aberto), **um cliente MCP real** (Claude Desktop/Code com `"command": "anigo-mcp"`; rodar `anigo_native_get_status`, `apply_command` com `{"kind":"set_proportions","head_scale":1.4}`, `undo`, `screenshot`, `render_frame`), janela **minimizada/oculta** (esperado: `-32002` sem aplicar nada; confirmar que o app acorda ao restaurar), latência e fluidez com GPU real e com o usuário arrastando um slider enquanto o MCP aplica comandos (o gesto em curso é cancelado e recomeça; conferir se a sensação é aceitável), o `PLACE_ASSET` com ativos grandes (a UI espera até 6 s por ativo), e se o fluxo "pedir ao agente -> Ctrl+Z na janela" desfaz o que se espera nos workspaces Personagem/Cenário/Iluminação/Render/Posing (Modelagem e Shading têm histórico próprio fora da bridge).

**Animação (`ANIMACAO_UI.md`, pesquisa em `pesquisa/animacao.md`)** — sensação da timeline com mouse/trackpad reais (arrasto de chaves, zoom/pan, caixa de seleção, alças Bézier do Graph, scrub) e da reprodução em tempo real em GPU real (o sandbox só roda lavapipe e xdotool com passos discretos); gosto visual do onion skin (é do esqueleto, não da malha), das cores do dope sheet e da física; **arquivos reais de artista**: `.vrma` de bibliotecas, FBX→glTF do Mixamo, BVH de captura (CMU, Rokoko) e a qualidade do retarget por direção de osso em animações longas e rigs fora do padrão (os testes usam esqueletos sintéticos com repouso diferente); abrir os `.glb`/`.vrma`/`.bvh` exportados no Blender, three-vrm, UniVRM e VSeeFace; física: os parâmetros do cabelo/saia de teste e o desempenho (ela não entra no PNG: o renderer não a consome, contrato congelado); `ffmpeg` real a partir da linha sugerida da sequência (o app nunca executa encode); render de sequência longa (centenas de quadros, 4K) em GPU real — só 3–4 quadros pequenos foram renderizados no sandbox; não há diálogo de arquivo nativo (importar/exportar usa caminho digitado); `scripts/check_rust_syntax.mjs` precisa de `npm ci` com as dev dependencies (`tree-sitter-wasms`); Windows/macOS (Ctrl vs Cmd); não implementados: onion da malha, lip-sync, curvas de Euler editáveis diretamente (as chaves guardam quaternions), preferências do workspace persistidas.
