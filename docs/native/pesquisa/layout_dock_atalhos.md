# Pesquisa: layout dockável persistido, atalhos configuráveis e barra de menus no ANIGO Studio nativo (T-2.12, out/2026)

Legenda (a mesma das outras pesquisas): **[V]** = fonte lida nesta sessão (documentação oficial, código-fonte do crate baixado/cacheado, API do crates.io);
**[S]** = visto só em trecho de busca ou fonte secundária (fórum, blog); **[C]** = conhecimento prévio/inferência, NÃO verificado agora.
Estado em 2026-10-12. Limitações: WebFetch falha por DNS; o Firecrawl respondeu, mas a página do Unreal sobre layouts só apareceu em fóruns [S]; o Godot só em trecho [S].

## 1. Ponto de partida (código lido no repo) [V]

* Cada workspace desenha painéis FIXOS com `egui::Panel::{left,right,top,bottom}(id).default_size(..).size_range(..)` dentro de `app.rs` / `*_ui/panels.rs`, e o viewport
  com `egui::CentralPanel` (frame `NONE`). O egui só lembra a **largura** de cada painel (memória do egui, por id) — não há reorganização, fechar/reabrir, nem
  layout por workspace persistido na sessão.
* Painéis por workspace hoje: Personagem (inspector direito), Modelagem (inspector direito + editor de UV inferior opcional), Posing (inspector), Shading (esquerdo + direito),
  Iluminação (inspector), Cenário (Posicionar opcional à esquerda, direita com Outliner/Histórico + Detalhes, gaveta de conteúdo inferior opcional), Render (direito + fila inferior opcional),
  Animação (inspector + timeline inferior), Biblioteca (esquerdo, central, direito, faixa inferior). Barra superior (workspaces), shelf de ferramentas (46 px) e barra de status são "chrome" fixo.
* Atalhos: ~90 chamadas `consume_key`/`key_pressed` espalhadas em `character_shortcuts`, `scene_hotkeys`, `handle_hotkeys`/`edit_hotkeys` (Modelagem, Blender-like), `shading_shortcuts`,
  `posing_shortcuts`, `animation_shortcuts`, escultura, cabelo, UV, Biblioteca. Nenhuma é configurável; não há menu de aplicativo (só botões de desfazer/refazer e o idioma na barra superior).
* `SessionState` é JSON versionado (`SESSION_VERSION = 1`) com campos opcionais `#[serde(default)]` — a forma segura de acrescentar o layout sem quebrar sessões antigas.

## 2. Bibliotecas de docking para egui 0.36

| Crate | Versão / compat | O que dá | Limites |
|---|---|---|---|
| **egui_tiles** (rerun-io) | 0.17.1 (2026-08-18) depende de `egui ^0.36.0` [V: API do crates.io e `Cargo.toml` do crate cacheado em `~/.cargo/registry`]; CHANGELOG "Update egui to 0.36 #152" [S] | árvore de **containers** (Linear horizontal/vertical com `shares`, Tabs, Grid) + panes; arrastar-e-soltar de abas (dividir/mesclar/reordenar), redimensionar, `Behavior` (trait) para desenhar pane/aba/fechar, simplificação automática (poda de containers vazios), `unsafe` proibido [V: leitura de `behavior.rs`, `tree.rs`, `tiles.rs`, `lib.rs`] | **não tem janelas flutuantes** [V: nenhuma API no código; README lista "layouts horizontal/vertical, tabs, drag-and-drop docking"]; serde da `Tree` é detalhe interno do crate (formato pode mudar entre versões) [V: `#[cfg_attr(feature = "serde", ..)]`] |
| **egui_dock** | 0.21.1 (2026-08-06) depende de `egui ^0.36` [V: API do crates.io] | árvore **binária** de splits com fração; **janelas flutuantes** (`DockState::add_window`, `detach_tab`), `hidable_tab_bars`, `retain_tabs`, serde [V: código baixado] | modelo binário (cada split = 2 filhos) é menos natural para "colunas" com N painéis; mais opinativo no visual das abas |
| dock próprio mínimo | — | controle total | reimplementar hit-test de soltar, previews, tabs, redimensionamento: semanas de trabalho e de bugs; sem ganho para o escopo |

**Decisão: `egui_tiles` 0.17.** Há versão compatível (0.17.1 ↔ egui 0.36), mantida por quem mantém o egui em passo com ele (a 0.17.0 saiu 2026-08-05, a 0.16.0 em 2026-06-26 — acompanham as versões do egui), a
topologia Linear+Tabs reproduz exatamente os layouts atuais (colunas laterais de largura/shares, bottom sob o viewport) e o `Behavior` deixa cada painel continuar sendo um método `StudioApp` que desenha em um `Ui`.
Consequências assumidas:

1. **Formato de persistência PRÓPRIO** (`LayoutNode`: `Split{vertical, filhos+shares}` / `Tabs{filhos, ativo}` / `Pane(id estável)`), convertido de/para a `Tree` do `egui_tiles`. Assim a migração de esquema, a tolerância a ids
   desconhecidos e os testes são independentes da versão do crate, e dá para testar sem egui.
2. **Janelas flutuantes**: o crate não permite. Plano: fase "avançada" implementa "Flutuar painel" como `egui::Window` do corpo do painel (fora da árvore), persistido no modelo; o `egui_dock` fica documentado como alternativa se
   o docking real para fora da janela principal for exigido (trocar o miolo de `dock/` é local, pois o modelo persistido é nosso).

## 3. Padrões de UX nos editores de referência

* **Blender** — "áreas" com editores, agrupadas em **Workspaces** (cada workspace guarda seu layout de tela); botão de duplicar workspace, alternar com Ctrl+PgUp/PgDn; resetar = adicionar um workspace de fábrica / "Reset to Default" [S: manual
  Workspaces + Blender Artists]. **Keymap** (Preferências → Keymap) [V: manual 5.2]: presets (Blender, Blender 27x, Industry Compatible), **filtro por nome ou por atalho atual**, botão "clique e pressione o atalho", checkbox
  *Active* por item, **Restaurar** por mapa, **Import/Export** de preset (`.py`) e opção "All Keymaps" desligada = exporta só o **delta** (o que o usuário modificou) — modelo que adotamos no JSON; aviso do manual: personalizar demais aumenta o
  risco de **conflitos** em versões novas → usamos **ids estáveis de ação** e guardamos só o delta.
* **Unreal Engine** — abas dockáveis, menu **Window** lista cada painel (marca os abertos) e tem **Save Layout / Load Layout / Reset Layout** [S: fóruns da Epic sobre "Save Layout"/"Load a saved layout"]; atalhos em
  Editor Preferences → Keyboard Shortcuts, com **contextos** (ex.: "Level Editor", "Viewport") [S]. O esquema de atalhos do Cenário já é UE-like (Q/W/E/R).
* **Unity** [V: manual 6.6 "Customize your workspace layout"] — o Editor é "uma coleção de abas"; qualquer janela do menu abre como aba; clique direito na aba: maximizar/fechar/adicionar; arrastar o título mostra o contorno
  da zona de soltar; abas podem ir para **janelas flutuantes**; menu **Layout**: *Save Layout* (sobrescreve um de fábrica ou cria nome novo), *Save layout to file*, *Load layout from file*, *Delete Layout*, *Reset all Layouts*.
  Fórum [S]: layout corrompido → diálogo "Failed to load window layout" com opção de usar o layout padrão ou o de fábrica ⇒ nossa política: arquivo ilegível nunca derruba o app, cai no padrão e **faz backup** (`.bak`).
* **Godot** [S: manual "Customizing the interface"] — menu *Editor → Editor Layouts → Save…* (dá nome), *Delete*, e layouts predefinidos *Default/Classic*; o editor salva o layout do projeto quando ele muda
  (`queue_save_layout` na API do `EditorPlugin`) ⇒ salvamento **no ato da mudança** (debounce até soltar o mouse), não só ao fechar.
* **VS Code** [S: docs de keybindings e "when clause contexts"] — editor de atalhos com busca, "Configure Keybinding" a partir de qualquer item de UI, **`when` clauses** (contexto) e **chords** (sequências como Ctrl+K Ctrl+C);
  conflito = mesma combinação em contextos que podem estar ativos ao mesmo tempo.

## 4. Proposta de escopo em fases (o que este trabalho entrega)

### MVP (obrigatório)
1. **Registro de painéis**: cada workspace declara `PanelSpec{id estável, título i18n, ícone, tipo (viewport|painel), fechável, flag opcional de visibilidade}` e uma função de desenho; o viewport é um painel especial (não fecha, obrigatório).
2. **Layout padrão por workspace** reproduz o atual (larguras em px convertidas em `shares` no primeiro quadro); painéis sem aba ficam "nus" com um **cabeçalho fino** (título + alça de arrastar + fechar) e só ganham barra de abas quando agrupados — assim o viewport não perde área.
3. **Dock ao vivo** (egui_tiles): arrastar painel para dividir/agrupar em abas/reordenar; fechar (X); **Janela → Painéis** reabre; **Janela → Redefinir layout** (por workspace).
4. **Persistência**: JSON versionado (`schema`) no diretório de dados do usuário (`<ANIGO>/Config/layouts.json`, `ANIGO_CONFIG_DIR` sobrescreve; `--layout-file` explícito) + cópia dentro de `SessionState` (campo opcional). Carregamento
   tolerante: JSON corrompido/esquema futuro → padrão + `.bak`; ids de painel desconhecidos são descartados; painéis obrigatórios ausentes (viewport) → reinsere; ids duplicados → mantém o primeiro.
5. **Atalhos**: registro central de **ações** (id estável `escopo.acao`, rótulo i18n, contexto, atalhos padrão = os ATUAIS); `Keymap` com só o **delta do usuário** persistido (`keymap.json`); painel de preferências
   (busca por nome/atalho, captura, conflitos por contexto, restaurar, importar/exportar JSON); handlers `*_shortcuts` consultam o registro.
6. **Menus**: Arquivo / Editar / Exibir / Janela / Ajuda ligados a ações reais, mostrando o atalho; itens sem implementação ficam desabilitados com explicação (nada morto).
7. **Testes** (modelo, migração, corrupção, conflitos, roteamento de ação) + cenários `--scenario layout-*` + smoke xdotool em Xvfb próprio.

### Completo (se couber)
* **Presets nomeados** de layout (salvar/aplicar/apagar por workspace) e exportar/importar arquivo de layout.
* Biblioteca dockável (sidebar/browser/detalhes), Cenário com Outliner/Histórico/Detalhes como painéis separados.
* "Flutuar painel" (janela egui do corpo do painel) e "Maximizar painel" (Ctrl+Espaço-like).

### Avançado (fora deste trabalho; documentado)
* **Chords** de duas teclas (Ctrl+K Ctrl+C), atalhos com botão do mouse/modificadores de gesto (navegação do viewport), esquemas de keymap alternativos (preset "Blender"/"Unreal"/"Maya"),
  `when` clauses arbitrárias em vez de contextos fixos, janelas flutuantes fora da janela principal (múltiplos viewports do egui/egui_dock), sincronizar layouts na nuvem.

## 5. Modelo de atalhos (decisões)

* **Ação** = `{id, label_key, contextos[], padrão[]}`. **Contexto** = `{workspace?, modo?, área}` onde `área ∈ {Qualquer, Viewport, Timeline}` (os handlers atuais só disparam com o ponteiro no viewport, ou na timeline).
  Dois atalhos **conflitam** se a mesma combinação está ligada a ações DIFERENTES cujos contextos se sobrepõem (global ∩ qualquer; mesmo workspace; modo igual ou sem modo; áreas iguais ou uma "Qualquer").
  Os padrões atuais NÃO podem gerar conflito (teste), exceto onde o app já faz o mesmo comando por dois caminhos.
* **Combinação** = tecla + `Ctrl/Cmd` + `Shift` + `Alt` (`Ctrl` = `COMMAND` do egui: Cmd no macOS). Texto canônico `"Ctrl+Shift+Z"`.
* **Persistência** só do delta: `{schema, bindings:{id:[combos]}}` (lista vazia = ação desligada). Ids desconhecidos são ignorados (tolerância a versões), combinações inválidas viram erro de importação sem derrubar o resto.
* Teclas MODAIS de transformação (Enter/Esc/X/Y/Z/dígitos durante G/R/S), pincel/Esc de cancelamento e navegação de grade da Biblioteca **não** entram no registro nesta fase (são parte do gesto, não "comandos") — listadas em `LAYOUT_ATALHOS.md`.

## 6. Divisão nuvem vs. local

* Nuvem (Xvfb + lavapipe): testes unitários, capturas `--scenario layout-*`, smoke xdotool (arrastar aba, fechar/reabrir, reiniciar com `--session-file`).
* Exige validação humana/local: sensação de arrastar abas com mouse/tablet reais, HiDPI/escala de fonte nas abas, atalhos em teclados não-US (códigos físicos vs. lógicos do egui), macOS (`Cmd`) e Wayland.
