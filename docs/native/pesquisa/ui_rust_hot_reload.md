# ANIGO: UI + viewport 100% Rust, hot reload (pesquisa em 11/out/2026)

Fontes primarias: API do crates.io (versoes, datas e dependencias de wgpu lidas direto dos manifests), codigo-fonte no GitHub, docs oficiais. O relatorio comparativo de terceiros github.com/GoldStrikeArch/rust-gui-desktop-ecosystem-state (jul-ago/2026) foi usado como apoio e e dado, nao verdade absoluta. Itens marcados [NAO VERIFICADO] vem de memoria.

## 0. Fato central: o wgpu 24 do workspace esta 6 majors atras
- wgpu atual: 30.0.1 (22/ago/2026); 30.0.0 em 01/jul/2026. https://crates.io/crates/wgpu
- Compartilhar o mesmo `wgpu::Device` exige a MESMA versao de wgpu no app e no framework (duas versoes coexistem no Cargo, mas os tipos sao incompativeis).
- Opcoes: (a) subir o workspace para wgpu 30 (recomendado, ver abaixo); (b) congelar em egui 0.31.1 / Slint 1.12, os ultimos a usar wgpu 24, ja velhos (fev e jun/2025).

## 1. Frameworks (versao, data, wgpu exigido)
| Framework | Ultima versao (crates.io) | wgpu exigido | Observacao |
|---|---|---|---|
| egui/eframe/egui-wgpu | 0.36.2 (08/set/2026) | ^30 | 0.31.x -> wgpu 24; 0.32 -> 25; 0.33 -> 27; 0.34/0.35 -> 29; 0.36 -> 30 |
| egui_tiles | 0.17.1 (18/ago/2026), egui 0.36 | via egui | para egui 0.31 use 0.12.0 |
| egui_dock | 0.21.1 (06/ago/2026), egui 0.36 | via egui | para egui 0.31 use 0.16.0 |
| Slint | 1.18.1 (21/set/2026) | feature `unstable-wgpu-30` (e -29) | 1.12.0 (16/jun/2025) tinha `unstable-wgpu-24`; 1.13 -> 26; 1.14 -> 27; 1.15/1.16 -> 27/28; 1.17 -> 28/29 |
| iced | 0.14.0 (07/dez/2025), sem release ha 10 meses | wgpu 27 | |
| Bevy / bevy_egui | 0.20.0 (08/out/2026) / 0.43.1 (10/out/2026) | wgpu 30 (Bevy 0.19 = 29) | bevy_egui 0.43 usa egui 0.36 |
| Xilem/Masonry | 0.4.0 (29/out/2025) | vello 0.6 (wgpu 26 segundo o relatorio de terceiros) | vello avulso ja esta em 0.11.0 (02/out/2026); crates.io do xilem ficou ~1 ano atras |
| Dioxus Native (Blitz) | 0.7.10 / 0.8.0-alpha.1 (30-31/jul/2026) | via anyrender-vello (nao apurado) | blitz-dom 0.3.0-beta.2 (24/ago/2026) |
| GPUI (Zed) | 0.2.2 (22/out/2025), sem release desde entao | nao usa wgpu (Metal, D3D11, blade) | main migrou o Linux para wgpu em fev/2026, nao publicado |
| Makepad | 1.0.0 (13/mai/2025) | nao usa wgpu (abstracao GPU propria) | |
| Floem | 0.2.0 (nov/2024), wgpu 22 | main usa wgpu 27, fork de winit via git | nao publicavel |

### Detalhes por criterio (o que importa para um editor 3D)
**egui + eframe (recomendado)**
- Viewport proprio no mesmo Device: `egui_wgpu::CallbackTrait` (`prepare` recebe device/queue/encoder; `paint` recebe o render pass; `callback_resources` guarda seus pipelines), e `WgpuSetup::Existing` aceita instance/device ja criados. Verificado em https://github.com/emilk/egui/blob/main/crates/egui-wgpu/src/renderer.rs. Padrao mais seguro: renderizar o `anigo-renderer` numa textura offscreen e exibir como imagem (`register_native_texture`).
- Docking: egui_tiles (mantido por emilk/Rerun) e egui_dock, ambos atualizados em 2026. Tabs, splits e arrastar painéis existem; janelas flutuantes de verdade dependem de viewports do egui (nao na web).
- Acessibilidade: AccessKit e dependencia obrigatoria desde 0.34 e ligada por padrao no eframe; 0.35 trouxe `egui_inspection`/`egui_mcp`. Ha o harness `egui_kittest` para testes de UI.
- IME/PT-BR: 0.35 melhorou composicao (sublinhado, cursor); 0.36 (05/ago/2026) corrigiu IME/autocorrecao em iOS/Android web. Acentos PT-BR via winit funcionam. Lacunas: sem bidi/RTL, sem fallback de fonte do sistema (embuta uma fonte com CJK se houver nomes em japones).
- Drag & drop: arquivos do SO built-in; arrastar/soltar interno via API de payload do egui. Menus nativos: nao (issue #3411). Dialogos: `rfd`.
- Risco: breaking changes 2-4 vezes por ano (0.34 renomeou `App::update` para `App::ui`); bus factor 1-2 (financiado pela Rerun); imediato = relayout todo frame (ok para dezenas de painéis, cuidado com asset browser: virtualize com `ScrollArea::show_rows`).
- Release notes: https://github.com/emilk/egui/releases

**Slint**: DSL declarativa com live-preview, AccessKit, IME ok. Integracao wgpu e `unstable-*` (sem garantia de API), via `set_rendering_notifier` / `Image::try_from(wgpu::Texture)` (https://slint.dev/blog/slint-1.12-released). Sem docking embutido. Licenca: GPLv3, Royalty-Free 2.0 (exige atribuicao visivel) ou paga. Bom, mas docking, timeline e grid de assets seriam todos custom.

**iced**: sem AccessKit (issue #552 aberta desde 2020), `pane_grid` e simples (splits, sem tabs/flutuantes), wgpu 27 parado desde dez/2025. Widget `shader` permite pipeline proprio. Risco medio-alto.

**Bevy + bevy_egui**: so faz sentido se voce trocar o `anigo-renderer` pelo renderer do Bevy. Bevy 0.20 (08/out/2026) tem Feathers (widgets "de editor") e BSN, mas nao ha docking nem editor oficial ainda (https://bevy.org/news/bevy-0-20/). Reescrita grande, ciclo de breaking changes trimestral.

**Dioxus Native/Blitz**: o proprio relatorio de terceiros chama de pre-alfa ("not yet recommend building apps with it"); 1 mantenedor principal. **Xilem**: alfa, crates.io defasado. **GPUI**: sem widgets (precisa de gpui-component, terceiros), a11y so na main desde 27/mai/2026, sem release em 1 ano. **Makepad**: nao e wgpu, nicho. **Floem**: crates.io 2 anos parado, sem a11y. Todos descartados para este caso.

## 2. Hot reload em Rust (2026)
- **Subsecond / `dx serve --hotpatch`** (Dioxus CLI 0.7.10, 30/jul/2026; 0.8.0-alpha.1 31/jul/2026). Patch binario do Rust em processo vivo; marque pontos com `subsecond::call(|| ...)`. Windows, Linux e macOS suportados. https://dioxuslabs.com/learn/0.7/essentials/ui/hotreload
  - Funciona fora do Dioxus: docs citam integracoes com Bevy e Iced. Para egui existe https://github.com/lucasmerlin/egui-subsecond-example (egui 0.32, ultimo commit set/2025): `dioxus_devtools::connect_subsecond(); subsecond::call(|| eframe::run_native(..))`, `subsecond::call` dentro do `ui()` e `subsecond::register_handler` chamando `ctx.request_repaint()`. Prova de conceito, nao produto.
  - Limites: era so o "tip crate"; **0.7.4 (28/mar/2026) adicionou hot-patch de workspace** e 0.7.6 corrigiu cache thrashing (https://github.com/DioxusLabs/dioxus/releases). Globais novas nao rodam destrutores e inicializadores estaticos nao sao observados. Em 0.8.0-alpha.0 (19/mai/2026) o hotpatch passa a ser padrao e foi removido o "rust-lld" como linker customizado no Windows (sinal de que Windows teve atrito com lld). Ainda rotulado experimental. Mudar layout de structs com estado vivo exige reinicio [NAO VERIFICADO].
  - Nao testei com o ANIGO; validar numa branch com egui + wgpu antes de adotar.
- **Reinicio rapido**: `bacon` 3.26.0 (26/set/2026) ou `cargo watch` (8.5.3, ultima release out/2024, manutencao parada) para rebuild+relaunch. cargo-watch e legado; prefira bacon.
- **Linker**: desde Rust 1.90 (set/2025) `rust-lld` e o padrao em x86_64-linux-gnu (https://blog.rust-lang.org/2025/09/01/rust-lld-on-1.90.0-stable/). No Windows MSVC o `link.exe` ainda e o padrao; relato da Embark: LLD 2x mais rapido em full e 5x em incremental (https://github.com/EmbarkStudios/rust-ecosystem/issues/15, dado antigo). Teste `-Clinker=rust-lld` no `.cargo/config.toml` e meca. mold: so Linux.
- **Cranelift** (`[unstable] codegen-backend = true` + `[profile.dev] codegen-backend = "cranelift"`): ainda distribuido como componente **nightly** (Linux, macOS, Windows x86_64), https://github.com/rust-lang/rustc_codegen_cranelift. Ganho tipico 25-30% em build de dev; nao use para o perfil que voce roda com wgpu pesado sem medir (shaders naga/wgpu ficam lentos em debug).
- **Perfil dev recomendado** [pratica geral, nao verificada nesta pesquisa]: `debug = "line-tables-only"`, `[profile.dev.package."*"] opt-level = 2` (wgpu/naga/image/glam) e `opt-level = 1` no workspace.
- **Shaders WGSL**: `notify` (ultimo 9.0.0-rc.5, 30/ago/2026) vigiando `crates/anigo-renderer/shaders/*.wgsl`; em debug leia do disco em vez de `include_str!`; valide com `device.push_error_scope(Validation)` + `create_shader_module` + `pop_error_scope` e so troque o pipeline se passar (erro vira overlay no painel, nao crash). `wesl` 0.6.0 (04/out/2026) adiciona imports ao WGSL se precisar. Bevy ja faz isto, o padrao e conhecido.
- **Assets**: mesmo watcher mapeando extensao -> recarregador (textura, VRM, material); invalide por hash/mtime.
- **Persistir sessao**: serde (ja no workspace) -> `session.json` versionado em `%APPDATA%/anigo/`, salvo com debounce e no encerramento: projeto aberto, camera, selecao, tempo da timeline e layout de docking. egui_tiles e egui_dock tem feature serde [NAO VERIFICADO, confirme no Cargo.toml deles]; eframe tem feature `persistence`. No startup restaura tudo, entao todo reinicio parece um hot reload de ~poucos segundos.

**Fluxo recomendado (edit -> ver)**
1. UI/inspector/painéis em crate "tip" (`anigo-app`, bin): `dx serve --hotpatch` (ou subsecond direto): segundos, estado preservado.
2. Shaders/assets: watcher, menos de 1 s, sem rebuild.
3. Mudancas em crates de engine (core, renderer, ik...): rebuild incremental (lld + perfil acima) + relaunch por bacon + restauracao de sessao. Meta de ~10 s e uma expectativa a medir, nao um numero verificado. Referencia: o app-exemplo do relatorio de terceiros mediu ~1 s incremental / 103 s build limpo com egui.

## 3. Tauri so como visualizador (wgpu nativo + WebView)
- Versoes: tauri 2.12.3 (10/out/2026), 3.0.0-alpha.4 (01/out/2026), wry 0.57.0, tao 0.37.1 (https://v2.tauri.app/release/tauri/).
- Nao existe solucao estavel/oficial em 2026 para wgpu + WebView compartilhando textura. O que existe sao gambiarras:
  - Janela nativa (winit+wgpu) com WebView filha por cima; o exemplo wgpu do wry usa winit com o webview como janela filha X11. Transparencia + wgpu + Tauri v2 gera flicker (https://github.com/tauri-apps/tauri/issues/9220, resposta do mantenedor amrbashir).
  - Janela overlay transparente/click-through sobre o webview; ou renderizar headless e mandar bytes ao canvas (relato de ~300 ms em PC fraco, input dificil) - https://github.com/orgs/tauri-apps/discussions/11944. Plugins comunitarios de 2026 para overlay (Steam) existem para Windows e macOS, nao para Linux/Wayland.
  - Multiwebview no Tauri ainda sob feature `unstable`.
- Pros: reaproveita a UI Svelte, acessibilidade/IME/PT-BR gratis do navegador, ecossistema de plugins.
- Contras: "airspace" (superficie nativa sempre acima do DOM, z-index nao vale), input e DPI em duas camadas, posicionamento via IPC assincrono, Linux/WebKitGTK com bugs conhecidos em NVIDIA, e a UI continua em JS/TS (contraria o objetivo 100% Rust).
- Veredito: ponte de transicao no maximo (hoje voce ja tem esse caminho), nao destino.

## 4. Recomendacao final (ranking para editor 3D anime, equipe pequena, Windows principal)
1. **egui/eframe 0.36 + egui_tiles (ou egui_dock) + wgpu 30 + subsecond.** Unico candidato que junta: viewport wgpu compartilhando Device sem atrito, docking maduro, AccessKit, release mensal/trimestral, 1000+ crates dependentes, exemplo de hot-patch. Preco: subir wgpu 24 -> 30 (PR de preparacao so no `anigo-renderer`), churn de API do egui, imediato = menos "polish" nativo (sem menus nativos, sem bidi, fonte embutida).
2. **Slint 1.18** (se preferir UI declarativa/live-preview): wgpu instavel, sem docking, licenca com atribuicao; mais trabalho custom que egui para o mesmo resultado.
3. **Bevy 0.20 + bevy_egui 0.43**: so se aceitar trocar o renderer proprio pelo Bevy (reescrita grande, mas editor/ECS prontos).
4. **iced 0.14**: sem a11y, wgpu 27 parado.
5. **Tauri como visualizador**: apenas transicao. Descartar: Dioxus Native, Xilem, GPUI, Makepad, Floem (alfa, sem release, ou sem wgpu).
- C++ via FFI: nada na pesquisa exige; so vale para libs prontas (ex.: cloth/physics/OpenUSD) atras de uma crate `-sys`.

### Plano sugerido de migracao
1. PR "wgpu 30" no workspace + `anigo-renderer` renderizando em textura offscreen.
2. Novo bin `anigo-app` (eframe + egui_tiles) com viewport, inspector, asset browser e timeline; Tauri segue como alternativa ate paridade.
3. Watchers de shader/asset + `session.json`.
4. Spike de 1 dia com `dx serve --hotpatch` no `anigo-app` e decida se vira fluxo padrao ou fica no bacon.
