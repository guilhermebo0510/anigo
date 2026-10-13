# Biblioteca de ativos — UI nativa (`anigo-studio`)

**Estado:** UI implementada sobre o backend `anigo-assets` ([`BIBLIOTECA_BACKEND.md`](BIBLIOTECA_BACKEND.md)) e validada no sandbox
(Xvfb + Vulkan por software/lavapipe, sem GPU real). Cobre T-5.7 e T-5.8 (UI), a parte GPU de T-5.6 e a parte de UI de T-5.10.
**Nada foi validado com GPU real, com arquivos de terceiros (VRoid/Blender/Sketchfab/Quixel) nem fora do Linux** (ver seção 11).

Legenda: **[T]** coberto por teste automático que rodou; **[S]** visto em screenshot/entrada real no Xvfb (lavapipe); **[N]** não validado.

## 1. O que existe

Workspace `Biblioteca` reescrito (`crates/anigo-studio/src/library_ui/`), sem mockups: tudo vem de `anigo_assets::Library`.

| Área | O que faz |
|---|---|
| **Painel esquerdo** | Biblioteca (todos, favoritos, lixeira, ausentes, duplicados — cada um com contagem), **locais** (gerenciada/ligadas; adicionar por caminho digitado ou pelo navegador de pastas "Adicionar pasta…"; remover com confirmação; rescan; trocar o caminho de um disco remontado), **tags hierárquicas** com contagem (o pai conta os descendentes; ancestrais ausentes são sintetizados), **coleções** manuais e inteligentes (consulta salva). As linhas são **alvos de soltar**: arrastar cards para uma coleção, uma tag, favoritos ou lixeira. |
| **Centro** | Grade e lista **virtualizadas**; slider de tamanho dos cards; ordenação (nome, importado, modificado, tamanho, nota, triângulos, relevância) com inversão; busca FTS com os operadores do backend (`"frase"`, `-excluir`, `tag:`/`name:`/`author:`/`desc:`, prefixo, sem acento); filtro por tipo (multi), por nota mínima e por tag/coleção/local pela árvore; chips dos filtros ativos; "salvar busca como coleção"; multi-seleção Ctrl/Shift; favorito (coração) e nota (5 estrelas) inline; menu de contexto; tooltip com os números reais. |
| **Miniaturas** | Pedidas **só para o que está visível** (± uma tela); texturas egui em LRU por bytes (96 MiB); ícone colorido do tipo enquanto carrega ou quando não há miniatura. 3D real com o renderer cel (seção 4). |
| **Detalhes** | Metadados reais (tamanho, triângulos, vértices, malhas, materiais, texturas, ossos, morphs, animações, caixa, armazenamento, origem, hash BLAKE3, revisão e histórico, datas); edição de **nome, tags, nota, descrição, autor, licença, URL**; dependências e "usado por" (clicáveis, remover dependência); coleções do ativo; **materiais toon canônicos** (`ToonMaterial`: cor-base, sombra 1/2, limiar/suavidade/degraus, especular, rim, contorno, emissão, normal, rampa, ILM/SDF); avisos do import; **preview 3D girável**; ações: Editar na Modelagem, Duplicar, Mover para a lixeira, Restaurar, Excluir definitivamente (confirmação), Relink (ausente). Multi-seleção: tag em lote, coleção, favoritar, lixeira/restaurar/excluir. |
| **Importar** | Botão + janela (caminhos digitados, "Procurar…", arquivos e pastas, opções: subpastas, duplicados, tipo, tags, autor, licença, **análise prévia** de um arquivo) **e arrastar-e-soltar do SO** em qualquer ponto da janela (`raw.dropped_files`). Fila de jobs, barra de progresso, **Cancelar**, **relatório por arquivo** (importado/nova revisão/duplicado/ignorado/**erro com a mensagem do backend**), copiar erros. |
| **Gestão** | Uso de disco (biblioteca, ativos, lixeira, índice, cache), RAM/GPU das miniaturas, **limpar cache de miniaturas**, remover versões antigas, **verificar integridade** (`verify`, com/sem re-hash, com/sem correções seguras, lista de problemas), **reconstruir índice**, esvaziar lixeira, **duplicados** (grupos por hash, escolher qual manter, resolver um ou todos — une tags/nota/coleções e manda o resto para a lixeira), **relink** por arquivo, por pasta (busca por conteúdo), da raiz inteira e **em lote**. |

## 2. Arquitetura

```text
 egui (thread principal)                           threads de fundo
 ─────────────────────────                         ─────────────────────────────────────────────
 LibraryWorkspace::ui ──set_query/request_page──►  anigo-lib-query   (Pager keyset, cursores por pagina)
   grid.rs (show_rows)  ◄──── QResp (gen) ───────
   side.rs  details.rs  ──command/facets/detail─►  anigo-lib-misc    (mutacoes, facetas, detalhes)
   dialogs.rs           ◄──── MResp ─────────────
   jobs.rs              ──────────────────────►   Job de import/scan/verify/rebuild (anigo-assets)
   thumb_service.rs     ──set_wanted(visiveis)─►  2x loader (ThumbnailCache::load) + 1x render (ensure + GPU)
   preview.rs           ──Req(yaw,pitch,zoom)──►  anigo-lib-preview  (HeadlessRenderer proprio)
```

* **A UI nunca espera o SQLite.** Toda consulta/mutação roda em thread; as respostas carregam a *geração* da consulta e as
  velhas são descartadas [T `workers_deliver_pages_and_drop_stale_generations`]. `Library` é `Clone + Send + Sync` (leitoras em
  paralelo no WAL).
* **Paginação keyset.** `Pager` guarda o cursor de cada página (200 itens); pedir a página N caminha a partir do maior cursor
  conhecido e devolve as intermediárias; nunca carrega a lista inteira [T `pager_walks_keyset_pages_and_reports_the_end`].
  Shift+clique e Ctrl+A pedem **só os ids** da faixa à thread de consulta [T `click_ctrl_shift_and_select_all`, 6 020 ids no smoke].
* **Memória constante.** `PagedList::evict_far` mantém ≤ 48 páginas (9 600 linhas) em RAM; a lista de 50 000 não cresce o resto
  [T `jumping_far_loads_only_nearby_pages`, `grid_frame_time_*`].
* **Atualização otimista** de favorito/nota (a grade muda na hora; o disco confirma) [T `favorites_and_ratings_are_optimistic_then_persisted`].
* **Fonte da verdade** continua sendo `asset.json`; o índice é descartável (`Gestão ▸ Reconstruir índice`).

## 3. Virtualização

`egui::ScrollArea::show_rows` com altura de linha fixa (card = miniatura quadrada + 34 pt de texto; lista = 30 pt). Só as linhas
visíveis viram widgets; cada card é um `interact` + pintura direta (sem `Frame`/`Grid`). O número de shapes por quadro **não cresce
com o tamanho da biblioteca** [T `shape_count_does_not_grow_with_library_size`: 400 vs 4 000 ativos]. Arrastar a barra de rolagem até o
meio de 6 000 itens carrega só as páginas próximas [T]. O ponto de entrada do smoke de entrada real é `ANIGO_TRACE_LIB=1`.

### Medições (CPU da UI por quadro: layout + tessellation, egui sem janela)

Teste: `crates/anigo-studio/src/library_ui/tests.rs::run_frame_time_bench` — biblioteca sintética (manifestos reais em disco), rolagem
contínua do topo ao fim em 240 passos + rolagem fina + quadros parados. **Não inclui a rasterização da GPU.**

| Build | Ativos | Janela (colunas) | Rolando: mediana / p95 / max | Parada: mediana | Páginas em RAM |
|---|---|---|---|---|---|
| **release** | **50 000** | 1440×900 (4 col., 12 500 linhas) | **0,36 ms / 3,25 ms / 7,51 ms** | 0,30 ms | 48 (teto) |
| release | 5 000 | idem (1 250 linhas) | 0,86 ms / 1,31 ms / 3,83 ms | 0,26 ms | 25 |
| debug (CI) | 5 000 | idem | 14,1 ms / 34,0 ms / 54,8 ms | 3,64 ms | 25 |

Leitura honesta: no 50 000 a mediana é menor que no 5 000 porque a rolagem de 240 passos é tão rápida que muitos quadros mostram os
cards "esqueleto" (a página ainda está chegando da thread de consulta — que é exatamente o comportamento projetado: a UI nunca espera).
O custo de um quadro **com os cards desenhados** é o da coluna "Parada" (~0,3 ms) e o do 5 000 (~0,9 ms com layout novo a cada passo).
A janela tem só ~25 cards visíveis; em 4K/cards pequenos serão mais (o custo é linear no visível, não no total). Ambiente: 4 vCPUs,
tessellation incluída, sem GPU (a rasterização do egui não entra), SQLite em disco virtual com cache quente. As miniaturas ficam fora
(biblioteca sintética sem payload: 0 texturas), então o custo de upload/desenho de imagens **não** está nestes números.
O orçamento dos testes é folgado de propósito (debug: p95 < 120 ms; release: p95 < 8 ms) — pega regressão grosseira, não mede 60 FPS.

Reproduzir:

```text
# 5 000 itens em debug (roda no CI; orçamento folgado)
cargo test -p anigo-studio grid_frame_time_with_synthetic_library -- --nocapture
# 50 000 itens em release (o número do plano T-5.10)
cargo test -p anigo-studio --release grid_frame_time_50k -- --ignored --nocapture
# qualquer N
ANIGO_BENCH_N=20000 cargo test -p anigo-studio --release grid_frame_time_with_synthetic_library -- --nocapture
```

## 4. Miniaturas

* **Pedido por visibilidade.** A cada quadro a grade monta a lista de desejos (visíveis primeiro, depois ± uma tela) e a entrega ao
  `ThumbService` só quando ela muda. O que sai da tela deixa de ser atendido (cancelamento implícito). O tamanho em disco é o mais
  próximo do tamanho em pixels físicos (64/128/256/512, `thumb_service::bucket`).
* **Duas filas.** Dois *loaders* decodificam do cache (`ThumbnailCache::load`: RAM → disco); o que falta vai para **uma** thread de
  render (`ThumbnailCache::ensure` + `ThumbnailRenderer`) — **um job de GPU por vez**. Itens sem miniatura possível (tipo sem
  renderer, erro) entram em `failed` e não são pedidos de novo até limpar o cache.
* **Renderer 3D** (`library_ui/cel_thumb.rs`, `CelThumbnailRenderer`): `anigo_renderer::HeadlessRenderer` com **device próprio**
  criado na primeira miniatura; `Mutex` serializa a GPU. Lê **GLB/VRM** (`Mesh::from_glb_bytes`), **OBJ** (leitor próprio com n-gons,
  índices negativos, normais suaves se faltarem) e **FBX** (`anigo-native`/ufbx, malhas em espaço do mundo). Enquadra pela esfera
  envolvente da caixa (câmera 3/4: yaw 35°, pitch 22°, fov 30°), luz de estúdio presa à câmera (a mesma do viewport), **fundo neutro**
  (`THUMB_BG`), skinning neutralizado (pose de repouso). Se o ativo tem **um único** material canônico, ele é aplicado
  (`ToonMaterial::to_stylized`). A chave do cache inclui `pipeline_ver` (`cel_thumb::PIPELINE_VER`): mude-o quando o visual mudar.
* **Sem GPU:** `HeadlessRenderer::new` falhando vira `Err("sem GPU…")`, a `ChainRenderer` cai no renderer CPU (imagem → swatch de
  material → ícone) e a UI mostra o ícone do tipo; sem `panic` [T `thumbnails_degrade_to_icons_when_the_gpu_is_unavailable`, que usa
  `CelThumbnailRenderer::unavailable`]. A barra inferior mostra "GPU ok / sem GPU: ícones".
* **Validação:** lavapipe/Xvfb [S: `studio_biblioteca-grid.png` etc.; T `real_3d_thumbnail_is_rendered_and_cached_when_a_gpu_exists`,
  que renderiza um cubo, confere cobertura de pixels e as 4 escalas em disco].
* **Preview 3D do painel de detalhes** usa outro `CelThumbnailRenderer` em thread própria (pedido mais novo vence): arrastar gira,
  roda aproxima, duplo clique reinicia, "Girar" liga o turntable.

## 5. Locais padrão e migração do MVP (T-5.0)

* Biblioteca gerenciada padrão: `Documents/ANIGO/Library` (`ANIGO_LIBRARY_DIR` sobrescreve). Index e cache dentro dela.
* O diretório antigo `Documents/ANIGO/Assets` (`ANIGO_ASSETS_DIR` sobrescreve) e as bases canônicas do repositório (`assets/models`,
  só quando o app roda de uma árvore de desenvolvimento) entram, na primeira abertura, como **locais LIGADOS** (o `scan_glb_dir` antigo
  foi removido). O scan incremental (mtime+tamanho) roda ao abrir.
* **Exportar objeto ativo p/ biblioteca** (Inspector da Modelagem): gera o GLB e o registra com `Library::import` como ativo
  **autoral** (tags `autoral`, `modelagem`); exportar de novo o mesmo nome entra como **nova revisão** do mesmo ativo
  [T `authored_assets_get_a_new_revision_instead_of_duplicates`].
* **Editar na Modelagem:** `import_model_mesh` (GLB/VRM/OBJ/FBX → `EditMesh` com `quadify`) [T `edit_in_modeling_emits_a_host_action_only_for_meshes`].

## 6. Atalhos

| Atalho | Ação |
|---|---|
| clique / Ctrl+clique / Shift+clique | selecionar / alternar / faixa (o Shift mantém o âncora; Ctrl define novo âncora) |
| duplo clique, Enter | Editar na Modelagem (ativos de malha) |
| ← ↑ → ↓ (Shift) | mover a seleção (estender) |
| Ctrl+A / Esc | selecionar tudo / limpar |
| F | alternar favorito |
| Del | mover para a lixeira (na lixeira: excluir definitivamente, com confirmação) |
| `/` ou Ctrl+F | focar a busca |
| botão direito | menu de contexto |
| arrastar um card | soltar em coleção, tag, Favoritos ou Lixeira |
| arrastar arquivos do SO | importar |

Os atalhos só valem quando nenhum campo de texto tem foco e nenhum modal está aberto.

## 7. Arrastar-e-soltar para o viewport (gancho)

Cada card publica `AssetDragPayload { ids, label, kind }` via `Response::dnd_set_drag_payload` (a seleção inteira quando o card
arrastado faz parte dela). **Soltar no viewport da Modelagem/Cenário NÃO foi implementado**: esses workspaces e a Biblioteca nunca estão
na tela ao mesmo tempo (a Biblioteca ocupa a janela). Para ligar: mostrar um painel de Biblioteca ao lado do viewport e, no código do
viewport, ler `response.dnd_hover_payload::<AssetDragPayload>()` (fantasma) e `dnd_release_payload::<AssetDragPayload>()` (instanciar o
ativo no ponto sob o ponteiro, usando `Library::resolve_files` + `library::import_model_mesh`). O tipo é público em `library_ui`.

## 8. Cenários de screenshot, CI e smoke

```text
cargo build -p anigo-studio
scripts/native_library_screens.sh ./target/debug/anigo-studio native-artifacts      # todos os cenários
anigo-studio --workspace biblioteca --scenario biblioteca-grid --screenshot grid.png # um cenário
scripts/native_library_smoke.sh ./target/debug/anigo-studio /tmp/smoke               # xdotool no Xvfb
```

Cenários (`--scenario`): `biblioteca-grid`, `-lista`, `-detalhes`, `-3d`, `-busca`, `-import`, `-gestao`, `-duplicados`, `-lixeira`,
`-ausentes`, `-importar`, `-seletor`, `-relink`. Cada um monta uma biblioteca **temporária** (6 000 ativos sintéticos por padrão;
`ANIGO_SCENARIO_N=50000`) com manifestos reais em disco mais **ativos reais gerados pelo próprio studio**: GLB das primitivas e das
bases canônicas, OBJ, PNG, materiais toon, um local ligado com um arquivo apagado (ausente), um item na lixeira, um duplicado, duas
dependências e duas coleções (uma inteligente). A UI só é capturada quando a consulta, as miniaturas e os jobs assentam
(`LibraryWorkspace::is_settled`). Os ativos sintéticos **não têm arquivo de payload**: o ícone do tipo é a miniatura correta deles e
`verify` os reporta como "arquivo ausente" (esperado).

O job `native` do CI roda os testes (`-p anigo-assets -p anigo-studio`), o benchmark com 50 000 em debug, os screenshots e o smoke.
O smoke usa `xdotool` **somente** dentro de um Xvfb próprio (`DISPLAY` fixo no script) e confere, por `libtrace`, que a roda do
mouse rolou a grade, que o clique selecionou e carregou os detalhes, que Ctrl+clique somou e que Ctrl+A selecionou todos os itens.
Nota prática: `xdotool key ctrl+a` (tudo de uma vez) perdeu o modificador no winit; o script usa `keydown/key/keyup` explícitos.

## 9. Testes

`cargo test -p anigo-studio` (44 da Biblioteca): dados/paginação/geração, miniaturas (serviço, LRU de texturas, renderer 3D, OBJ),
workspace headless (seleção, busca, favoritos, import com erros por arquivo, soltar arquivos do SO, lixeira/restaurar/excluir/bloqueio por
dependentes, duplicados, remover local ligado sem tocar nos arquivos, editar na Modelagem, ativo autoral com revisão, soltar em alvos
do painel, metadados no índice FTS), cenário sintético, benchmark da grade e i18n (toda chave `lib.*` usada no código existe em pt e en).
Nenhum exige GPU (o teste do renderer 3D se adapta: com adaptador renderiza de verdade; sem, prova que não dá `panic`).

## 10. Limites conhecidos

* Miniaturas 3D mostram **geometria + material canônico único**; **não aplicam texturas** do modelo, múltiplos materiais por submalha,
  morphs nem pose. Skins ficam em repouso. Câmera de miniatura não é editável; sem turntable no hover (só no painel de detalhes).
* OBJ: sem eixo-Z-up/escala por MTL; FBX: sem texturas embutidas; glTF separado (`.gltf`) e Draco/meshopt não geram miniatura 3D (ícone).
* Seleção por faixa/Ctrl+A guarda até a lista inteira em memória como ids (50 000 ids ≈ 1 MiB; ok); operações em lote sobre ela são
  uma única chamada ao backend, mas `trash`/`purge` de dezenas de milhares serializam por manifesto (lento, sem barra de progresso).
* "Duplicar" reimporta o arquivo primário (copia, não clona sub-recursos nem nota/favorito).
* Preferências (modo grade/lista, tamanho do card, ordenação) **não** são persistidas entre sessões.
* O navegador de pastas é próprio (egui, sem `rfd`): não mostra volumes/atalhos do SO além de Home e raiz.
* Uma única janela do app por biblioteca (o backend não coordena dois processos).
* Textos: pt-BR e inglês (`i18n_lib.rs`); japonês não.

## 11. O que NÃO foi validado

* **Sensação de rolagem em GPU real** e a 60 FPS de verdade: só há o custo de CPU da UI (seção 3) e o smoke em lavapipe (rasterização
  por CPU, não representativa). Latência real de miniaturas 3D em GPU real, uso de VRAM e interferência com o viewport da Modelagem
  (device próprio ajuda, mas não foi medido) são desconhecidos.
* **Gosto visual**: tema, densidade, tipografia, cor do fundo das miniaturas, enquadramento 3/4 — foram só inspecionados em PNG do Xvfb.
* **Arquivos reais** VRoid/UniVRM, Blender, Sketchfab, Quixel/Megascans, FBX reais: todos os ativos de teste e de cenário são
  sintéticos ou gerados pelo próprio studio. Nenhum FBX válido foi renderizado (só o caminho de erro com lixo/truncado).
* **Windows/macOS**: tudo rodou em Linux. Caminhos longos, letras de unidade, `\\`, antivírus, notify fora do inotify, seletor de
  pastas com unidades.
* **Disco frio/rotativo/rede** e bibliotecas > 50 000 na UI; HiDPI (`pixels_per_point` ≠ 1 só foi exercitado por cálculo do bucket).
* **Acessibilidade** (leitor de tela, navegação só por teclado nos painéis), IME em campos de texto.
* Drag-and-drop de arquivos do SO foi testado injetando `RawInput.dropped_files` (headless), **não** com um gerenciador de arquivos real.
