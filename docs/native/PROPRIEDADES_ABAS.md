# Painel Propriedades com abas verticais

Os painéis de propriedades têm uma **faixa de ícones vertical encostada na borda esquerda** (como o editor de Propriedades do Blender), entre a viewport e o
conteúdo. Cada ícone abre uma categoria; o painel deixa de ser uma lista longa de seções empilhadas. Tooltip com o nome, aba lembrada por painel/modo.

| Workspace (painel) | Abas |
|---|---|
| Modelagem · Objeto | Objeto · Modificadores · Visualização · Histórico |
| Modelagem · Edição | Ferramenta · Modificadores · Dados da malha · Visualização · Histórico |
| Modelagem · Escultura | Pincel · Simetria e máscara · Malha · Histórico · Atalhos |
| Modelagem · Cabelo | Cabelo · Desenho e pente · Física · Saída · Histórico · Atalhos |
| Personagem | Ferramenta (corpo/rosto, conforme a barra da esquerda) · Projeto (salvar/abrir/resetar) · Histórico |
| Posing | Ferramenta (estrutura óssea / IK / poses) · Histórico |
| Iluminação | Rigs de luz · Sol e ambiente · Sombras e rim · Avançado |
| Animação | Clipe · Osso (substitui a antiga linha de abas horizontal) |
| Shading | Base e sombra · Brilho · Rim e contorno · Rosto e olhos · Texturas |
| Cenário · Detalhes | Transformação · Ativo (só ativo da Biblioteca) · Lente (só câmera de cena) · Material · Organização · Cena (configurações da cena; única aba sem seleção). As abas mostradas dependem da seleção |

Ficam de fora, de propósito: **Render** (as abas Câmera/Passes/Saída já são as ferramentas da barra da esquerda), o **Outliner e o Histórico do Cenário** (listas, não
propriedades) e a **Biblioteca** (barra lateral, navegador e detalhes).

## Comportamento

* Pedir foco numa seção abre a aba onde ela mora: no Shading, clicar no rim/contorno do preview ou os cenários que focam um grupo; na Iluminação, clicar nas
  partes do gizmo/overlay que apontam uma seção (sol, sombras, ambiente, extras).
* O primeiro grupo de cada aba do Shading abre por padrão (Base, Especular, Rim, Rosto).
* Largura: a faixa ocupa ~40 px do painel. **Nenhum painel do dock fica abaixo de 260 pt** (`theme::PANEL_MIN_W`) e, nessa largura, nada é cortado:
  * `lab(rótulo, slider)`: painel largo = como sempre (slider, valor e rótulo à direita); estreito (< 280 pt de espaço útil) = rótulo em cima e slider na largura toda;
  * `field_row` / `xyz_row`: rótulo + controle (ou X/Y/Z) lado a lado quando cabe, empilhados quando não;
  * `header(ui, título)`: título de seção truncado com "…" à largura (o egui o mede sem limite); o estado aberto/fechado não reseta ao redimensionar;
  * `combo(ui, id)`: largura limitada ao espaço, texto escolhido truncado;
  * linhas de botões/campos dos painéis são `horizontal_wrapped` (o que não cabe vai para a linha de baixo);
  * regra de ouro: um widget que estoura **também alarga o espaço das linhas seguintes** no mesmo bloco (o egui expande o `max_rect`), então o primeiro
    estouro de um bloco faz os de baixo parecerem "sem quebra". Corrija o primeiro.
* **Medição automática**: `props_scroll` mede quanto o conteúdo passa da largura e `GET_UI_STATE` devolve `props_overflow` (0 = coube) e `panels[].rect`;
  `UI_ACTION expand_sections` abre todas as seções e `debug_expand` pinta de vermelho o widget que alarga o painel. A varredura (9 workspaces, todas as abas,
  painel no mínimo, PT e EN) está em `docs/native/MCP_BRIDGE_NATIVA.md` (seção "Luzes extras e largura dos painéis") e termina com 0–1 abas acima de 5 pt.

## Código (`crates/anigo-studio/src/app/props_tabs.rs`)

`PropTab` (a categoria: chave, ícone, rótulo i18n `ptab.*`), `TabSet` (qual painel e quais abas) e `StudioApp::tabbed(ui, set)`, que desenha a faixa, grava a aba
escolhida e devolve `(aba, Ui do conteúdo)`. Cada painel só decide o que mostrar em cada aba. Estado de apresentação (não vai para a sessão).

## Bridge / MCP

`UI_ACTION select_props_tab` (ferramenta `anigo_native_ui_action`) escolhe a aba do workspace atual; `GET_UI_STATE.props_tab` / `props_tabs` dizem a atual e as
possíveis. Workspace sem abas responde `-32602`.

## Não validado

* Os scripts `xdotool` do Shading (`scripts/native_shading_smoke.sh`) só rodam em Linux; o equivalente multiplataforma, com entrada injetada pela bridge, é
  `scripts/bridge_shading_smoke.py`. As capturas de referência (`docs/native/img`) não foram regeneradas.
* Teclado: a faixa só responde a mouse (sem atalho para trocar de aba).
