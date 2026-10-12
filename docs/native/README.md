# ANIGO nativo — índice

| Documento | Conteúdo |
|---|---|
| [ARQUITETURA_NATIVA.md](ARQUITETURA_NATIVA.md) | ADR: UI/viewport 100 % Rust (egui + wgpu 30), por que não Tauri/Slint/iced/Bevy, C/C++ só onde vale, hot reload, plano de migração |
| [ROADMAP_TAREFAS.md](ROADMAP_TAREFAS.md) | Todas as tarefas por trilha, com estado e tipo de validação |
| [MODELAGEM_FERRAMENTAS.md](MODELAGEM_FERRAMENTAS.md) | Mapa de ferramentas do workspace de Modelagem (Blender como referência) |
| [VALIDACAO_LOCAL.md](VALIDACAO_LOCAL.md) | **O que o chat local precisa validar** (entrada nativa, GPU real, gosto, Windows/MSVC, hot reload) |
| [CPP_FFI.md](CPP_FFI.md) | Política C/C++ (`anigo-native`: meshopt, ufbx, xatlas) e lista "C++ depois" |
| [CEL_ENGINE_FASE1.md](CEL_ENGINE_FASE1.md) | Melhorias do motor de cel shading (fase 1: DoF, rampa 2D, contorno, face SDF, paridade WGSL↔Rust) |
| [BIBLIOTECA_BACKEND.md](BIBLIOTECA_BACKEND.md) / [BIBLIOTECA_BENCH.md](BIBLIOTECA_BENCH.md) | `anigo-assets`: SQLite/FTS5, locais, import GLB/glTF/OBJ/FBX/VRM, material toon canônico, miniaturas, benchmark de 50 mil ativos |
| [BIBLIOTECA_UI.md](BIBLIOTECA_UI.md) | Workspace Biblioteca: grade virtualizada, miniaturas 3D cel, gestão, import, cenários de captura |
| [FISICA.md](FISICA.md) | `anigo-physics`: spring bone VRM-exato, XPBD (cabelo, roupa por proxy, acessórios), `PhysicsWorld`, presets, números medidos, o que não foi validado visualmente e o que o renderer precisaria (storage buffer de N ossos) |
| [pesquisa/](pesquisa/) | Relatórios de pesquisa (UI Rust + hot reload, C++ vs Rust p/ modelagem, estado da arte de cel shading, bibliotecas de assets) |
| img/ | Screenshots de referência gerados sob Xvfb + lavapipe |

Crates novas: `anigo-modeling` (motor), `anigo-assets` (biblioteca), `anigo-native` (C/C++), `anigo-session` (CoreSession compartilhada), `anigo-studio` (app).
Comandos úteis: `cargo run -p anigo-studio`, `scripts/dev_native.sh`, `scripts/native_input_smoke.sh`, `bacon.toml`.
