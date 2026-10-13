# Biblioteca de ativos — backend (`crates/anigo-assets`)

**Estado:** backend implementado e testado em Linux (sandbox, sem GPU). A UI que consome esta API está descrita em [`BIBLIOTECA_UI.md`](BIBLIOTECA_UI.md).
Cobre T-5.2, T-5.3, T-5.4, T-5.5, T-5.6 (parte CPU), T-5.8 (parte de dados) e T-5.10 (medido em
[`BIBLIOTECA_BENCH.md`](BIBLIOTECA_BENCH.md)). Pesquisa de origem: [`pesquisa/biblioteca_assets.md`](pesquisa/biblioteca_assets.md).

Legenda de confianca neste documento: **[T]** coberto por teste automatico que rodou; **[L]** lido no codigo/pesquisa, NAO
exercitado; **[N]** nao validado (ver secao 12).

## 0. Testes

`cargo test -p anigo-assets`: 156 testes + 1 doctest passam (90 unitarios, 14 import, 14 lifecycle/integridade, 15 scan linked, 13 miniaturas/seguranca/watcher, 9 consulta/metadados, 1 escala de 5 000 ativos) e tambem com
`--no-default-features` (sem `fbx`/`watch`). `tests/scale.rs::fifty_thousand_assets_meet_the_50ms_search_budget` (`#[ignore]`, `--release -- --ignored`) passou: 6 consultas com mediana de 0,4 a 6,6 ms em 50 000 ativos.
`cargo check --workspace --all-targets` continua passando. Nenhum teste exige GPU, rede ou privilegios.

## 1. Principios

1. **`asset.json` na pasta do ativo e a verdade.** O `index.db` (SQLite) e um indice descartavel: `Library::rebuild_index()`
   o reconstroi (e a abertura faz isso sozinha se o arquivo sumir) [T].
2. **Identidade estavel**: `AssetId` = UUID v7, independente do caminho. A **revisao** e imutavel e identificada por BLAKE3 do conteudo.
3. **Managed e imutavel**; **linked nunca altera nem apaga** a pasta externa (o manifesto do ativo linked vive em `home/linked/`) [T: a pasta externa fica identica byte a byte apos o scan; `purge` de ativo linked nao apaga o arquivo].
4. **Nada de `panic` em entrada malformada**: analise em `catch_unwind`, limites configuraveis, URIs confinadas, erros por arquivo
   sem abortar o lote [T: arquivos truncados em todas as posicoes, bytes corrompidos, lixo, zero bytes, traversal, symlink].
5. **Sem tokio.** Trabalho pesado em threads (`std::thread` + `crossbeam-channel`), paralelismo de CPU com `rayon` em pool local.
6. **Sem GPU**: o render 3D da miniatura entra por um trait (`ThumbnailRenderer`), implementado depois no `anigo-studio`.

## 2. Layout em disco

```text
<home>/                         = Library::open(home)
  anigo-library.json            config: library_id, raizes (verdade das raizes)
  collections.json              snapshot das colecoes (restaurado se o indice for perdido)
  index.db (+ -wal / -shm)      indice SQLite (OpenOptions::index_dir move para disco local)
  assets/ab/cd/<uuid>/          ativo MANAGED (shard pelos ULTIMOS bytes do uuid; os primeiros do v7 sao tempo)
      asset.json                manifesto (verdade)
      r1/ r2/ ...               uma pasta por REVISAO, arquivos com a estrutura relativa original
      materials/000_<nome>.anigomat.json   materiais canonicos gerados no import
  linked/ab/cd/<uuid>/          manifesto (+ materials/) de ativo LINKED; arquivos ficam fora
  trash/<yyyymmdd>/<uuid>/      lixeira (soft delete); mesma estrutura do ativo
  .staging/<uuid>/              import em andamento (limpo ao falhar/cancelar; cleanup_staging varre orfaos)
  cache/thumbs/<64|128|256|512>/<hh>/<blake3>-v<pipeline_ver>.png    DESCARTAVEL (OpenOptions::cache_dir)
```

Raizes managed adicionais (`add_root(.., Managed)`) repetem `assets/`, `trash/`, `.staging/` dentro da propria pasta.
Desvios deliberados em relacao ao rascunho da pesquisa: config em **JSON** (nao TOML; evita dependencia), pastas de revisao `r<N>/`
(em vez de `src/`), thumbs em PNG (o `image` do workspace so tem png/jpeg).

## 3. Modelo de dados

* `AssetManifest` (serde, `schema = 1`): `asset_id`, `name`, `kind` (`model|character|outfit|hair|accessory|material|texture|pose|animation|hdri|scene|prefab`),
  `description/author/license/source_url`, `format`, `storage`, `primary`, `link{root_uuid, rel_path}` (linked), `current_revision`,
  `revisions[{rev, content_hash, size, created_at, files[{path,size,blake3}]}]`, `tags[]`, `rating 0..5`, `favorite`,
  `stats{tris,verts,meshes,nodes,materials,textures,bones,morphs,anims,has_skin,has_vrm,bbox,width,height}`, `dependencies[{id,kind}]`,
  `materials[]` (arquivos em `materials/`), `trashed_at`, `props`, `warnings`, timestamps. Validado ao ler (`AssetManifest::validate`:
  schema, revisao atual existe, caminhos confinados) [T].
* **Hash de conteudo**: arquivo unico = BLAKE3 do proprio arquivo (verificavel com `b3sum`) [T]; varios arquivos (OBJ+MTL+texturas, glTF+bin+imagens)
  = BLAKE3 da lista ordenada `"<caminho>\0<hash hex>\n"` (`combined_hash`) [T]. Duplicado = mesmo hash.
* **Revisao**: `import(.., replace: Some(id))` (managed) ou `scan` (linked, arquivo mudou) acrescenta uma revisao e mantem o `AssetId`,
  tags, nota, favorito [T]. A pasta `r<N>` anterior permanece.
* Tags: caminho hierarquico normalizado (minusculo, `/`, ate 8 niveis x 64 caracteres). Filtrar por `estilo` casa `estilo/cel/noir` [T].

## 4. Esquema SQLite (`PRAGMA user_version = 1`)

WAL, `synchronous=NORMAL`, `foreign_keys=ON`, `busy_timeout=10 s`; 1 conexao escritora (mutex) + pool de ate 6 leitoras somente-leitura;
cache de 32 MiB (escritora) / 8 MiB (leitora); `mmap_size` 128 MiB. Migracoes: `db::MIGRATIONS[i]` leva de `i` a `i+1` dentro de transacao;
banco mais novo que o binario => `AssetError::SchemaTooNew` [T: incremental, idempotente, rollback em falha, recusa de versao futura].

| Tabela | Conteudo |
|---|---|
| `meta` | `library_id` |
| `root` | `id, uuid, name, path, mode, online, watch, last_scan` (espelha `anigo-library.json`; a 1 e a propria pasta) |
| `asset` | `seq` (INTEGER PK = rowid estavel sob VACUUM), `id BLOB UNIQUE`, `root_id`, `rel_path`, `manifest_dir`, `name`, `name_key` (minusculo p/ ordenar), `kind`, `format`, `description/author/license/source_url`, `content_hash`, `size`, `primary_size`, `storage`, `state` (0 active, 1 trashed, 2 missing, 3 corrupt), `rating`, `favorite`, `revision`, `tris/verts/materials/bones/anims`, `has_skin/has_vrm`, `file_mtime_ns`, `imported_at/modified_at/trashed_at` (STRICT) |
| `tag`, `asset_tag` | tag hierarquica (`path UNIQUE`, `color`) e ligacao `(tag_id, asset_seq)` WITHOUT ROWID |
| `dep` | `(asset_id, dep_id, kind)` + indice reverso (`used_by`) |
| `collection`, `collection_item` | manual (lista de ids) e inteligente (`query` = `Query` serializada em JSON, nunca SQL) |
| `file_claim` | arquivos dependentes de ativos linked (o scan nao os registra como ativos soltos) |
| `asset_fts` | FTS5 `name, description, tags, author`, `unicode61 remove_diacritics 2`, `prefix='2 3'`, `rowid = asset.seq` |

Indices de ordenacao `(state, name_key|kind+name_key|imported_at|modified_at|size|rating, seq)` para keyset. Nao ha tabelas `catalog`, `thumb`
nem `job` (itens do rascunho da pesquisa): colecoes+tags cobrem catalogos; o estado de miniatura e o proprio arquivo em disco; "retomar" import
e re-executar (idempotente por hash).

## 5. API publica (resumo)

Tudo em `anigo_assets::*`. `Library` e `Clone + Send + Sync` [T]; jobs sao `Job<Evento, Resultado>` com `events()` (crossbeam),
`cancel()`, `cancel_token()`, `is_finished()`, `join()` (um `panic` do worker vira `AssetError::Internal`) [T].

| Area | API |
|---|---|
| Abrir | `Library::open(path)`, `open_with(path, OpenOptions{index_dir, cache_dir, limits, create, pipeline_ver})`, `home/index_path/cache_dir/library_id/schema_version` |
| Raizes | `roots()`, `root(id)`, `refresh_roots()`, `add_root(path, Managed|Linked, nome)`, `remove_root(id, delete_files)` (linked: so esquece; managed: so apaga arquivos com `delete_files`) |
| Import | `import(path, ImportOptions) -> ImportJob`, `import_blocking`, `preview(path, limits)`, `cleanup_staging`, `find_by_hash`, `find_by_path` |
| Scan | `scan(root, ScanOptions) -> ScanJob`, `scan_blocking`, `check_presence(root)` |
| Consulta | `query(&Query) -> Page<AssetRow>`, `count`, `query_ids`, `get(id) -> AssetDetail`, `row`, `require_row`, `manifest`, `resolve_files` (caminhos absolutos), `asset_dir` |
| Metadados | `set_favorite`, `set_rating`, `rename`, `update_info(InfoPatch)`, `set_tags/add_tags/remove_tags/add_tags_many`, `tags()`, `set_tag_color`, `rename_tag`, `delete_tag`, `prune_tags`, `tags_of` |
| Colecoes | `create_collection`, `create_smart_collection(nome, Query)`, `update_smart_collection`, `rename_collection`, `delete_collection`, `add_to_collection`, `remove_from_collection`, `collection(s)`, `collections_of` |
| Dependencias | `dependencies(id)`, `used_by(id)`, `add_dependency` (rejeita ciclo), `remove_dependency`, `reference_graph(raiz, profundidade, incluir_usuarios, max_arestas)` |
| Lixeira | `trash(ids, TrashOptions{force})` (bloqueia ativos com dependentes ativos), `restore`, `purge`, `empty_trash(older_than_secs)` |
| Integridade | `find_duplicates(limite)`, `resolve_duplicates(keep, remove)`, `find_missing`, `relink`, `relink_candidates`, `relink_root`, `verify(VerifyOptions{rehash, fix}) -> VerifyJob`, `rebuild_index`, `rebuild_index_job` |
| Materiais | `asset_materials(id)`, `load_material(id, arquivo)`, `ToonMaterial::{from_json,to_json,to_stylized,from_stylized,validate}`, `material::convert::*` |
| Miniaturas | `thumbnails() -> Arc<ThumbnailCache>`, `thumb_request(id)`, `assets_without_thumbnail(query, max)`, `generate_thumbnails(ids, renderer, ThumbOptions) -> ThumbJob`, trait `ThumbnailRenderer` |
| Watcher | `watch(root, debounce) -> Watcher` (feature `watch`, padrao ligada) |

### 5.1 Consulta

`Query { text, kinds, tags (E), any_tags (OU), collection, favorite, min_rating, formats, root, storage, min_tris, max_tris, state (Active padrao | Trashed | Missing | Corrupt | Any), sort {by: Name|ImportedAt|ModifiedAt|Size|Rating|Tris|Relevance, desc}, limit (0 = 200, max 1000), after: Option<Cursor>, with_total }`.

* **Paginacao keyset** `(chave, seq)`: `Page.next` e opaco; passe em `Query.after`. Exata (sem repeticao/omissao) com empates e nas 6 ordenacoes + relevancia [T]. O cursor invalida se o indice for reconstruido; usar com outra ordenacao => `InvalidInput`.
* **Texto** (`parse_text`, nunca repassa texto cru ao FTS5 [T: ~30 entradas hostis, validadas no proprio SQLite]): palavras = E, casam por prefixo (`cabel` acha `cabelo`),
  `"frase exata"`, `-palavra` exclui, `tag:`/`name:`/`author:`/`desc:` restringem a coluna; diacriticos ignorados; maximo 32 termos / 512 caracteres.
* Colecao inteligente = consulta salva combinada (E) com a do chamador; combinacao impossivel (ex.: tipos disjuntos) devolve vazio sem consultar [T].
* `AssetRow.available = false` quando a raiz do ativo esta offline (HD externo).

### 5.2 Receita para a UI virtualizada (nao implementada aqui)

`total = count(&q)` (cacheie; 4 ms a 200 k) -> `show_rows(total_rows)`; para a faixa visivel use `query` com `after` guardado por pagina de 200
(cursores sao baratos de guardar) e um mapa `pagina -> Vec<AssetRow>` com prefetch de +-2 telas. Miniatura da celula visivel:
`lib.thumbnails().load(&row.content_hash, 128)` (RAM -> disco, `Arc<RgbaImage>`) ou `load_or_placeholder(hash, kind, 128)` para ter
algo desenhavel NA HORA; se `None`, enfileire `generate_thumbnails(vec![id], renderer, ..)`. Eventos de jobs: `job.events().try_iter()` a cada quadro.

## 6. Import (T-5.4)

`import(path, opts)`: `path` = arquivo ou pasta (recursivo, ignora ocultos, nao segue symlinks de diretorio [T]). Fases: enumerar -> **modelos primeiro** ->
preparar em paralelo (analise + copia para `.staging` com BLAKE3 na mesma passada) -> commit sequencial (decidir duplicado/revisao, mover staging, escrever
`asset.json`, indexar uma transacao por lote). Texturas/buffers referenciados por um modelo importado NAO viram ativos soltos [T].

| Formato | O que e extraido | Observacao |
|---|---|---|
| GLB / glTF / VRM 1 | stats (tris, vertices, malhas, nos, materiais, texturas, ossos, morphs, animacoes, bbox por min/max dos accessors), buffers/imagens externos como arquivos do ativo, materiais -> toon, metadados VRM (`name`, `authors`, `licenseUrl`), kind `character` se `VRMC_vrm.humanoid` | usa `anigo_vrm::gltf::{parse_glb, parse_gltf, summarize_import}` (validacao estrutural completa). `KHR_draco_mesh_compression` obrigatoria => recusado com mensagem clara [T]; `EXT_meshopt_compression` => catalogado com aviso. VRM 0.x: detectado, materiais nao convertidos (aviso) |
| OBJ (+MTL) | contagem de vertices/triangulos (n-gons), bbox, `mtllib` (confinada), texturas do MTL (relativas a pasta do MTL), materiais PBR->toon | leitor em streaming com limite de linhas e bytes/linha [T] |
| FBX | **via `anigo-native` (ufbx), feature `fbx` (padrao ligada)**: stats, ossos, morphs, clips, bbox, materiais+textura base (so o nome do arquivo, procurada ao lado do FBX) | sem a feature: catalogado como opaco com aviso. Texturas EMBUTIDAS no FBX nao sao extraidas [N]. Testado so com lixo/truncado (nao ha FBX real no repo de testes) |
| PNG / JPEG | dimensoes do cabecalho, limite de dimensao | corpo truncado so e detectado ao gerar a miniatura |
| TGA/BMP/WebP/DDS/KTX2/EXR/HDR | catalogados como textura (`hdri` para `.hdr/.exr`); sem decodificar | miniatura = placeholder |
| `.anigomat.json` | vira ativo `material`; validado | |

Opcoes: `mode (Managed|Linked)`, `root`, `recursive`, `kind`, `name` (so se resultar em 1 ativo), `tags`, `description/author/license/source_url`, `duplicates (Skip|Allow)`,
`replace (nova revisao)`, `collection`, `threads`. Eventos: `Discovered, Progress, Imported, Revised, Duplicate, Skipped, Failed, Relinked, Finished`.
Relatorio: `imported, revised, relinked, duplicates, failed, skipped, bytes, cancelled, elapsed_ms`.

* **Cancelar**: para entre arquivos e DENTRO da copia/hash; o que ja foi confirmado fica; staging descartado; `cancelled = true` [T].
* **Retomar**: re-executar o mesmo import. Ja importados viram `Duplicate` (hash); nenhum duplicado e criado [T].
* **Limites (`Limits`, padroes)**: arquivo 2 GiB, ativo 4 GiB, 4096 arquivos/ativo, analise em memoria ate 512 MiB, JSON ate 128 MiB, 500 k nos, 100 k malhas, 1 M accessors,
  50 k imagens, 200 M vertices, imagem 32768 px, 5 M arquivos por scan, profundidade 64, 100 M linhas de texto, 1 MiB por linha. Todos testados por reducao [T parcial: nos, vertices, bytes, JSON, linhas, linha, arquivos/ativo, dimensao].
* **Seguranca de caminhos** (`paths::normalize_relative/confine`): rejeita vazio, `..` (inclusive `%2e%2e`, `..%2f`), absolutos (`/x`, `C:\x`, `\\srv`), esquemas (`file:`, `http:`), NUL/controle, `:` em componente, e escape por symlink [T]. `data:` URI so em buffers (base64) e imagens embutidas.

## 7. Raizes linked e scan (T-5.3)

`add_root(pasta, Linked)` + `scan(root)`: enumera, pula arquivos cujo `(tamanho do primario, mtime)` nao mudou (sem hash; 2 000 arquivos em 30 ms) [T], hasheia so o novo/alterado,
**arquivo mudou => nova revisao** [T], **arquivo sumiu => `missing`** [T], **arquivo reaparece no mesmo caminho => `active`** [T], **arquivo movido/renomeado (mesmo hash) => relink, mesma identidade** [T] (`detect_moves`),
raiz inacessivel => `online=false` e NADA e marcado ausente [T]. Duplicatas dentro de pastas ligadas sao registradas (`DuplicatePolicy::Allow` no scan) para nao re-hashear a cada scan.
Ativo mandado para a lixeira nao e reimportado pelo scan; depois do `purge` o arquivo e redescoberto como novo [T].
Mudancas apenas nos ARQUIVOS DEPENDENTES (textura, .bin) de um modelo linked nao sao detectadas pelo mtime do primario; use `ScanOptions::rehash_all` ou `verify`.
`relink(id, arquivo)` exige mesmo tamanho+BLAKE3; `relink_candidates(id, pasta)` procura por hash; `relink_root(root, novo_caminho)` troca o caminho de um disco remontado e roda `check_presence` [T].
`watch(root, debounce)` (feature `watch`, `notify` 8): apos o silencio roda um `scan` e emite `WatchEvent::{Changed, Scanned, Error}` [T no inotify do sandbox]. Watchers perdem eventos: mantenha o scan na abertura.

## 8. Material canonico "ANIGO Toon Material" (T-5.5)

`ToonMaterial` serializa como `.anigomat.json` (`schema = "anigo.toon-material"`, `schema_ver = 1`). Blocos: `base`, `shade` (cor/textura 1 e 2, `threshold`, `smoothness`, `steps`, `hue_shift`, segunda sombra, `ramp`,
`shift_texture`, e `mtoon{shading_shift, shading_toony, gi_equalization}` preservado), `ilm` (textura + papel por canal), `spec`, `rim`, `matcap`, `outline`, `face_sdf`, `emission`, `normal` (+ `convention`),
`uv_anim`, `pbr_fallback`, `transparent_with_zwrite`, `render_queue_offset`, `extras`. Cada textura declara `role`, `color_space` (cor = sRGB, dados = linear), `tex_coord`, `scale`, `swizzle`,
e a fonte: `uri` relativa confinada **ou** `gltf_image` (indice de imagem embutida). Cores lineares. Leitura valida schema/versao/NaN/uri e faz `clamp` das faixas [T].

* `to_stylized()` / `from_stylized()` ligam ao `anigo_core::StylizedMaterial`; o padrao do `ToonMaterial` coincide campo a campo com `StylizedMaterial::default()` [T].
* **glTF PBR -> toon** (`toon_from_pbr`): sombra DERIVADA (heuristica): `shade = base * 0.72` + `hue_shift = -15` do renderer; a mesma imagem da cor base vira `shade.texture`; contorno no estilo da casa; rim desligado;
  especular de `roughness/metallic`; `KHR_materials_emissive_strength`, `unlit`, MASK/BLEND, `pbrSpecularGlossiness` (so diffuse, com aviso); demais `KHR_materials_*` geram aviso [T].
* **`VRMC_materials_mtoon` -> toon** (`toon_from_mtoon`): mapeamento dos parametros da spec 1.0 — `shadeColorFactor`, `shadeMultiplyTexture`, `shadingShiftFactor/Texture`, `shadingToonyFactor`, `giEqualizationFactor`, `matcapFactor/Texture`
  (aditivo), `parametricRimColor/FresnelPower/Lift`, `rimMultiplyTexture`, `rimLightingMixFactor`, `outlineWidthMode/Factor/MultiplyTexture`, `outlineColorFactor`, `outlineLightingMixFactor`, `uvAnimation*`,
  `transparentWithZWrite`, `renderQueueOffsetNumber`; defaults da spec quando ausentes [T com documentos construidos a partir da spec].
  Derivados para o renderer: `threshold = 0.5 - 0.5*shift` (o shader acende quando `half_lambert > threshold`; em MToon `shift > 0` acende mais) e `smoothness = (1 - toony)/2`.
  **Isto difere do sinal de `anigo_vrm::vrm1::map_mtoon_to_anime_material` (`0.5 + 0.5*shift`)**; o meu foi derivado lendo `cel_shading.wgsl`, mas NAO foi comparado visualmente [N].
* **MTL** (`parse_mtl`): `Kd/Ke` (sRGB -> linear), `Ns -> roughness = sqrt(2/(Ns+2))`, `d/Tr`, `map_Kd/map_Ke/map_Bump` (com opcoes `-s ...` ignoradas) [T]. **FBX**: cor base + textura -> PBR -> toon.
* No import, os materiais de cada modelo sao gravados em `materials/` e listados em `manifest.materials`; `asset_materials(id)`/`load_material(id, arquivo)` leem.
* NAO implementado: export GLB/VRM a partir do toon, promocao de material a ativo proprio ("Extrair..."), KTX2/Basis, ILM/face-SDF/ramp **detectados** automaticamente (so existem se o `.anigomat.json` os declarar).

## 9. Miniaturas (T-5.6, parte CPU)

* `ThumbnailCache` (`lib.thumbnails()`): disco `<cache>/thumbs/<lado>/<hh>/<blake3>-v<pipeline_ver>.png`, tamanhos **64/128/256/512** (512 = master; 256/128/64 por `fast_image_resize` Lanczos3,
  alpha tratado — borda transparente nao escurece [T]); LRU em RAM por **orcamento de bytes** (128 MiB padrao) [T]; escrita atomica; PNG corrompido e apagado ao ser lido (regerado) [T];
  `purge_stale(keep)` (versoes antigas/hashes orfaos), `evict_to(bytes)` (mais antigo por mtime), `clear()`, `disk_usage()` [T].
* **Invalidacao**: a chave e `blake3 + pipeline_ver`. Mudar o visual => suba `OpenOptions::pipeline_ver` [T]. Mudou o conteudo => hash novo.
* `ensure(req, renderer)`: renderiza so o que falta; **placeholder (icone por tipo) nunca vai para o disco**; erro/panic do renderer vira `Err`, nao derruba [T].
* Renderizadores CPU: `ImageThumbnailRenderer` (PNG/JPEG com limites), `MaterialSwatchRenderer` (esfera toon desenhada a partir do `.anigomat.json`), `PlaceholderRenderer`, `ChainRenderer`, `ClosureRenderer`,
  `default_cpu_renderer()` = imagem -> material -> placeholder [T]. **Modelos 3D (GLB/OBJ/FBX) hoje caem no placeholder.**
* **Como plugar o render cel (GPU)**: no `anigo-studio` implemente `ThumbnailRenderer::render(&ThumbRequest) -> Result<Rendered, String>` (carregar `req.primary` com `anigo_core::Mesh::from_glb_file`,
  enquadrar pelo bbox do `AssetRow`/manifesto, `HeadlessRenderer::render_to_texture`, devolver RGBA 512x512 com `placeholder: false`) e use
  `ChainRenderer::new(vec![gpu, default_cpu_renderer()])` em `generate_thumbnails(ids, renderer, ThumbOptions{parallelism: 1, ..})` (**um job de GPU por vez**).
  Nao ha feature `cel-thumbs`: a dependencia do renderer fica no studio.

## 10. Lixeira, duplicados, ausentes, integridade (T-5.8, dados)

* `trash`: move a pasta do ativo para `trash/<data>/<uuid>/` (so o manifesto, no caso linked); **bloqueia** ativos com dependentes ativos (`TrashReport.blocked`) salvo `force` ou dependente no mesmo lote [T].
  `restore` devolve ao lugar original [T]. `purge` so aceita itens ja na lixeira, so apaga DENTRO de `trash/` (indice adulterado nao escapa [T]), limpa dependencias nos manifestos dos dependentes, tira da colecao,
  apaga miniaturas orfas [T]. `empty_trash(Some(segundos))` respeita a idade [T].
* `find_duplicates` (grupos por BLAKE3, maiores desperdicios primeiro, `reclaimable_bytes`); `resolve_duplicates(keep, remove)` une tags/favorito/nota/colecoes, repontua dependentes e manda o resto para a lixeira [T].
* `verify`: por ativo confere manifesto legivel, hash do indice == manifesto, existencia, tamanho e (opcional) BLAKE3 de cada arquivo; dependencias pendentes; **pastas de manifesto que o indice nao conhece e linhas sem pasta**;
  raiz offline. `fix: true` so atualiza estados (`corrupt/missing/active`), remove linhas orfas e indexa manifestos orfaos — **nunca apaga arquivos** [T]. Em linked, conteudo diferente = `ExternalChanged` (nao e `corrupt`).
* `rebuild_index`: reconstroi tudo dos manifestos (em lotes de 4096 para limitar memoria), preserva raizes, cores de tag, mtimes e estados `missing/corrupt`, restaura colecoes de `collections.json`,
  reporta manifestos invalidos sem apagar nada [T].

## 11. Concorrencia

Um escritor SQLite por biblioteca (mutex) e leitoras em paralelo (WAL) [T: consultas durante import]. Edicoes de manifesto (tags, nota, revisao, trash...) serializam por um lock de mutacao
(ler-modificar-gravar). Jobs: `import`, `scan`, `verify`, `generate_thumbnails`, `rebuild_index_job`. `Job` descartado sem `join()` cancela o trabalho.
Duas instancias do APP sobre a MESMA biblioteca nao sao coordenadas (o SQLite serializa escritas, mas os lock de mutacao de manifestos e o cache de configuracao sao por processo) [N].

## 12. O que NAO foi validado (ou nao existe)

* **Nada com arquivos reais de terceiros**: todos os GLB/OBJ/PNG dos testes sao sinteticos (triangulo, cubo). Nao testado com VRoid/UniVRM/Blender/Sketchfab/Quixel, nem com o corpus Khronos glTF-Sample-Assets. Sem `cargo fuzz` (so varredura de truncamento/bit-flip em GLB).
* **FBX**: so lixo/truncado/limites; nenhum FBX valido foi importado (nao ha fixture no repo). A contagem de triangulos/ossos/clips vem de `anigo-native` e nao foi conferida.
* **Mapeamento MToon**: nomes e padroes vieram da spec/pesquisa e de memoria, conferidos so contra documentos que eu mesmo construi. O sinal do `shift` e a conversao `toony -> smoothness` NAO foram comparados visualmente com o renderer.
* **Windows/macOS**: nada rodou fora do Linux. `rename` de diretorios, nomes reservados (`CON`, `NUL`), caminhos longos (>260), maiusculas/minusculas e antivirus nao foram exercitados. `notify` testado so com inotify.
* **Rede**: `index.db` em SMB/NFS pode corromper ou travar (risco da pesquisa); use `OpenOptions::index_dir` num disco local. Raizes linked em rede funcionam em principio, nao testado.
* **Desempenho a frio** (cache de pagina do SO vazio), disco rotativo, bibliotecas de 10^6 ativos, FTS com vocabulario real. Ver `BIBLIOTECA_BENCH.md`.
* **UI**: virtualizacao, 60 FPS, drag-and-drop, uso de memoria do app inteiro.
* **GPU/render 3D das miniaturas**: apenas o contrato (`ThumbnailRenderer`).
* Sem: Draco, meshopt, KTX2/Basis, USD, `.anigopack` (T-5.9), `AssetSource` remoto (T-5.12), sub-assets promoviveis, turntable, paleta de cores/`thumbnail_camera`, importador Bridge/Megascans, edicao simultanea por 2 processos.
* Compatibilidade futura: o `schema` do manifesto e `user_version` estao versionados, mas nao ha ainda nenhuma migracao real alem da v1.

## 13. Observacoes sobre `anigo-vrm` (nao alterado)

Achados ao reaproveitar o parser (por leitura do codigo; o item 2 foi confirmado por teste que falhou):

1. `parse_gltf` procura `extensionsRequired` em `/asset/extensionsRequired`; na spec glTF fica na raiz do documento — a recusa de Draco nunca dispara. O `anigo-assets` confere no lugar certo antes.
2. `summarize_import` le `extensionsUsed` de `/asset/extensionsUsed` (idem) — `is_vrm` e `vrm_extensions` ficam vazios para arquivos conformes. O `anigo-assets` detecta `VRMC_vrm` por conta propria.
3. `parse_gltf` exige `bytes.len() == buffers[i].byteLength` exatamente; GLB conforme pode ter o chunk BIN com ate 3 bytes de padding a mais. O `anigo-assets` apara o padding no resolvedor.
4. `Vrm1MToon` (`vrm1.rs`) usa nomes de propriedade que nao sao os da spec `VRMC_materials_mtoon` 1.0 (`mainTex`, `shadowColor`, ...); por isso o mapeamento MToon foi reescrito em `material/convert.rs`.
