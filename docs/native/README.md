# ANIGO nativo — índice

| Documento | Conteúdo |
|---|---|
| [ARQUITETURA_NATIVA.md](ARQUITETURA_NATIVA.md) | ADR: UI/viewport 100 % Rust (egui + wgpu 30), por que não Tauri/Slint/iced/Bevy, C/C++ só onde vale, hot reload, plano de migração |
| [RELATORIO_FINAL.md](RELATORIO_FINAL.md) | **Relatório final**: o que foi entregue, o que não foi feito e o que falta para o nível comercial (P0/P1/P2) |
| [ROADMAP_TAREFAS.md](ROADMAP_TAREFAS.md) | Todas as tarefas por trilha, com estado e tipo de validação |
| [MODELAGEM_FERRAMENTAS.md](MODELAGEM_FERRAMENTAS.md) | Mapa de ferramentas do workspace de Modelagem (Blender como referência) |
| [MODELAGEM_AUDITORIA.md](MODELAGEM_AUDITORIA.md) | Auditoria de completude da Modelagem: o que existe, o que está só no motor e o que falta, em ordem de impacto (imagens de referência é a nº 1) |
| [SHADING_ANIME_PLANO.md](SHADING_ANIME_PLANO.md) | Plano do sistema de shading anime "acima do mercado" (calibração por referência, sombra desenhada, on twos, camadas de cel, pilha de look com grafo) + ferramentas de calibração prontas (`scripts/look_calibration`) |
| [FERRAMENTA_CABELO.md](FERRAMENTA_CABELO.md) | Modo Cabelo da Modelagem (estilo VRoid): uso, atalhos, modelo de dados, cadeias de física neutras, limites e o que não foi validado |
| [VALIDACAO_LOCAL.md](VALIDACAO_LOCAL.md) | **O que o chat local precisa validar** (entrada nativa, GPU real, gosto, Windows/MSVC, hot reload) |
| [CPP_FFI.md](CPP_FFI.md) | Política C/C++ (`anigo-native`: meshopt, ufbx, xatlas) e lista "C++ depois" |
| [CEL_ENGINE_FASE1.md](CEL_ENGINE_FASE1.md) | Melhorias do motor de cel shading (fase 1: DoF, rampa 2D, contorno, face SDF, paridade WGSL↔Rust) |
| [BIBLIOTECA_BACKEND.md](BIBLIOTECA_BACKEND.md) / [BIBLIOTECA_BENCH.md](BIBLIOTECA_BENCH.md) | `anigo-assets`: SQLite/FTS5, locais, import GLB/glTF/OBJ/FBX/VRM, material toon canônico, miniaturas, benchmark de 50 mil ativos |
| [BIBLIOTECA_UI.md](BIBLIOTECA_UI.md) | Workspace Biblioteca: grade virtualizada, miniaturas 3D cel, gestão, import, cenários de captura |
| [FISICA.md](FISICA.md) | `anigo-physics`: spring bone VRM-exato, XPBD (cabelo, roupa por proxy, acessórios), `PhysicsWorld`, presets, números medidos, o que não foi validado visualmente e o que o renderer precisaria (storage buffer de N ossos) |
| [PERSONAGEM_BASE.md](PERSONAGEM_BASE.md) | `anigo-charbase`: base anime Gen-0 masculina/feminina (10,2 k vértices, 100 % quad, 65 ossos VRM + twist/busto, pesos, 64 morphs, contrato de topologia, GLB/VRM em `assets/models_v2`), manequim x escultura, ações locais e plano de migração |
| [CENARIO_UI.md](CENARIO_UI.md) | Workspace Cenário (editor de níveis estilo UE5): viewport/fly, Content Drawer da Biblioteca, seleção BVH, gizmo e snap, Outliner/Details/Histórico, smoke xdotool |
| [ILUMINACAO_UI.md](ILUMINACAO_UI.md) | Workspace Iluminação: gizmo de direção da luz, sol/Kelvin/ambiente/sombra/rim, 7 rigs, luz presa à câmera, shadow map de 1 luz (aditivo, desligado por padrão), provas por pixels e smoke xdotool |
| [RENDER_UI.md](RENDER_UI.md) | Workspace Render: câmera de render, saída (presets, supersampling, alfa, P3, PNG 8/16/JPEG), passes e AOVs, preview com máscara/guias, fila de render real (thread de GPU, cancelar/retomar), histórico com miniaturas, gancho de sequência, smoke xdotool |
| [SHADING_UI.md](SHADING_UI.md) | Workspace Shading (editor de materiais toon/anime): alvos de Personagem/Cenário/Modelagem, parâmetros agrupados, editor de gradiente da rampa 2D, contorno por material, face SDF, paleta, presets `.anigomat.json` com miniatura em shader ball, A/B, undo, smoke xdotool |
| [POSING_UI.md](POSING_UI.md) | Workspace Posing: modelo de pose do núcleo (`SetPose`, `PoseRig`, `.anigopose.json`), FK/IK/limites/presets/mãos/olhar, API para a Animação, smoke xdotool |
| [MCP_BRIDGE_NATIVA.md](MCP_BRIDGE_NATIVA.md) | Bridge local do `anigo-mcp` com o app nativo (`--bridge`): protocolo NDJSON/HTTP, ações (projeto, `Command`, undo/redo, log, Biblioteca, render, screenshot, workspace), segurança (loopback, token, Host/Origin, limites), ferramentas `anigo_native_*`, testes, smoke e o que não foi validado |
| [ANIMACAO_UI.md](ANIMACAO_UI.md) | Workspace Animação: modelo de animação do núcleo (`EditAnimation`, canais/chaves/interpolação), timeline/dope sheet/Graph, auto-key, onion, clips na Biblioteca, glTF/VRMA/BVH, física no playback, render de sequência, smoke xdotool |
| [LAYOUT_ATALHOS.md](LAYOUT_ATALHOS.md) | Layout dockável persistido (`egui_tiles`, painéis registráveis, presets, redefinir), atalhos configuráveis (registro de ações, conflitos por contexto, import/export JSON) e barra de menus; smoke xdotool |
| [PROPRIEDADES_ABAS.md](PROPRIEDADES_ABAS.md) | Painel Propriedades com faixa vertical de abas (Modelagem por modo, Personagem, Posing, Iluminação, Animação, Shading): categorias, foco de seção, código e o que não foi validado |
| [MULTIVISTA.md](MULTIVISTA.md) | Multi-vista do viewport (1 / 2 lado a lado / 2 empilhadas / 4): modelo (slots, câmera ativa, vista sob o ponteiro), tipos de vista (perspectiva, ortogonal, frente…base), divisórias arrastáveis, preview Cel por vista, persistência na sessão, atalhos, como foi testado e limites |
| [pesquisa/](pesquisa/) | Relatórios de pesquisa (UI Rust + hot reload, C++ vs Rust p/ modelagem, estado da arte de cel shading, bibliotecas de assets) |
| img/ | Screenshots de referência gerados sob Xvfb + lavapipe |

Crates novas: `anigo-bridge` (servidor local do MCP), `anigo-modeling` (motor), `anigo-assets` (biblioteca), `anigo-native` (C/C++), `anigo-session` (CoreSession compartilhada), `anigo-studio` (app).
Comandos úteis: `cargo run -p anigo-studio`, `scripts/dev_native.sh`, `scripts/native_input_smoke.sh`, `bacon.toml`.
