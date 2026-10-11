# ANIGO nativo — índice

| Documento | Conteúdo |
|---|---|
| [ARQUITETURA_NATIVA.md](ARQUITETURA_NATIVA.md) | ADR: UI/viewport 100 % Rust (egui + wgpu 30), por que não Tauri/Slint/iced/Bevy, C/C++ só onde vale, hot reload, plano de migração |
| [ROADMAP_TAREFAS.md](ROADMAP_TAREFAS.md) | Todas as tarefas por trilha, com estado e tipo de validação |
| [MODELAGEM_FERRAMENTAS.md](MODELAGEM_FERRAMENTAS.md) | Mapa de ferramentas do workspace de Modelagem (Blender como referência) |
| [VALIDACAO_LOCAL.md](VALIDACAO_LOCAL.md) | **O que o chat local precisa validar** (entrada nativa, GPU real, gosto, Windows/MSVC, hot reload) |
| [CPP_FFI.md](CPP_FFI.md) | Política C/C++ (`anigo-native`: meshopt, ufbx, xatlas) e lista "C++ depois" |
| [CEL_ENGINE_FASE1.md](CEL_ENGINE_FASE1.md) | Melhorias do motor de cel shading (quando concluído) |
| [pesquisa/](pesquisa/) | Relatórios de pesquisa (UI Rust + hot reload, C++ vs Rust p/ modelagem, estado da arte de cel shading, bibliotecas de assets) |
| img/ | Screenshots de referência gerados sob Xvfb + lavapipe |

Crates novas: `anigo-modeling` (motor), `anigo-native` (C/C++), `anigo-session` (CoreSession compartilhada), `anigo-studio` (app).
Comandos úteis: `cargo run -p anigo-studio`, `scripts/dev_native.sh`, `scripts/native_input_smoke.sh`, `bacon.toml`.
