# Multi-vista do viewport

Modelagem, Cenário, Posing, Animação e Iluminação (os workspaces com viewport 3D editável) mostram **1, 2 (lado a lado ou empilhadas) ou 4 vistas**,
cada uma com a sua câmera. Render, Shading, Personagem e Biblioteca ficam com uma vista só (o Render é um enquadramento de câmera de render, não um
espaço de trabalho 3D; Personagem e Shading têm viewports de preview).

## Uso

| | |
|---|---|
| layout | 4 ícones na barra do viewport (1 · 2 lado a lado · 2 empilhadas · 4). `Ctrl+Alt+Q` alterna 4 vistas; maximizar/restaurar a vista ativa: `Ctrl+Espaço` (Cenário: `Ctrl+Shift+Espaço`, porque `Ctrl+Espaço` abre a gaveta) |
| tipo da vista | seletor no canto de cada vista: Perspectiva, Ortogonal (livre), Frente, Costas, Direita, Esquerda, Topo, Base (ortogonais alinhadas). Orbitar uma vista alinhada a torna "Ortogonal" livre. `Alt+P` alterna perspectiva/ortogonal (Modelagem) |
| vista ativa | a que está sob o ponteiro (como no Blender); moldura roxa. Só ela recebe entrada; as outras apenas desenham. Não troca durante um arrasto, um modal G/R/S ou um menu aberto; **arrastar da Biblioteca não trava** (soltar numa vista usa a câmera dela) |
| divisórias | arrastáveis (o cruzamento do 2×2 move as duas); duplo clique volta ao meio; nenhuma vista fica menor que 15 % |
| sessão | layout, divisórias, câmeras de todas as vistas e vista ativa são salvos por workspace (`multiviews` em `SessionState`; Posing e Animação dividem as mesmas) |

## Como funciona (`crates/anigo-studio/src/app/multiview.rs`)

* Cada vista é um *slot* (0..4) com a sua `OrbitCamera`. A câmera do slot **ativo** vive onde sempre viveu (`StudioApp::camera`, `scene_ui.camera`,
  `pose_ui.camera`, `light_ui.camera`): todo o código de edição continua usando-a. As dos demais ficam em `MultiView::cams` e são trocadas para
  dentro da câmera viva só enquanto a vista correspondente é desenhada.
* `Workbench` tem um alvo de render por slot (`set_slot`) e o `Gpu` um preview Cel por slot (`ensure_cel`/`cel_slot`), criado só quando a vista pede Cel.
  Em ortogonal o preview Cel usa a mesma altura de volume do Workbench (`celview::core_camera`).
* Com várias vistas a barra de ferramentas do workspace ganha uma faixa própria acima delas (não cobre mais o topo).

## Testes

`cargo test -p anigo-studio --lib multiview` (divisão, divisórias, estado, rodízio da vista ativa), `session` (ida e volta + bloco quebrado nunca derruba a sessão),
`bridge_input`. Em app real, pela bridge (`INPUT` injeta eventos na janela): passar o ponteiro troca a vista ativa; orbitar e dar zoom mudam só a vista sob o
ponteiro em Modelagem/Cenário/Posing/Animação/Iluminação; clique seleciona (objeto no Cenário, osso no Posing) numa vista que não era a ativa; o eixo X do gizmo
move o nó (1 passo de desfazer) e Ctrl+Z o desfaz; caixa de seleção; soltar um ativo da Biblioteca numa vista que não era a ativa coloca o ativo; arrastar
divisória, duplo clique, salvar a sessão, reabrir o app e conferir layout/divisórias/tipos.

## Limites conhecidos

* Cada vista com Cel renderiza o preview completo (4 renderizações em 2×2): em cenas muito pesadas prefira Solid nas vistas secundárias.
* O preview Cel das vistas 1..3 não recebe o mapa SDF facial do workspace Shading (que só tem uma vista).
* Não há medida de desempenho do 2×2 fora da máquina de desenvolvimento (GTX 1650, Vulkan).
