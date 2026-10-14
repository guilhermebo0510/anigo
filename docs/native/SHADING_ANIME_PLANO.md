# Shading anime — plano de um sistema "acima do mercado"

Estado: **plano + ferramentas de calibração prontas**. Nada do sistema novo foi implementado no shader; o shader de produção segue o
contrato congelado. Este documento fixa o rumo para entrarmos na área quando você tiver os quadros de referência.

Decisão de partida (sua): **manter o que temos** (material cel com rampa, 2ª sombra, rim, realce, contorno, SDF facial, presets) e
construir **por cima**, com uma forma mais profissional — não trocar tudo por um grafo de nós genérico.

## 1. Onde estamos (verificado no código)

| Já existe | Onde |
|---|---|
| Material cel: base/sombra, limiar e suavidade, degraus (`toon_steps`), rampa 2D editável, 2ª sombra (MToon), matiz da sombra, AO | `anigo-core::StylizedMaterial`, `cel_shading.wgsl` |
| Realce (tamanho, suavidade, deslocamento, cor), matcap, emissão, rim (cor, abertura) | idem |
| Contorno em passe próprio (largura, cor, opacidade, suavidade, viés de profundidade) | pipeline `outline` |
| Atributos de vértice que "dobram" a sombra à mão (`anime_attr`: deslocamento do terminador, máscara de rim/realce) | `anime_attr.g/a` |
| SDF facial (sombra do rosto desenhada, não calculada) | `face_sdf.wgsl` |
| Luzes extras (ponto, holofote, painel, direcional) que **tingem** em vez de estourar | `cel_shading.wgsl` §9b |
| AOVs: profundidade, normal, máscara, id de objeto, contorno | Render |
| Controle total pela bridge/MCP (`SHADING …`, `anigo_native_material*`) e histórico próprio | `bridge_shading*.rs` |

O que a concorrência tem de parecido: *toon shaders* de Blender/Unity/Unreal (rampas + contorno), MToon/lilToon, Clip Studio 3D.
Todos exigem **ajuste manual olhando** e entregam um "3D com cara de cel". É aí que podemos nos separar.

## 2. Os cinco pilares (o diferencial)

1. **Calibração por referência (o que ninguém tem).** Você arrasta um quadro de anime; o app **mede** o look (paleta, nº de faixas, dureza
   da sombra, espessura da linha, realces, rim, deslocamento de matiz da sombra) e **propõe os parâmetros**; depois um otimizador ajusta o
   material até o render ficar numericamente perto do quadro, mostrando antes/depois e a diferença. As medidas já existem como ferramenta
   (`scripts/look_calibration`, §4) — o produto é transformar isso num botão "Ajustar à referência".
2. **Sombra "desenhada", não calculada.** Hoje o terminador vem de `N·L`. No anime ele é uma decisão do artista: formas de sombra
   limpas, que não "fervem" quando a luz mexe. Plano: pintar a *intenção* da sombra na malha (já temos o atributo e o SDF facial),
   somar edição de normais (já temos), e **estabilizar no tempo** (a forma só muda quando a luz muda de verdade).
3. **Sombreamento "em dois/três" (on twos).** A animação já tem `hold_step` (poses seguradas). Plano: o sombreamento e a linha **seguem o
   mesmo ritmo** da pose — a sombra só atualiza nos quadros-chave, como no desenho à mão. É o que faz um 3D deixar de parecer 3D.
4. **Camadas de cel como saída.** Em vez de AOVs genéricos, exportar o que o pós-produtor de anime espera: **base / 1ª sombra / 2ª sombra /
   realce / rim / linha** como camadas separadas (PNG/PSD/EXR) para recolorir e compor em After Effects/Clip Studio.
5. **Look em camadas com grafo por baixo.** A interface continua sendo uma **pilha de camadas de look** (cada uma com tipo, máscara e
   parâmetros), com presets e arrastar para reordenar. Por baixo, a pilha é um **grafo tipado** (entradas: `N·L`, visão, curvatura, AO,
   posição, UV, atributos de vértice; operações: rampa, limiar, posterizar, ruído, curva; saídas: tinta de base, sombra, realce, rim,
   linha) que compila para WGSL com cache por hash. Quem quiser abre a pilha **como grafo** (o editor de nós vem depois, sem refazer nada).
   Extra: **Look Keys** (animar o look no tempo: hora do dia, clima) e **ligação de luz** (luz só de sombra, luz sem especular).

## 3. Arquitetura proposta (sem tocar no contrato atual)

* **Dados:** campo opcional e versionado `StylizedMaterial.look: Option<LookStack>` (`#[serde(default)]`, `None` = comportamento atual).
  Exemplo do formato (rascunho):

  ```json
  { "version": 1, "layers": [
      { "kind": "base_tint",   "source": "main_texture" },
      { "kind": "shadow",      "bands": 2, "threshold": 0.5, "softness": 0.02, "tint": [0.78, 0.63, 0.73], "hue_shift_deg": -12 },
      { "kind": "highlight",   "style": "hard", "size": 0.15, "mask": "vertex.a" },
      { "kind": "rim",         "width": 0.4, "color": [0.7, 0.8, 1.0] },
      { "kind": "line",        "width_px": 2.0, "by_distance": true, "by_curvature": 0.3 } ],
    "timing": { "shade_on": "animation_hold" } }
  ```
* **Render:** com `look == None` o caminho de hoje roda **idêntico** (quadros congelados intactos). Com `look` presente, um segundo
  caminho de fragment (gerado do grafo) é selecionado por pipeline, com o próprio hash no contrato. Nada de bifurcar o shader existente.
* **Tempo:** o estado de sombra "segurado" vive no renderer (um buffer por objeto, atualizado só nos quadros-chave de `hold_step`).
* **Interface:** uma aba **Look** no Shading (pilha de camadas) ao lado das atuais; as abas Base/Realce/Contorno continuam como
  atalhos para as camadas equivalentes. Painel responsivo (regras de largura de `PROPRIEDADES_ABAS.md`).
* **Bridge/MCP:** `SHADING look` (list/add/set/move/remove/preset, validação estrita, 1 passo de histórico por edição), no mesmo
  padrão das demais. Nomes reservados: `anigo_native_material_look`, `anigo_native_material_fit` (ajustar à referência).

## 4. O que já está pronto agora (o "deixar preparado")

* `scripts/look_calibration/` — **funciona hoje**:
  * `analyze_frame.py`: mede um quadro (paleta, faixas, dureza da sombra, linha, realces, rim, deslocamento da sombra) e gera relatório;
  * `compare.py`: compara referência × render do ANIGO, linha a linha, com **dica de qual parâmetro mexer** e nota 0–100;
  * `capture_app.py`: captura o viewport ou faz um **render real** (até 4K) pela bridge para comparar;
  * `test_look_calibration.py`: 6 testes com esferas cel sintéticas de parâmetros conhecidos (sombra dura × suave, linha de 4 px, 2 e 3 faixas).
* `referencias/anime/` — pasta e convenção para os quadros (ver o `README.md` de lá). **Os quadros não vão para o git** (direitos autorais);
  só o manifesto de cada cena (título, plano, notas de luz/câmera) pode ser versionado.
* Luzes extras com tingimento limitado e `SCENE light` (a base para "ligação de luz").

## 5. Fases (cada uma termina com algo verificável)

| Fase | Entrega | Critério de aceite |
|---|---|---|
| 0 — Calibrar (**pronta a usar**) | Quadros de referência + ferramentas acima | Para 5 cenas suas, nota de look e dicas por parâmetro; lista de lacunas do shader atual |
| 1 — Ajustar à referência | Botão "Ajustar" no Shading: otimizador (busca por coordenada/CMA-ES) sobre os parâmetros atuais | Nota sobe ≥ 15 pontos em 4 de 5 cenas, sem intervenção manual, em < 60 s |
| 2 — Pilha de look + grafo | `look` no material, compilação para WGSL, aba Look, bridge/MCP | Quadros congelados idênticos com `look = None`; pilha reproduz o material atual byte a byte |
| 3 — Sombra desenhada e estável | Pintura da intenção de sombra, estabilização temporal | Sem "fervura" numa rotação de luz de 360° (diferença entre quadros vizinhos medida) |
| 4 — On twos + camadas de cel | Sombra/linha no ritmo do `hold_step`; export base/sombra/realce/rim/linha | Imagem composta das camadas == render final (diferença 0) |
| 5 — Look Keys e ligação de luz | Animar o look; luz só de sombra | Teste de timeline e de luz ligada por bridge |

## 6. Riscos e decisões que são suas

* **Direitos dos quadros:** usaremos referências **só como medida local** (nada é copiado para o produto nem para o git).
* **"Ajustar à referência" não é mágica:** otimiza parâmetros que já existem; formas de sombra desenhadas à mão (fase 3) são outro problema.
* **Contrato de render:** a fase 2 acrescenta um caminho novo; o hash e a lista de bindings entram no contrato como fizemos com as luzes extras.
* Para decidir quando você mandar os quadros: **quais cenas** (personagem em plano médio/close/geral?), **qual estilo** (TV moderna, filme,
  anos 90?) e **se a linha deve variar de espessura** (pincel) ou ser constante.
