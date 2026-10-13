# Layout dockável, atalhos configuráveis e menus — UI nativa (`anigo-studio`)

**Estado (T-2.12):** MVP + "completo" de [`pesquisa/layout_dock_atalhos.md`](pesquisa/layout_dock_atalhos.md) implementados e validados no sandbox (Xvfb + lavapipe,
xdotool em display próprio). **Não** foi sentido com mouse/tablet reais, HiDPI, teclados não-US nem Wayland/macOS (ver §10).

Legenda: **[T]** teste automático que rodou · **[S]** visto em captura ou em entrada real (xdotool) no Xvfb · **[N]** não validado.

## 1. O que existe

| Área | O que faz | Validação |
|---|---|---|
| **Painéis registráveis** (`layout/registry.rs`) | cada workspace declara seus painéis: id estável (`cenario.outliner`), título i18n pt/en, ícone, de onde reaparece (`Home`) e se a visibilidade é uma flag do app. O viewport é o painel central, obrigatório (não fecha). 9 workspaces, 26 painéis. A função de desenho de cada painel é um `match` por id em `app/dock_ui.rs`, que chama os mesmos métodos `StudioApp` de antes | [T] ids únicos e com o prefixo do workspace, título traduzido, central único |
| **Layout padrão = o antigo** | larguras/alturas em px dos painéis fixos do egui viram `shares` no primeiro quadro (§3) | [T] `default_widths_match_the_old_fixed_panels`; [S] traces e capturas (§8) |
| **Dock ao vivo** (`egui_tiles` 0.17) | arrastar o cabeçalho de um painel (ou a aba) para a borda de outro = **dividir**; para o centro = **agrupar em abas**; arrastar o divisor = redimensionar; X = fechar. Painel sozinho tem um **cabeçalho fino** (alça + título + X); só ganha barra de abas quando agrupado — assim o viewport não perde área | [S] smoke xdotool: arrastar Detalhes para a esquerda do viewport muda a árvore (92 000+ px de diferença na tela) |
| **Fechar / reabrir** | X do cabeçalho/aba; **Janela → Painéis** (caixas de marcar) reabre no lugar de origem. Painéis condicionais (gaveta e Posicionar do Cenário, fila do Render, editor de UV da Modelagem) seguem as flags antigas: fechar desliga a flag e ligar a flag (botão antigo, Ctrl+Espaço, enfileirar um render) reabre o painel — **no lugar onde estava**, se o resto do layout não mudou | [T] `flag_driven_panels_follow_the_flag_and_come_back_where_they_were`; [S] smoke (fechar Posicionar pelo X, reabrir pelo menu) |
| **Redefinir layout** | Janela → Layouts → *Redefinir layout deste workspace* / *Redefinir todos* (volta às flags padrão e ao padrão em px da janela atual) | [S] smoke |
| **Persistência** | JSON versionado (`schema: 1`) em `<ANIGO>/Config/layouts.json` (`ANIGO_CONFIG_DIR` sobrescreve; `--layout-file` explícito), gravado ao soltar o mouse (nunca no meio do arrasto) e **dentro da sessão** (`SessionState.layouts`, opcional: sessões antigas abrem com o padrão). Precedência: `--layout-file` > sessão > arquivo do usuário. `--no-session` não lê nem grava o arquivo do usuário (capturas determinísticas) | [T] 25 testes de modelo; [S] smoke: reiniciar com `--session-file` e config **vazia** devolve o layout com **0 px** de diferença |
| **Tolerância** | JSON corrompido, não-UTF8, esquema futuro → layout padrão + cópia `.bak` do original + aviso (Sobre); entrada ruim (um workspace/preset) é descartada sem perder as outras; ids de painel desconhecidos/repetidos saem, containers vazios somem, `share` inválido (NaN, negativo, zero) é corrigido, painel obrigatório ausente → padrão; **nunca** pânico. Bloco `layouts` ilegível dentro da sessão vira "sem layout" e não derruba a sessão | [T] `corrupted_file_falls_back_without_panic_and_keeps_a_backup`, `a_bad_entry_is_dropped_but_the_rest_loads`, `garbage_or_future_layouts_never_break_the_session`, `unknown_and_duplicate_panes_are_dropped…` |
| **Presets nomeados** | Janela → Layouts → *Salvar layout atual como…* / *Layouts salvos* (aplicar, excluir), por workspace; exportar/importar todos os layouts em arquivo (`.json`). Aplicar um preset ajusta as flags dos painéis condicionais ao que ele contém | [T] salvar/sobrescrever/excluir/mesclar; [S] smoke: salvar "meu", redefinir, aplicar → **0 px** de diferença |
| **Atalhos configuráveis** (`keymap/`) | registro central de **ações** (§6) com atalhos padrão = os que já existiam; janela de preferências (busca por nome/id/atalho, captura da próxima combinação, vários atalhos por ação, botão direito remove, restaurar por ação/tudo, conflitos em vermelho, importar/exportar JSON) | [T] 14 testes de keymap + 3 de i18n; [S] capturas |
| **Menus** | Arquivo / Editar / Exibir / Janela / Ajuda (§7), mostrando o atalho da ação | [S] capturas e smoke (menus clicados de verdade) |

## 2. Como usar

| Quero | Faço |
|---|---|
| Mover um painel | arrasto o cabeçalho (ícone de seis pontos) ou a aba; a zona de soltar aparece em destaque |
| Agrupar em abas | solto no centro de outro painel |
| Dividir | solto na borda (esquerda/direita/cima/baixo) |
| Fechar / reabrir | X; **Janela → Painéis** |
| Voltar ao padrão | **Janela → Layouts → Redefinir layout deste workspace** |
| Guardar um arranjo | **Janela → Layouts → Salvar layout atual como…** |
| Levar o layout para outro PC | **Janela → Layouts → Exportar layouts…** (caminho do `.json`) e **Importar layouts…** |
| Mudar um atalho | **Ajuda/Janela → Atalhos de teclado…**, clico no atalho (ou em `+`) e pressiono a nova combinação (Esc cancela; botão direito num atalho remove) |
| Linha de comando | `--layout-file <json>` · `--keymap-file <json>` · `ANIGO_CONFIG_DIR=<pasta>` · `ANIGO_TRACE_LAYOUT=1` (imprime `layouttrace` com a estrutura e o retângulo de cada painel) |

## 3. Modelo e formato

`LayoutNode` (serde, `layout/model.rs`): `Split{vertical, children:[{share, node}]}` · `Tabs{children, active}` · `Pane{id}`. É **o nosso** formato — convertido de/para a
árvore do `egui_tiles` (`layout/tiles.rs`) — para o arquivo não depender da versão do crate. As `shares` do padrão são **pixels** na janela atual (ex.: Cenário a 1234 px de largura:
Posicionar 150 | viewport 744 | direita 340), então o padrão reproduz exatamente os painéis fixos de antes; depois disso valem as proporções (a janela redimensionada escala as colunas).

```json
{ "schema": 1,
  "workspaces": { "animacao": { "kind": "split", "vertical": false, "children": [
      { "share": 877, "node": { "kind": "split", "vertical": true, "children": [
          { "share": 300, "node": { "kind": "pane", "id": "animacao.viewport" } },
          { "share": 400, "node": { "kind": "pane", "id": "animacao.timeline" } } ] } },
      { "share": 357, "node": { "kind": "pane", "id": "animacao.inspector" } } ] } },
  "presets": [ { "name": "viewport grande", "workspace": "animacao", "layout": { "…": "…" } } ] }
```

Exemplos versionados (usados pelas capturas e validados por teste): [`layouts/cenario_modelagem_de_cena.json`](layouts/cenario_modelagem_de_cena.json) e
[`layouts/animacao_timeline_alta.json`](layouts/animacao_timeline_alta.json).

Migração (`LayoutStore::migrate`): `schema` ausente (arquivo escrito à mão) = v0, tratado como v1 com campos faltantes preenchidos; esquema **maior** que o suportado é recusado
(padrão + `.bak`). Cada versão futura acrescenta um passo `vN → vN+1` ali.

## 4. Painéis por workspace e layout padrão

| Workspace | Painéis (id) | Padrão (a 1280×800) |
|---|---|---|
| Personagem | `viewport`, `inspector` | inspetor 350 px à direita |
| Modelagem | `viewport`, `inspector`, `uv` (condicional: modo Edição + flag) | inspetor 300 px à direita; editor de UV 360 px sob o viewport |
| Posing | `viewport`, `inspector` | inspetor 360 px |
| Shading | `viewport`, `targets`, `params` | alvos/presets 250 px à esquerda; parâmetros 350 px à direita |
| Iluminação | `viewport`, `inspector` | inspetor 350 px |
| Cenário | `viewport`, `place` (flag), `outliner`+`history` (abas), `details`, `drawer` (flag) | Posicionar 150 px à esquerda; coluna direita 340 px (Outliner/Histórico acima, Detalhes abaixo); gaveta 250 px sob o viewport |
| Animação | `viewport`, `inspector`, `timeline` | inspetor 360 px à direita; timeline 330 px sob o viewport |
| Render | `viewport`, `settings`, `queue` (flag) | ajustes 370 px à direita; fila 230 px na largura toda embaixo |
| Biblioteca | `browser` (central), `sidebar`, `details` | navegação 250 px; detalhes 330 px; a faixa de status continua fixa |

Fora do dock (cromo fixo): menu, barra de workspaces, shelf de ferramentas, barra de status. Medido com `ANIGO_TRACE_LAYOUT=1` a 1280×800: todas as larguras acima batem com os
`default_size` antigos (±1 px do divisor de 1 px).

## 5. Persistência: onde cada coisa vive

| Dado | Onde | Quando grava |
|---|---|---|
| Layouts + presets | `<ANIGO>/Config/layouts.json` (ou `--layout-file`) | ao soltar o mouse depois de mudar/fechar/abrir/redefinir; presets ao salvar/excluir |
| Layouts na sessão | `SessionState.layouts` (campo opcional) | ao fechar / salvar a sessão |
| Atalhos | `<ANIGO>/Config/keymap.json` (ou `--keymap-file`), **só o delta** | a cada mudança na janela de atalhos |
| Flags dos painéis condicionais | `ScenePrefs`/`RenderPrefs` da sessão (como antes) | ao salvar a sessão |

`<ANIGO>` = `Documents/ANIGO` (`library::studio_data_dir`); `ANIGO_CONFIG_DIR` troca a pasta `Config`.

## 6. Atalhos configuráveis

Registro em `keymap/actions.rs` (128 ações): `{id, contextos, atalhos padrão}`; o rótulo é `act.<id>` (pt/en em `i18n_dock.rs`). Ids por workspace (`cenario.tool.move`,
`modelagem.edit.extrude`, `animacao.play`, `posing.copy`, `material.copy`…) e `edit.undo`/`edit.redo` globais.

**Contexto** = workspace (ou global) + modo da Modelagem (Objeto/Edição/Escultura/Cabelo) + área (qualquer, viewport 3D, timeline, editor de UV). Dois atalhos iguais em ações diferentes
**conflitam** só se algum par de contextos pode estar ativo junto (global colide com tudo; R é "escalar" no Cenário e "girar" na Modelagem sem conflito). Os padrões não têm conflito
(teste). O painel marca em vermelho e lista com quem colide; **não bloqueia** (como o Blender).

Padrão (os handlers antigos, sem mudança de comportamento por padrão):

| Escopo | Atalhos |
|---|---|
| Global | Ctrl+Z desfazer · Ctrl+Shift+Z / Ctrl+Y refazer |
| Modelagem (Blender-like, ponteiro no viewport) | Tab Objeto/Edição · G/R/S modal · A / Alt+A / Ctrl+I seleção · Ctrl++ / Ctrl+− expandir/reduzir · Alt+Z raio-X · Shift+Z sólido/aramado · `.`/Home enquadrar · Objeto: X/Delete excluir, Shift+D duplicar · Edição: 1/2/3, L, O, E, I, Ctrl+B, Ctrl+R, K, X/Delete, M, F, Ctrl+T, Ctrl+N, Alt+H, H · Escultura: F, Shift+F, `[`, `]` · Cabelo: D/V/C, Shift+D, X/Delete, Shift+X, M, Shift+M, `[`, `]` · Editor de UV: G/R/S, A, Alt+A, Ctrl+I, L, 1/2/3, Home, `.` |
| Cenário (Unreal-like) | Q/W/E/R ferramentas · Espaço alterna · Ctrl+\` espaço mundo/local · Ctrl+Espaço gaveta · Delete/Backspace · Ctrl+W duplicar · Ctrl+A/Ctrl+I · Shift+E mesmo ativo · H / Ctrl+H · Ctrl+G / Ctrl+Shift+G · Ctrl+C/X/V · Esc · F2 · End / Ctrl+End · Z · V · `[` `]` / Shift+`[` `]` · F · G · Alt+G/H/J/K vistas · Alt+2/3/4 sombreamento |
| Posing (e Animação fora da timeline) | Ctrl+C/V/Shift+V · Ctrl+A · Alt+R / Alt+Shift+R · M · F |
| Shading / Iluminação | Ctrl+C/V, F / F |
| Animação | Espaço · ←/→ (±1) · Shift+←/→ (±10) · ↑/↓ · Home/End · I / Alt+I · timeline: Delete/X, Ctrl+C/V/Shift+V, Shift+M, M marcador, Ctrl+A, F |

`Ctrl` = `COMMAND` do egui (Cmd no macOS). Teclas gravadas por nome do egui (`Z`, `1`, `Backtick`, `OpenBracket`, `Right`…); `"Ctrl+Shift+Z"` etc.

**Arquivo de atalhos** (`keymap.json`): `{"schema":1,"bindings":{"cenario.tool.move":["T"],"cenario.hide":[]}}` — só o que mudou (lista vazia = ação desligada). Exportar com
"tudo" escreve o mapa completo; importar normaliza para o delta; ids desconhecidos e combinações inválidas viram **avisos** e o resto entra; esquema futuro/JSON ilegível → padrão + `.bak`.

### 6.1 Correções de ordem encontradas na migração

O egui ignora Shift/Alt extras em `consume_key`; handlers antigos checavam a tecla simples **antes** da com Shift, então estes atalhos documentados nunca disparavam
(a tecla simples consumia o evento). Agora a combinação mais específica é checada primeiro:
**Alt+Shift+R** (zerar a pose inteira, Posing) · **Shift+E** (selecionar mesmo ativo, Cenário) · **Shift+`[` / Shift+`]`** (passo de rotação, Cenário) · **Shift+D** (duplicar mecha, Cabelo) ·
**Shift+←/→** (±10 quadros, Animação). Um padrão com Shift que já funcionava continua igual.

### 6.2 O que NÃO está no registro (e por quê)

* **Teclas modais**: Enter/Esc/Backspace/X/Y/Z/dígitos durante G/R/S (Modelagem e editor de UV) e a trava de eixo X/Y/Z segurada no Posing — fazem parte do gesto.
* **Esc de cancelar gesto** (Cenário, Posing, Animação, Escultura, Cabelo) e **voo da câmera** (WASD/Q/E com o botão direito).
* **Biblioteca**: navegação da grade (setas, Enter, F favorito, Delete, Ctrl+A, Ctrl+F, `/`) — `library_ui` trata as teclas no próprio módulo.
* **Ctrl+C/X/V do sistema**: chegam como eventos de área de transferência (`Event::Copy/Cut/Paste`) e valem sempre; reatribuir a ação não desliga esses eventos.
* **Gestos de mouse** (orbitar/pan/zoom, Alt+clique…) e **dicas de ferramenta** com o atalho escrito à mão (`(Ctrl+Espaço)`, `(Home)`, `(Ctrl+Z)`): não acompanham uma troca de atalho (os menus acompanham).
* Atalhos de **dois toques** (chords) e esquemas completos alternativos: fase "avançada" da pesquisa.

## 7. Menus

| Menu | Itens (todos chamam ações reais) |
|---|---|
| **Arquivo** | Novo projeto (confirma e troca o documento do núcleo por um vazio) · Abrir… / Salvar como… (mesmo diálogo do Personagem; no Cenário, o do Cenário) · Salvar (grava no último caminho; desabilitado sem caminho) · Exportar objeto para a Biblioteca (Modelagem com objeto ativo) · Renderizar imagem (workspace Render) · Sair |
| **Editar** | Desfazer / Refazer (com o atalho; habilitados conforme a sessão do workspace) · Histórico (os últimos 12 passos, mais recente primeiro) |
| **Exibir** | Workspace (9) · Sombreamento do viewport (Modelagem) · Enquadrar seleção (Modelagem/Cenário) · Idioma |
| **Janela** | Painéis (marcar/desmarcar) · Layouts (redefinir, salvar como, salvos, exportar/importar) · Atalhos de teclado… |
| **Ajuda** | Atalhos de teclado (filtrados no workspace atual) · Sobre (versão, GPU, arquivos de layout/atalhos/sessão, avisos de carregamento) |

Itens que não podem rodar agora ficam **desabilitados com a explicação na dica** (ex.: *Renderizar imagem — Disponível no workspace Render*); nada é item morto.

## 8. Regressão visual vs. as referências

`docs/native/img/*` foram regeneradas com o novo layout (`scripts/native_screenshots.sh`); a comparação antes/depois foi feita com o binário anterior (`git`, mesmo cenário, 1280×800):

* **Geometria dos painéis**: igual ao px (Personagem 350, Modelagem 300, Posing 360, Shading 250/350, Iluminação 350, Cenário 150/340/250, Animação 360/330, Render 370/230, Biblioteca 250/330).
* **Diferenças intencionais**: **+24 px** de barra de menus no topo de todas as telas (a viewport fica 24 px mais baixa) e **+20 px** de cabeçalho
  (alça/título/X) em cada painel lateral/inferior — o conteúdo dos painéis é o mesmo. O viewport sozinho NÃO tem cabeçalho.
* Smokes existentes com entrada real (Cenário, Iluminação, Render, Shading, Posing, Animação, Biblioteca, Modo Cabelo, editor de UV, bridge) **passam sem mudança de coordenadas** (leem as posições do trace).
  `native_sculpt_smoke.sh` já falhava antes desta tarefa neste sandbox ("desfazer não restaurou a imagem inicial"), igual com o binário anterior — não é regressão.

Capturas novas: `layout_padrao_<workspace>.png`, `layout_customizado_cenario.png`, `layout_customizado_animacao.png`, `layout_corrompido_usa_padrao.png`, `layout_menu_{arquivo,editar,exibir,janela,ajuda}.png`,
`layout_atalhos.png`, `layout_atalhos_conflito.png`, `layout_atalhos_modelagem.png`, `layout_sobre.png` (cenários `--scenario layout-*`; `layout-<workspace>` só abre o workspace, `layout-menu-*` abre o menu, `layout-keyboard[-conflict]`, `layout-about`).

## 9. Testes

* `layout::model` (14), `layout::registry` (6), `layout::tiles` (5): ida e volta JSON, ids desconhecidos/duplicados, `share` inválido, aba ativa, remover/inserir (esquerda/direita/baixo/sob o viewport), padrão em px, migração, esquema futuro, arquivo corrompido + `.bak`, entrada ruim isolada, presets, reabrir todos os painéis, painéis por flag voltam ao lugar, exemplos em `docs/native/layouts/` válidos.
* `keymap` (14): texto/parse de combinações (sinônimos, erros), padrões sem conflito, conflito por contexto (workspace/modo/área/global), **roteamento** (a mesma tecla vai para a ação certa em cada contexto), delta/export/import, tolerância, arquivo corrompido, `consume` com eventos reais do egui (consome uma vez, usa o atalho trocado), todo id consultado em algum handler existe e toda ação do registro tem handler (varre o código-fonte), tradução pt/en.
* `session`: `layouts` ida e volta; sessão antiga sem o campo; lixo/futuro/tipos errados em `layouts` nunca derrubam a sessão.
* `cli`: `--layout-file`/`--keymap-file`; `app::menu_ui`: texto de contexto legível para toda ação. Total novo: 46 testes (studio: 566 passam, `anigo-core` 301, `anigo-session` 25).
* Smoke `scripts/native_layout_smoke.sh` (Xvfb `:98` próprio; xdotool com `mousedown`/`mouseup` explícitos): padrão do Cenário → arrastar Detalhes → layout gravado no arquivo → fechar Posicionar pelo X → reabrir por Janela > Painéis → Arquivo > Sair (menu real) → reiniciar com `--session-file` e config vazia (estrutura igual, **0 px** de diferença) → salvar preset "meu" digitando no diálogo → Redefinir → aplicar o preset (**0 px** de diferença).

## 10. Limites e o que NÃO foi validado

* **Janelas flutuantes**: o `egui_tiles` não tem; o plano (`egui::Window` do corpo do painel, ou trocar para `egui_dock` 0.21, que tem) está em `pesquisa/layout_dock_atalhos.md`. Hoje: dividir, agrupar em abas, redimensionar, fechar.
* Sensação de arrastar abas com mouse/tablet reais, HiDPI/escala de fonte nas abas e cabeçalhos, Wayland, macOS (`Cmd`), teclados não-US (o egui entrega a tecla lógica; `[` `]` e `` ` `` mudam de lugar) — **[N]**.
* Ao redimensionar a janela, as colunas escalam proporcionalmente (não ficam em px fixos); a largura mínima de um painel é 110 px (`Behavior::min_size`), sem o teto antigo (`size_range`).
* Painéis cujo corpo tem largura fixa (ex.: campos de 300 px do inspetor da Animação) rolam/cortam se o painel ficar mais estreito que isso.
* Mover o viewport para dentro de um grupo de abas o esconde enquanto outra aba está ativa (comportamento do `egui_tiles`); ele reaparece ao ativar a aba.
* O arquivo de layouts do usuário é global (não por projeto); layouts por projeto/nuvem ficam fora do escopo.
* Atalhos: sem chords, sem botões do mouse, sem `when` arbitrário; o painel não impede conflitos (só avisa).
