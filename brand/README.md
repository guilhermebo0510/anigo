# Identidade visual do ANIGO

Tudo nesta pasta sai de **um único gerador** (`gen_brand.py`, geometria vetorial própria com `shapely`): os SVG são os caminhos
exatos e os PNG/ICO são rasterizações deles. Não há fonte tipográfica nem imagem de terceiros — a marca e o nome são 100 % do projeto.

```text
python brand/gen_brand.py      # regrava svg/, png/, ico/, splash/ e as cópias de crates/anigo-studio/assets/brand e src-tauri/icons
```

## Conceito

* **A marca** é um **A sem travessa (Λ)** com uma **esfera** no vão — a bola de shading, o objeto que todo artista de cel usa para
  julgar uma luz. O Λ é **cortado ao meio** no eixo, como o terminador de um cel shading: um lado claro e um lado de sombra, em duas
  cores chapadas e sem degradê. A esfera recebe a mesma luz (lua crescente de sombra no mesmo sentido).
* **O nome** "ANIGO" é desenhado em monolinha geométrica (cortes retos, sem serifa), com o mesmo peso e os mesmos ângulos do Λ.
* Lê-se como **letra**, como **personagem** (corpo + cabeça) e como **ferramenta de luz** — e continua legível a 16 px.

## Cores

| Papel | Escuro (fundo escuro) | Claro (fundo claro) |
|---|---|---|
| Lado iluminado do Λ | `#b9a8ff` | `#8a6cff` |
| Lado de sombra do Λ | `#6a50e0` | `#4a34b5` |
| Esfera — luz / sombra | `#ffd2a1` / `#f5976a` | `#ff9a2e` / `#d9601f` |
| Nome (tinta) | `#f2f0fb` | `#1c1633` |
| Ícone do app (degradê) | `#2b2158` → `#120e24` | `#ffffff` → `#e9e4ff` |
| Fundo de apresentação | `#0f0c1b` | `#f6f4ff` |

O violeta da interface (`theme::ACCENT = #8a6cff`) é o mesmo do conjunto claro; o laranja da esfera é o laranja de seleção do app
(`theme::SELECT`).

## Construção e uso

* Grade de 1000 unidades: Λ de (150, 880) a (850, 120); esfera de raio 74 centrada em (500, 745); corte de luz no eixo x = 500.
* **Área de respiro:** meia altura da esfera (≈ 0,07 da altura da marca) em todos os lados; no lockup, a largura do Λ separa a
  marca do nome. **Tamanho mínimo:** marca 16 px (versão "gorda" automática ≤ 32 px), lockup horizontal 20 px de altura.
* **Não** girar, esticar, trocar as cores das metades, aplicar sombra/brilho, nem colocar o conjunto escuro sobre fundo claro (use o
  conjunto claro ou as versões `mono`).

## Arquivos

| Pasta | Conteúdo |
|---|---|
| `svg/` | `mark_{dark,light}`, `wordmark_{dark,light}`, `lockup_{dark,light}`, `mark_mono_{white,black}`, `app_icon_dark` |
| `png/` | `app_icon_{dark,light}_{16…1024}` (16, 20, 24, 32, 40, 48, 64, 72, 96, 128, 150, 192, 256, 310, 512, 1024), `mark_*`, `lockup_*` (alturas 48/96/192/384), `lockup_stacked_*`, `mark_mono_*`, `lockup_mono_*` |
| `ico/` | `anigo.ico` e `anigo_light.ico` (16, 24, 32, 48, 64, 128, 256; PNG embutido) |
| `splash/` | `splash_{dark,light}_{1920x1080,960x540}.png` (só a arte; versão e progresso são desenhados pelo app) |
| `contact_sheet.png` | folha de contato para revisão rápida |

## Onde o app usa

* **Ícone da janela / barra de tarefas / Alt+Tab:** `brand::window_icon()` (PNG 256).
* **Ícone do arquivo `.exe`, atalho da área de trabalho e barra de tarefas fixada:** `build.rs` embute `assets/brand/anigo.ico` com
  `rc.exe` (Windows SDK). Sem o SDK o build continua, só sem o ícone do arquivo.
* **Barra superior** (lockup) e **Ajuda ▸ Sobre**; **splash** de abertura dentro da janela (`brand::Splash`: some sozinha em ~1,7 s ou
  ao clique/tecla; `ANIGO_SPLASH=0` desliga, `=hold` espera o clique, `=1` força mesmo com bridge/captura).
* **Tauri legado:** `src-tauri/icons/*` regravados com a mesma arte.
* **Modo claro (futuro):** `brand::Variant::of(ctx)` já escolhe o conjunto claro quando o tema do egui for claro.
