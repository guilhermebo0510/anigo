# Ferramenta de cabelo (estilo VRoid) — Modo Cabelo da Modelagem

Ferramenta de modelagem de cabelo por **mechas editáveis**: você desenha guias sobre a cabeça, o app gera a malha
(cards em fita ou mechas com seção) como objetos da cena e mantém as mechas editáveis. Referência de UX: o Hair Editor do
VRoid (grupos → mechas, freehand, retoque, Flip, Hair Bounce). Pesquisa de origem: `pesquisa/personagem_rig_cabelo_roupa.md`,
seção D e F4.

* Motor (Rust puro, sem GPU, testável em CI): `crates/anigo-modeling/src/hair/` (`mod.rs`, `surface.rs`, `build.rs`, `comb.rs`, `chain.rs`).
* Ferramenta no studio: `crates/anigo-studio/src/modeling/hair.rs` (estado + ações com undo + regeneração), `app/hair_ui.rs`
  (entrada, painel, overlay), `i18n_hair.rs` (pt/en).
* Capturas de referência: `img/modelagem_cabelo_desenho.png`, `img/modelagem_cabelo_grupos.png`, `img/modelagem_cabelo_cel.png`.

## 1. Como usar

1. No modo **Objeto**, selecione a cabeça (qualquer malha; a esfera serve de teste) e escolha **Cabelo** no seletor de modo do
   viewport. O objeto ativo vira a **cabeça-alvo**. Com um penteado já existente, o alvo dele é mantido. "Usar o objeto ativo como
   cabeça" (painel) troca o alvo.
2. Ferramenta **Desenhar mecha** (`D`): arraste sobre a cabeça. O traço é projetado na superfície (raycast com BVH), reamostrado,
   suavizado, simplificado em poucos pontos de controle e **elevado** da superfície (raiz colada ao couro cabeludo, ponta mais
   afastada). A primeira amostra tem de acertar a cabeça; se o traço sair da silhueta, ele continua num plano paralelo à tela à
   profundidade do último acerto (o cabelo "cai" para fora da cabeça). **A direção do traço é raiz → ponta.**
3. Ferramenta **Editar** (`V`): clique numa mecha para selecioná-la; arraste os pontos de controle (a raiz fica recolada ao couro
   cabeludo); **Ctrl+clique** na mecha insere um ponto; `X`/`Del` remove o ponto selecionado (ou a mecha); `Shift+X` remove a mecha.
4. Ferramenta **Pentear** (`C`): arraste; os segmentos sob o pincel giram na direção do movimento (comprimento preservado, raiz
   fixa). `[` `]` mudam o raio. Em grupos com simetria, o par espelhado acompanha.
5. **Grupos** (painel): criar, renomear, duplicar, excluir, ocultar, **Espelhar** (cria um grupo novo espelhado em X — o "Flip"
   do VRoid), mover a mecha selecionada para outro grupo, **Simetria X** (`M`). Cada grupo é UM objeto da cena
   (`Cabelo - <nome>`) com material próprio (cor base, sombra, brilho, AO de raiz, viés de normais, orientação de UV).
6. **Parâmetros da mecha** (painel): seção (fita plana, fita curva, triangular, tubular), largura, espessura, torção, curvatura,
   afinar ponta (+ expoente), bojo, resolução, clump (mecha composta). Com uma mecha selecionada o painel edita ela (e o par
   espelhado); sem seleção edita o **modelo das mechas novas**. "Aplicar ao grupo" copia para todas as mechas do grupo.
7. **Física (bounce)**: "Auto-gerar grupos de bounce" (1–30 grupos por proximidade das raízes, ossos 1–16), ajuste de ponto fixo,
   rigidez, arrasto, gravidade e raio de colisão por grupo, e "Exportar cadeias (JSON)".
8. **Saída**: "Exportar grupos para a Biblioteca (GLB)" (um ativo por grupo, fluxo existente da Biblioteca). A malha também pode
   ser exportada pelo botão normal do modo Objeto (objeto ativo).

Undo/redo (`Ctrl+Z`, `Ctrl+Shift+Z`, botões da barra superior): **um checkpoint por traço, por arrasto de ponto, por passada do
pente e por ação de painel**; desfazer restaura mechas E objetos gerados juntos (o `Document` inteiro está no snapshot).
Sliders contínuos gravam um checkpoint por gesto.

## 2. Atalhos (com o ponteiro no viewport, modo Cabelo)

| Atalho | Ação |
|---|---|
| `D` / `V` / `C` | ferramenta Desenhar / Editar / Pentear |
| arrastar (Desenhar) | traço da mecha (`Esc` cancela) |
| clique / arrastar (Editar) | seleciona mecha / move ponto de controle |
| `Ctrl`+clique na mecha | insere ponto de controle |
| `X`, `Del` | remove o ponto selecionado (ou a mecha se não há ponto) |
| `Shift+X` | remove a mecha |
| `Shift+D` | duplica a mecha selecionada (e o par espelhado) |
| `M` | liga/desliga a simetria X do grupo ativo (ligar cria o par das mechas existentes; desligar desfaz os vínculos) |
| `Shift+M` | Espelhar: cria um grupo novo com as mechas espelhadas |
| `[` `]` | raio do pente |
| `Ctrl+Z`, `Ctrl+Shift+Z` (`Ctrl+Y`) | desfazer / refazer |
| botão do meio, `Alt`+arrastar | orbitar (`Shift` pan, `Ctrl` zoom); roda: zoom |
| `.` / `Home` | enquadrar; `Alt+Z` raio-X |

## 3. Modelo de dados (`anigo_modeling::hair`)

Tudo serializável (`serde`, JSON determinístico), validado (`validate()`, `from_json` recusa dado inválido) e no **espaço local
da cabeça-alvo**; o plano de simetria é `x = 0` desse espaço. Os grupos de cabelo ficam em `Document::hair` (a sessão de
Modelagem); sessões antigas sem o campo carregam normalmente, e um penteado corrompido é descartado sem perder a cena.

```
HairSet { version, groups: Vec<HairGroup>, bounce: Vec<BounceGroup>, scalp_object, scalp_center, root_node_hint }
HairGroup { id, name, strands: Vec<HairStrand> /*mechas*/, material_slot, material: HairMaterial, symmetry,
            visible, object_id, atlas, atlas_cycle, uv_layout }
HairStrand { id, guide: Curve /*curve.rs: CatmullRom raiz->ponta, CurvePoint{pos,tilt,radius}*/, params: StrandParams,
             root_normal, mirror_of }
StrandParams { section: HairSection{kind: Flat|Arc|Triangle|Tube, width, thickness, twist, curl},
               tip_taper, taper_power, resolution, flare, clump: Option<HairClump> }
```

Convergência com `curve.rs`: a guia é uma `Curve`; Flat/Arc usam `hair_card` (taper, curl, flare, normais viesadas); Triangle/Tube
são uma varredura própria sobre os mesmos frames de Bishop (`frames_uniform`); torção entra como `tilt` por ponto; clump usa
`hair_clump`.

**Geração de malha** (`hair::build`): a malha NÃO é guardada, é regenerada a cada edição.
* Normais por corner viesadas para fora da cabeça (`material.normal_bias`, 0 = normal do card, 1 = esfera de sombreamento).
* UV: V = fração do comprimento (0 raiz → 1 ponta), U através; `UvLayout::LengthU` troca os eixos (a tangente derivada da UV passa a
  ser o fio); `atlas` (+ `atlas_cycle`) remapeia para uma célula de atlas.
* Cor de vértice (canais NPR): R (AO) escurece na raiz (`root_ao`) e sobe até 1 ao longo do comprimento.
* `HairMesh` devolve também, por vértice, `vert_strand`, `vert_t` (fração do comprimento) e `vert_dir` (direção do fio): insumo
  para pesos de skin (2 ossos vizinhos por `t`) e brilho anisotrópico — **ainda não consumidos** (ver §5).
* Ponta: `tip_taper = 0` termina num único vértice (cards em triângulo; sólidos em bico, fechados e 2-manifold).

**Operações** (todas atômicas: em erro nada muda): `add_strand` (com par espelhado), `remove_strand`, `duplicate_strand`,
`move_strand` (entre grupos), `set_point_pos`/`insert_point`/`remove_point`/`set_params` (propagam ao par), `set_symmetry`,
`flip_group`, `duplicate_group`, `simplify` (RDP), `taper_tip`, `comb`, `auto_bounce`, `generate_chains`.

**Saída física neutra** (`hair::chain::HairChainSpec`, sem depender de nenhum crate de física):

```
HairChainSpec { name, root_node_hint /*"head" por padrão*/, points: Vec<Vec3> /*bones+1 pontos, âncora -> ponta, espaço local da cabeça*/,
                radius /*hit radius*/, stiffness, drag, gravity, fixed_point, bounce_group, source: StrandRef }
```
`points[0]` é o ponto fixo (a fração `fixed_point` do comprimento; antes dele o cabelo é rígido); a direção segue o traço. As cadeias
são DERIVADAS das guias a cada chamada de `HairSet::generate_chains()` — editar o penteado depois atualiza as cadeias (no VRoid
isso não acontece). Faixas validadas: ossos 1–16, `fixed_point` 0,01–0,99, rigidez/arrasto/gravidade 0–1, raio 0–0,1.
Conversão para `anigo-physics`/`VRMC_springBone` fica para quem consumir o tipo.

## 4. Cenários de captura e testes

* Cenários: `--scenario hair-draw` (desenho com simetria, guias e pontos), `hair-groups` (3 grupos coloridos, ferramenta Editar,
  painel de grupos), `hair-cel` (o cabelo no preview **Cel** sobre a cabeça de teste, materiais por objeto) e `hair-ready` (cabeça
  sem mechas, usado pelo smoke). Eles rodam o roteiro **pelo mesmo caminho do mouse** (pontos locais → pixels → raios → traço).
  Estão em `scripts/native_screenshots.sh` e no job `native` do CI.
* Smoke de entrada real: `scripts/native_hair_smoke.sh [binário]` — Xvfb próprio (`:96`, recusa um display já existente),
  `xdotool` com `windowfocus`, `mousedown`/`mousemove`/`mouseup` para desenhar uma mecha, `keydown`/`key`/`keyup` para
  `Ctrl+Z` e `Ctrl+Shift+Z`, e comparação de screenshots (a mecha aparece, desfaz volta pixel a pixel, refaz traz de volta).
  Nunca toca o mouse/teclado do usuário.
* Testes: `cargo test -p anigo-modeling hair::` (31) e `cargo test -p anigo-studio hair` (12): involução do espelho e do Flip,
  simetria ligada ao par, raiz sobre a superfície e elevação monotônica, ponta degenerada com `taper = 0`, sólidos fechados,
  normais para fora, UV/atlas/material gravados, pentear (raiz e comprimento preservados, fora do raio intocado, par simétrico),
  auto-bounce (cada mecha em exatamente um grupo, determinístico), cadeias (ossos/ponto fixo/serde), undo/redo restaurando mechas
  e objetos, um checkpoint por arrasto/passada, raiz recolada na superfície, sessão antiga/corrompida, exportação GLB.

## 5. Limites conhecidos e o que NÃO foi validado

**Limites do código**
* O cabelo segue o formato de "cards/mechas", sem fios individuais; mechas se atravessam e atravessam a cabeça se o usuário as
  puxar para dentro (não há colisão com a cabeça na edição nem no pente).
* Traço projetado: só a primeira amostra precisa acertar; fora da silhueta o plano é paralelo à tela (não "cai" por gravidade). Não
  há pincel de retoque que refaça um grupo seguindo um traço novo, nem grade-guia sobre a cabeça, nem modo procedural (contagem/seção
  por grupo além do "Aplicar ao grupo" e do clump), nem "Convert to Freehand".
* Pressão da caneta não afeta largura (só `Event::Touch.force` seria lido; não está ligado ao traço de cabelo).
* O alvo é a malha-base (sem modificadores); mover/escalar a cabeça só é seguido quando as mechas são regeneradas (cada edição ou
  "Regenerar malhas"); os objetos de cabelo copiam a transformação da cabeça no momento da regeneração.
* Material: o preview Solid usa a cor base como `tint`; o preview Cel recebe base/sombra/brilho por objeto (`ModelObject::material`).
  **O GLB não leva o material** (a malha do core não tem slots) nem `vert_dir`/`vert_t`: tangente do fio e pesos de skin ainda não
  saem da ferramenta; o "angel ring" real depende do renderer cel consumir a tangente do fio.
* Cadeias de física: só a estrutura (`HairChainSpec`); sem pesos de skin dos vértices, sem colisores, sem simulação ao vivo, sem
  export `VRMC_springBone`. O JSON das cadeias é gravado em `<dados do studio>/Hair/chains.json`.
* Regenerar a malha a cada quadro durante o arrasto de ponto reconstrói o grupo inteiro e recalcula o hash da malha-alvo para o
  BVH; testado com cabeças de ~2 k faces. Cabeças de dezenas de milhares de faces e grupos com centenas de mechas não foram medidos.
* Seleção após desfazer/refazer não é restaurada (a mecha some da seleção; a geometria volta).

**Não validado (precisa de olho/mão no chat local)**
* Sensação do desenho com mouse e caneta (espaçamento, suavização, simplificação RDP, elevação padrão), do arrasto de pontos
  (plano da tela) e do pente (raio/força em pixels, direção pelo movimento). Os padrões (`StrokeParams`, comb 70 px/0,35) foram
  ajustados em capturas sobre uma esfera de raio 1, não numa cabeça real.
* Gosto visual do cabelo: largura/afinamento padrão, normais viesadas, AO de raiz, cores do material, aparência no preview Cel
  (hoje chapada, sem tangente de fio). Comparação com o VRoid real (workflow, atalhos, grade-guia, resultado) **não foi feita**.
* Cabeças reais (não esferas): raycast em malha com orelhas/rosto, cabeça com modificadores, escala ≠ 1.
* Windows/macOS, GPU real, e `Alt+arrastar` com WM real (o smoke roda sem gerenciador de janelas).
