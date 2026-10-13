# Pesquisa: Biblioteca de ativos profissional para o ANIGO (out/2026)

Legenda: [V] = lido na fonte nesta sessão; [S] = visto só em snippet de busca; [C] = conhecimento prévio/inferência, NÃO verificado agora. Datas = estado em 2026-10-12.
Contexto local [V]: rustc 1.97 (cobre todos os MSRV abaixo); `anigo-core::StylizedMaterial` já espelha MToon (shade_color, toon_steps, rim, outline, matcap, slots de textura); `anigo-vrm` já lê/escreve glTF+VRM1; `anigo-renderer::HeadlessRenderer::render_to_texture` já existe (serve p/ miniaturas); `library.rs` atual (237 linhas) é catálogo em memória com 8 categorias fixas e `AssetRecord` sem hash/DB.

## A) Como os líderes escalam (10^4–10^6 assets)

**Unreal Asset Registry / Content Browser**
- Registry = subsistema que coleta dados de assets NÃO carregados, de forma assíncrona, mantido em memória e "autoritativo", atualizado quando arquivos mudam em disco; o Content Browser é o consumidor principal. Eventos: OnAssetAdded/Removed/Renamed/OnFilesLoaded/progresso; UI mostra indicador de descoberta. [V] https://dev.epicgames.com/documentation/unreal-engine/asset-registry-in-unreal-engine
- `FAssetData` = {ObjectPath, PackageName, PackagePath, AssetName, AssetClass, TagsAndValues}. Tags só de propriedades marcadas `AssetRegistrySearchable` (gravadas no header do .uasset; exige re-save) ou via `GetAssetRegistryTags()`. `GetAllAssets()` "pode ser lento": usa-se `FARFilter` (PackageName/Path/Collection/Class/Tags; AND entre componentes, OR dentro). [V] mesma URL.
- Lição: separar "registro leve e consultável" (índice) do "payload pesado" (arquivo). Nunca carregar o asset para listar.
- Content Browser: 5 tamanhos de miniatura (Tiny..Huge), thumbnails em tempo real opcionais, "Thumbnail Edit Mode" (câmera da miniatura salva no asset), aviso ao carregar muitos assets de uma vez. [S] https://dev.epicgames.com/documentation/unreal-engine/content-browser-settings-in-unreal-engine
- Filtros + Coleções [S] https://dev.epicgames.com/documentation/unreal-engine/filters-and-collections-in-unreal-engine; Reference Viewer: grafo de referências/dependências com profundidade, limite de largura (nós "Overflow"), filtro por tipo/coleção [S] https://dev.epicgames.com/documentation/unreal-engine/reference-viewer-in-unreal-engine
- Mover/renomear deixa "redirector" que precisa ser consertado ("Fix up redirectors") [S] reddit r/unrealengine — lição: ID estável independente do caminho (UUID/hash, não path) evita redirectors.
- DDC (Derived Data Cache): dados derivados são regeneráveis a qualquer hora a partir dos .uasset; hierarquia de caches do mais rápido ao mais lento; Zen Storage Server é o padrão persistente (desde 5.4) [S] https://dev.epicgames.com/documentation/unreal-engine/using-derived-data-cache-in-unreal-engine. Lição: tudo derivado (miniaturas, previews, mips, bounding) vive num cache DESCARTÁVEL, endereçado por (hash do conteúdo + versão do pipeline).

**Blender Asset Browser / Libraries (5.2 LTS)**
- Asset = datablock com metadados (catálogo, descrição, autor, tags); biblioteca = pasta registrada nas Preferências (nome+caminho); .blend em qualquer subpasta são varridos. Blender gera um ÍNDICE de todos os assets da biblioteca, mantido atualizado e guardado no diretório de cache local ("primeira carga lenta, seguintes bem mais rápidas"). [V] https://docs.blender.org/manual/en/latest/files/asset_libraries/introduction.html
- Catálogos = hierarquia lógica independente da pasta no disco (arquivo de definição de catálogos, que deve ser compartilhado junto) [V mesma URL; nome/formato `blender_assets.cats.txt` [C]]. Preview gerado automaticamente ao "Mark as Asset", substituível por imagem própria. "Clear Asset" remove só metadado.
- 5.2: bibliotecas ONLINE (registradas por URL, baixadas sob demanda, cache local, não podem ser "linked", só appended/packed); página dedicada "Asset Libraries" nas Preferências; "All Libraries"/"Essentials"; "Preferred Import Method" por asset. [V] https://developer.blender.org/docs/release_notes/5.2/assets/
- Roadmap Blender: assets não-datablock (imagens, áudio; importadores USD/glTF/FBX) usariam metadados em sidecar XMP. [V introdução acima] — valida "arquivo original + sidecar" para glTF.
- Regra: Blender não escreve em outros .blend — espelho para nós: biblioteca gerenciada imutável após import; edição gera nova revisão.

**Quixel Bridge / Megascans / Fab**
- Bridge: biblioteca LOCAL (pasta com estrutura original + um .json por asset; importadores de terceiros exigem preservar a estrutura e os .json; existiu índice legado `assetsData.json`); LODs (LOD0..n), variações, fbx/abc; formato mudou ao longo do tempo e downloads do site ≠ nomes do Bridge. [S] https://superhivemarket.com/products/megascans-library-importer/docs ; [V] https://odtools.notion.site/Megascan-Assets-925bc7bd32f64e10b5cfc022120455ec . Schema JSON exato NÃO confirmado.
- Epic: todo o conteúdo do Bridge está no Fab; plugin Bridge segue até o app sair do ar (sem data oficial achada). [S] https://dev.epicgames.com/documentation/unreal-engine/quixel-bridge-plugin-for-unreal-engine . Lição: local library (arquivos) + cloud (catálogo remoto + download manager) como dois "providers" na mesma UI.
- Houdini OD Tools: pasta marcada como "Megascan Library" gera galeria pesquisável/filtrável/taggeável e arrasta-e-solta p/ a cena. [V] link acima.

**Outros**
- Eagle: NÃO usa banco: `Lib.library/metadata.json` (árvore de pastas, smartFolders, tagsGroups) + `mtime.json` + `images/<ID>.info/{metadata.json, original, *_thumbnail.png}`; item = {id,name,size,btime,mtime,ext,tags,folders[ids],isDeleted,annotation,star,palettes,width,height}. [S, reverse-engineered] https://github.com/Stef4678/eaglepack-importer/blob/main/docs/FORMAT.md . Crítica de usuários: pasta por item dificulta backup/escala → preferir SQLite + manifestos. Ideias a copiar: `isDeleted` (soft delete), `star`, paleta de cores, `folders` por id, smartFolders.
- Adobe Bridge: cache central de miniaturas/previews, "Purge cache" global e por pasta, previews 100% opcionais purgáveis à parte; reclamações de lentidão em bibliotecas grandes [S] https://helpx.adobe.com/bridge/desktop/get-started/set-cache-management-preferences.html — lição: cache com política (tamanho máx, idade), reconstruível, purge granular.
- Sketchfab Download API: busca por título/tags/categoria/nº de polígonos; devolve ZIP glTF (scene.gltf + scene.bin + textures/) e USDZ com links temporários (300 s) [S] https://sketchfab.com/developers/download-api . Substance 3D Assets: ~18k assets [S] https://www.awn.com/news/adobe-substance-3d-announces-full-assets-library-access-subscribers .
- Sem fontes verificáveis nesta rodada sobre Unity Package Manager/Billfish ([C]: UPM = pacotes versionados com manifesto + dependências; inspiração para "packs" com semver).

**Padrões consolidados (o que copiar)**: 1) índice leve ≠ payload; 2) descoberta assíncrona com eventos/progresso; 3) ID estável + caminho como atributo; 4) derivados em cache descartável, chave=hash+versão; 5) catálogos/coleções lógicas independentes de pastas; 6) coleções dinâmicas (queries salvas) + estáticas; 7) grafo de dependências (Reference Viewer) p/ delete seguro; 8) soft delete + purge; 9) múltiplas bibliotecas (local/projeto/online) via "provider"; 10) sidecar para formatos externos, original intocado.

## B) Formato de ativo ANIGO (recomendação)

**Modelo**: `Asset` = unidade lógica com UUID v7 (`asset_id`) + 1..n `Revision` (conteúdo imutável identificado por BLAKE3). Modos por *root*:
- **Managed** (padrão): arquivos copiados para o root e geridos (dedup, delete, mover, backup); imutável após import.
- **Linked**: aponta para pasta externa (HD/NAS do usuário); nunca altera/apaga; detecta ausência (offline) e permite *relink* por hash/tamanho/nome.
- **Project** (futuro): root dentro do projeto .anigo.

**Disco (managed root)**
```
<root>/
  anigo-library.toml            # {library_id, schema_ver, name, created}
  index.db (+ -wal/-shm)        # SQLite (reconstruível a partir dos manifestos)
  assets/ab/cd/<asset_id>/      # shard 2 níveis (evita 10^5 entradas/pasta)
     asset.json                 # manifesto legível (fonte-da-verdade dos metadados)
     src/ model.glb | model.fbx # original preservado
     tex/<hash8>_<mapa>.ktx2|png
     asset.anigomat.json        # materiais canônicos (ver C)
  trash/<yyyymmdd>/<asset_id>/  # soft delete
<cache_dir>/anigo/library/<library_id>/   # DESCARTÁVEL
  thumbs/{64,128,256,512}/<hash2>/<blake3>-<pipeline_ver>.webp|png
  previews/ (turntable)  derived/
```
`index.db` é cache de consulta; `asset.json` é verdade → "Rebuild index" sempre possível (como Blender/DDC).

**Campos do registry (AssetData-like)**: `asset_id, library_id, kind (model|character|outfit|hair|accessory|material|texture|pose|animation|hdri|scene|prefab), name, description, author, license, source_url, content_hash(blake3), size_bytes, storage(managed|linked), path, format, importer_ver, created_at, imported_at, modified_at, rating 0-5, favorite, state(active|trashed|missing|importing|failed), catalog_id, tags[], dependencies[], stats{tris,verts,materials,bones,morphs,anims,bbox,has_skin,has_vrm}, thumb_ver, palette[5], thumbnail_camera, props(JSON)`.

**SQLite** (`rusqlite` bundled; FTS5 vem habilitado no build bundled: `-DSQLITE_ENABLE_FTS5` em libsqlite3-sys/build.rs [V]). PRAGMAs: `journal_mode=WAL; synchronous=NORMAL; foreign_keys=ON; mmap_size=256MB; temp_store=MEMORY`; `user_version` p/ migrações; 1 conexão escritora (thread dedicada) + N leitoras.
```sql
CREATE TABLE asset(id BLOB PRIMARY KEY, library_id INT, kind TEXT, name TEXT, description TEXT, author TEXT, license TEXT,
  source_url TEXT, content_hash BLOB, size INT, storage INT, path TEXT, format TEXT, state INT DEFAULT 0, rating INT DEFAULT 0,
  favorite INT DEFAULT 0, catalog_id INT, tris INT, verts INT, materials INT, bones INT, anims INT, bbox BLOB,
  imported_at INT, modified_at INT, thumb_ver INT, props TEXT) STRICT;
CREATE INDEX ix_asset_kind ON asset(kind,state,name); CREATE INDEX ix_asset_hash ON asset(content_hash);
CREATE INDEX ix_asset_cat ON asset(catalog_id,state); CREATE INDEX ix_asset_fav ON asset(favorite) WHERE favorite=1;
CREATE TABLE catalog(id INTEGER PRIMARY KEY, parent_id INT, name TEXT, path TEXT UNIQUE);   -- 'Personagens/Hero/Roupas'
CREATE TABLE tag(id INTEGER PRIMARY KEY, path TEXT UNIQUE, color TEXT);                      -- 'estilo/cel/noir'
CREATE TABLE asset_tag(asset_id BLOB, tag_id INT, PRIMARY KEY(tag_id,asset_id)) WITHOUT ROWID;
CREATE TABLE dep(asset_id BLOB, dep_id BLOB, kind TEXT, PRIMARY KEY(asset_id,dep_id)); CREATE INDEX ix_dep_rev ON dep(dep_id);
CREATE TABLE collection(id INTEGER PRIMARY KEY, name TEXT, smart INT, query TEXT);          -- smart = query JSON salva
CREATE TABLE collection_item(coll_id INT, asset_id BLOB, PRIMARY KEY(coll_id,asset_id)) WITHOUT ROWID;
CREATE TABLE root(id INTEGER PRIMARY KEY, path TEXT, mode INT, online INT, watch INT, last_scan INT);
CREATE TABLE thumb(asset_id BLOB, size INT, hash BLOB, ver INT, ok INT, PRIMARY KEY(asset_id,size)) WITHOUT ROWID; -- estado; pixels no cache em disco
CREATE TABLE job(id INTEGER PRIMARY KEY, kind TEXT, asset_id BLOB, state INT, tries INT, err TEXT);          -- fila persistente
CREATE VIRTUAL TABLE asset_fts USING fts5(name, description, tags, author, tokenize='unicode61 remove_diacritics 2', prefix='2 3');
```
Consultas típicas: (1) grade: `SELECT id,name,kind,thumb_ver,rating FROM asset WHERE state=0 AND kind=?1 AND catalog_id IN(subárvore) ORDER BY name,id LIMIT 200` com paginação *keyset* (`WHERE (name,id)>(?,?)`); (2) busca: `JOIN asset_fts ON asset_fts.rowid=asset.rowid WHERE asset_fts MATCH 'cabel* AND tags:noir' ORDER BY bm25(asset_fts)`; (3) tags hierárquicas: `tag.path LIKE 'estilo/cel/%'`; (4) coleção inteligente = JSON → WHERE parametrizado (nunca SQL cru); (5) duplicatas: `GROUP BY content_hash HAVING COUNT(*)>1`; (6) "usado por": `SELECT asset_id FROM dep WHERE dep_id=?` (avisa/bloqueia delete); (7) missing: marcar `state=2` os ausentes no último scan. Escala: SQLite+FTS5 com 10^5–10^6 linhas responde em ms [C — medir no benchmark da Fase 0]; a UI guarda só a página visível em memória; `COUNT(*)` cacheado.

**Miniaturas**
- Padrão: render com nosso renderer offscreen (cel-shading + contorno) → a miniatura mostra o look anime real (diferencial vs. Unreal/Blender). `HeadlessRenderer` em fila própria (1 job de GPU por vez, orçamento de ms/frame p/ não travar a UI); câmera auto-enquadrada pelo bbox (3/4 frontal; retrato p/ personagens); fundo transparente; master 512 → downscale com `fast_image_resize` para 256/128/64. Texturas/materiais: render em esfera/plano; imagens: resize direto.
- Placeholder por tipo imediato → miniatura troca quando pronta (`thumb.ok`).
- Cache: chave `blake3(conteúdo)+pipeline_ver+tamanho`; invalida só se hash OU `pipeline_ver` (renderer/estilo) mudar; LRU em RAM (texturas egui da janela visível ±2 telas) + disco com teto (2–5 GB, evicção por atime) + "Limpar cache"/"Reconstruir miniaturas". Um arquivo por miniatura (simples/robusto); ATLAS só em runtime na GPU (atlas dinâmico de células 128/256), não em disco (complica invalidação).
- Opcional: turntable no hover (sob demanda, cache à parte) e câmera de miniatura editável (`thumbnail_camera`, como Thumbnail Edit Mode do Unreal [S]).

**Gestão (UX completa)**: roots (adicionar/remover/offline/rescan/relink/converter managed↔linked); import por arrastar arquivos/pastas com fila, progresso, cancelamento e relatório de erros; delete = lixeira + "Esvaziar"; delete avisa/bloqueia se há dependentes; mover entre catálogos sem tocar disco; duplicatas por hash; `verify` (re-hash); exportar "pack" (.anigopack = zip + manifesto + dependências); estatísticas de disco/cache; watcher (notify) com debounce em linked roots + rescan na inicialização (watchers perdem eventos).

## C) Materiais e texturas p/ ANIME

| Formato | Estado (out/2026) | Bom para | Limite p/ anime |
|---|---|---|---|
| glTF 2.0 metal-rough + KHR_materials_* | Ratificadas: clearcoat, sheen, specular, ior, transmission, volume, iridescence, anisotropy, dispersion, emissive_strength, unlit, variants, texture_transform, texture_basisu, mesh_quantization, draco; `pbrSpecularGlossiness` ARQUIVADA; `diffuse_transmission` Release Candidate; `subsurface` Initial Draft; **não há extensão OpenPBR nem toon da Khronos**; `KHR_texture_procedurals` + `EXT_texture_procedurals_mx_1_39` (MaterialX em glTF) em Review Draft. [V] https://github.com/KhronosGroup/glTF/tree/main/extensions | interop universal e entrega | PBR ≠ cel; sem rampa/limiar/contorno |
| VRM 1.0 `VRMC_materials_mtoon` | vigente; lido/escrito por add-on VRM do Blender (atualizado 10/out/2026 [S] https://extensions.blender.org/add-ons/vrm/), UniVRM (Unity), three-vrm; crates `vrm-spec` 0.1.1, `serde_vrm` 0.4, `vrm-runtime` 0.1 [V crates.io]. Props [V]: shadeColorFactor(def [0,0,0]), shadeMultiplyTexture, shadingShiftFactor(0)/Texture, shadingToonyFactor(0.9), giEqualizationFactor(0.9), matcapFactor/Texture, parametricRimColorFactor/FresnelPowerFactor(5.0)/LiftFactor, rimMultiplyTexture, rimLightingMixFactor(1.0), outlineWidthMode/Factor/MultiplyTexture, outlineColorFactor, outlineLightingMixFactor(1.0), uvAnimation*, transparentWithZWrite, renderQueueOffsetNumber. https://github.com/vrm-c/vrm-specification/blob/master/specification/VRMC_materials_mtoon-1.0/README.md | único padrão aberto de material toon com 3+ ferramentas reais; o ANIGO já exporta | sem rampa-textura, ILM, face-SDF, máscara de especular |
| UTS3 / lilToon (Unity) | UTS3 0.13.0 (12/2025) exige Unity 6; modelo base + 1st shade + 2nd shade, highlight, rim, matcap, outline [S] https://docs.unity3d.com/Packages/com.unity.toonshader@0.13/manual/index.html ; lilToon: shader de avatar (rim, matcap, outline) [S] | referência de PARÂMETROS (2nd shade ↔ nosso `mtoon_second_shade_*`) | .mat/shader do Unity não é padrão de troca |
| MaterialX 1.39 / OpenPBR Surface 1.1 | OpenPBR 1.1 em MaterialX ≥1.39.2/USD 25.02; Maya 2026 e Cinema 4D 2026.2 o têm como padrão; Omniverse Kit 110 (mar/2026) [S] https://tac.aswf.io/meetings/2025-02-05/MaterialX_TAC_Update_2025_Final.pdf , https://help.autodesk.com/cloudhelp/2026/ENU/Maya-WhatsNew/files/GUID-2B248BA3-CC49-4695-BE16-251D5725BEE1.htm | DCC/VFX, PBR offline | modelo físico; sem toon padronizado |
| OpenUSD UsdPreviewSurface | baseline portátil de todo consumidor USD; OpenPBR (MaterialX) como alta fidelidade [S] https://nvidia.github.io/simready-foundation/2026.08.0/capabilities/visualization/materials/requirements.html | pipelines USD | idem PBR |

**Recomendação objetiva**
1. **Canônico interno = "ANIGO Toon Material" (`.anigomat.json`, `schema_ver`)**: superconjunto estruturado do `StylizedMaterial`, em blocos: `base` (cor, tex, alpha mode/cutoff), `shade` (shade1/shade2 cor+tex, shift, toony, steps OU `ramp` 1D), `ilm` (mapa por canal configurável: spec-mask / threshold-sombra / spec-size / outline-mask), `spec`, `rim` (paramétrico MToon + máscara), `matcap`, `outline` (modo, largura, mult-tex, cor, lighting-mix, depth bias), `face_sdf` (textura + vetores frente/direita + threshold), `emission`, `normal`, `uv_anim`, e `pbr_fallback` (baseColor/metal/rough) p/ exportar a mundos PBR. Cada textura declara `role` + espaço de cor + swizzle — nunca inferido do nome.
2. **Importar**: (a) glTF PBR (+emissive_strength/unlit/ior/specular/transmission/variants) → `base`, com `shade` derivado automático (multiplicar/dessaturar + `hue_shift`) e flag `converted_from=pbr`; (b) **VRM1 MToon → mapeamento 1:1 (prioridade máxima)**; (c) VRM0 (`materialProperties`) opcional [C]; (d) OBJ/MTL e FBX (ufbx) → PBR → (a).
3. **Exportar**: GLB com `pbrMetallicRoughness` de fallback + **`VRMC_materials_mtoon`** (já feito em `anigo-vrm`) → abre em Blender (add-on VRM) e Unity (UniVRM); Unreal via VRM4U [não verificado]. Recursos sem equivalente (ILM, face SDF, ramp) vão em `extras`/extensão própria `ANIGO_materials_toon` (prefixo reservável por issue no repo glTF [V]). Fase tardia: UsdPreviewSurface/MaterialX-OpenPBR só como fallback PBR.
4. **Por que não OpenPBR/MaterialX como canônico**: nenhuma norma (Khronos/USD) define toon; manteria parâmetros sem sentido para cel-shading e perderia o look. Reavaliar quando `EXT_texture_procedurals_mx_1_39` sair de Review Draft [V].

**Convenções de textura**
- Cor (baseColor, shade, emissive, matcap): **sRGB**. Dados (normal, ORM/metal-rough, ILM, máscaras, ramp-de-dados, face SDF, largura de outline): **linear/UNORM**. glTF: baseColor sRGB; metal-rough/occlusion/normal lineares [V]; KTX2: transferFunction SRGB p/ cor e LINEAR p/ não-cor [V KHR_texture_basisu].
- glTF ORM: occlusion=R, roughness=G, metalness=B (metallicRoughnessTexture: B metal, G rough) [V spec]. Normal glTF = **OpenGL (+Y para cima)** [V]; Unreal/DirectX = Y invertido [C]: guardar `normal_convention` e inverter G ao importar de DirectX.
- UV: glTF origem superior-esquerda (V para baixo) [C — conferir com `Mesh::from_glb_file`]; flip V ao importar OBJ/FBX com V para cima.
- Alpha: preferir `MASK` (cutoff) a `BLEND` p/ cabelo/cílios; `transparentWithZWrite` do MToon quando necessário [V].
- Mipmaps: cadeia completa gerada no import (cor com filtro gama-correto; máscaras sem sRGB; normal renormalizada).
- Compressão: KTX2+Basis para entrega/cache, nunca como fonte. Regra Khronos: cor → ETC1S, dados (normal, ORM, ILM, SDF) → UASTC; largura/altura múltiplas de 4; pirâmide de mips completa [V]. Para anime (gradientes suaves, ramps, SDF) ETC1S pode dar banding → permitir UASTC por textura; manter original sempre. MVP: guardar PNG/JPEG originais; KTX2 na Fase 3.
- Face SDF e ramps: sem compressão com perdas fortes, sem sRGB, filtro linear + clamp.

## D) Importadores

| Fonte | Crate (versão, licença) | Notas |
|---|---|---|
| glTF/GLB | `gltf` 1.4.1 (MIT/Apache, 05/2024) [V]: features KHR_lights_punctual, materials_{emissive_strength,ior,specular,transmission,unlit,variants,volume,pbrSpecularGlossiness}, texture_transform; NÃO decodifica Draco/meshopt/basisu | já existe parser próprio em `anigo-vrm/gltf.rs` (895 linhas): manter (controle de limites + VRM/MToon) |
| VRM | `vrm-spec` 0.1.1 (Apache-2.0), `serde_vrm` 0.4.0, `vrm-runtime` 0.1.0 (08/2026) [V crates.io] | usar só como referência/teste cruzado de `vrm1.rs` |
| FBX | `ufbx` 0.11.5 (MIT OR Unlicense; já `=0.11.5` em anigo-native) | C vendorizado, ok no sandbox |
| OBJ | `tobj` 4.0.5 (MIT, 08/2026) | MTL → PBR → canônico |
| USD | `openusd` 0.8.0 (MIT, MSRV 1.96, publicado 11/10/2026): Rust puro, usda/usdc/usdz, composição, sem C++ [V docs.rs]; schemas em `openusd-schemas` | muito novo (v0.x) → Fase 4, só leitura, feature flag |
| meshopt | `meshopt` 0.6.2 (C vendorizado, MIT/Apache); `meshoptimizer-rs` 0.2.1 (Rust puro, 10/2026); `awsm-renderer-codec-meshopt` 0.38 [V crates.io] | EXT_meshopt_compression é ratificada [V]; gltfpack é comum → prioridade média |
| Draco | sem crate maduro (`draco` 0.1.2 de 2019; `bevy_gltf_draco` 0.2.1) [V crates.io] | MVP: detectar e recusar com mensagem clara; depois cc vendorizado se houver demanda |
| KTX2/Basis | `ktx2` 0.5.0 (parser, Apache-2.0); `basis-universal` 0.3.1 (C++ vendorizado, 2023); `basisu` 0.1.0 (transcoder Rust puro, 07/2026, jovem) [V crates.io] | transcodificar para BC7/ASTC/RGBA8 conforme GPU; validar com texturas reais |

**Segurança** (arquivos de terceiros): limites configuráveis (bytes do arquivo, nº de nós/meshes/primitivas/vértices/texturas/joints, profundidade da hierarquia, dimensão máx. de imagem, tamanho de buffers; validar `accessor.count*stride <= bufferView`); resolver URIs só dentro da pasta do asset (bloquear `..`, absolutos, `file://`, `http://`); teto de descompressão (zip-bomb); import em worker com timeout e `catch_unwind`; `cargo fuzz` nos parsers próprios (header GLB, accessors, JSON VRM) + corpus Khronos glTF-Sample-Assets; erro por asset (`state=failed`) sem abortar o lote.

**Um GLB com vários meshes/hierarquia/skin/animações = quê?** Proposta (espelha Blender, que separa Object/Collection/Material/Pose como tipos [V]):
- **1 arquivo importado = 1 asset "Cena/Prefab"** (`model`, ou `character` se skin+humanoid/VRM) com manifesto interno de sub-recursos (`meshes[]`, `materials[]`, `textures[]`, `skeleton`, `animations[]`, `nodes`). Preserva o original e evita explodir 1 GLB em 40 linhas na grade.
- **Sub-assets promovíveis** ("Extrair…"): materiais → assets `material`; animações → `animation`/`pose` (retarget depende do esqueleto); meshes → `model`. Vínculo na tabela `dep` (sub-asset → pai) para delete seguro.
- Arrastar p/ a cena instancia o prefab inteiro (hierarquia+skin+materiais); Shift+arrastar abre "o que importar". Heurística de kind: VRM humanoid → `character`; skin sem humanoid → `model`+rig; só mesh → `model`; só materiais → `material`.
- Normalizar escala/eixos no import (metros, Y-up) e guardar `unit_scale`/`up_axis` originais.

## E) Backend Rust e UI egui

**Crates (versão atual no crates.io em 2026-10-12 [V], licença)**
- `rusqlite` 0.40.2 (MIT), feature `bundled` (SQLite vendorizado; FTS5 ligado [V]). Migrações: `PRAGMA user_version` + fn própria (a consulta de `rusqlite_migration` não respondeu; `refinery` 0.10.0 é overkill).
- `blake3` 1.8.7 (CC0/Apache-2.0) p/ hash de conteúdo (SIMD; feature `rayon`). `xxhash-rust` 0.8.19 é BSL-1.0 (evitar); `twox-hash` 2.1.5 p/ hash não criptográfico.
- `notify` 8.2.0 estável (CC0; 9.0.0 só RC) + `notify-debouncer-full` 0.7.0 (MIT/Apache) — o projeto já usa `notify = "8"`. Varredura: `walkdir` 2.5.0 (Unlicense/MIT) ou `jwalk` 0.9.0 (MIT, paralelo, p/ 10^5+ arquivos); `ignore` 0.4.33 p/ globs.
- Imagem: `image` 0.25.10 (já no workspace, só png/jpeg → habilitar webp/tga/exr/hdr conforme necessário); `fast_image_resize` 6.1.0 (MIT/Apache, SIMD) p/ 512→256/128/64; `ktx2` 0.5.0; `zstd` 0.14.2 (BSD-3); `ddsfile` 0.6.0 se BCn.
- Cache RAM: `lru` 0.18.5 (MIT). Concorrência: `crossbeam-channel` 0.5.17 + `rayon` 1.12.0 + threads dedicadas (DB writer, render de miniaturas, hash/import) — preferível a tokio: carga é CPU/disco e a UI egui é síncrona; tokio/`ureq`/`reqwest` só se houver provider remoto. `memmap2` 0.9.11 p/ GLB grandes.
- Sistema: `trash` 5.2.9 (MIT), `rfd` 0.17.2 (MIT), `infer` 0.22.0 (magic bytes). Busca alternativa só se FTS5 não bastar: `tantivy` 0.26.2 (MIT). `redb` 4.3.0 desnecessário.

**Padrões egui (egui/eframe 0.36.2 já no projeto)**
- Grade virtualizada: `ScrollArea::show_rows(ui, row_height, total_rows, |ui, row_range| …)` (`egui-0.36.2/src/containers/scroll_area.rs:983` [V]), `total_rows = ceil(n/colunas)`, `colunas = floor(largura/(célula+espaço))` recalculado por frame; `show_viewport` (linha 1020 [V]) se alturas variarem. Desenhar só `row_range` com `allocate_exact_size` + `painter` (sem widgets pesados). `n` vem de `COUNT(*)`; metadados da janela visível pedidos ao DB por faixa (keyset) com prefetch ±2 telas e cache de páginas.
- Texturas por visibilidade: célula entra na janela → pede miniatura ao worker (fila por prioridade: visíveis primeiro; cancelar as que saíram); resultado volta por canal → `ctx.load_texture(..., TextureOptions::LINEAR)`; mapa `asset_id → TextureHandle` em LRU (600–2000 entradas conforme tamanho) com descarte fora dele; `request_repaint()` coalescido (8–16 uploads/frame). Escolher o tamanho de cache mais próximo do zoom (64/128/256/512).
- Painéis: árvore de catálogos (virtualizar se grande), busca com debounce ~150 ms → FTS5, filtros (kind, tag, rating, favorito, formato, tris, estado), ordenação, detalhes (preview 512, metadados, tags editáveis, dependências/"usado por"), multisseleção (Ctrl/Shift), atalhos (F2, Del, Space).
- Arrastar-e-soltar p/ a cena: `Response::dnd_set_drag_payload(payload)` na célula; no viewport `dnd_hover_payload::<T>()` (fantasma/preview) e `dnd_release_payload::<T>()` (soltou → instanciar) (`response.rs:499/516/532` [V]) + `Ui::dnd_drop_zone` (`ui.rs:2694` [V]). Payload = `AssetId`+kind; viewport faz raycast p/ posição. Arquivos do SO: `ctx.input(|i| i.raw.dropped_files / hovered_files)` → fila de import.

## F) Roadmap por fases e riscos

**Fase 0 (fundação, ~1 sprint)**: crate `anigo-library` sem egui: `Registry` + SQLite (schema acima, WAL, migrações), roots managed/linked, scanner (jwalk), hash BLAKE3, import GLB/VRM com manifesto+stats; teste de integração com 50k assets sintéticos; benchmark (abrir, filtrar, FTS, paginar).
**Fase 1 (MVP útil)**: grade virtualizada + árvore de catálogos + busca FTS5 + tags + favoritos/rating + coleções estáticas; miniaturas via HeadlessRenderer em fila (128/256, cache disco + LRU); drag-and-drop p/ cena; delete=lixeira; import com fila/progresso; migrar `library.rs` (8 categorias fixas → catálogos editáveis) com adapter.
**Fase 2 (robusto)**: coleções inteligentes, watcher + rescan + `missing` + relink por hash, dedup, dependências/"usado por" + delete seguro, verify/repair, rebuild do índice, política de cache (teto, LRU disco, purge granular), packs `.anigopack`, `ANIGO Toon Material` v1 + MToon 1:1 + conversão PBR→toon.
**Fase 3 (interop/escala)**: OBJ/FBX → asset; `EXT_meshopt_compression`; `KHR_texture_basisu` (ler + gerar KTX2 de cache); export glTF+MToon+extras; sub-assets promovíveis; turntable no hover; atlas dinâmico; tags com sinônimos; estatísticas de uso.
**Fase 4 (ecossistema)**: USD (`openusd`), Draco se preciso, providers remotos (catálogo+download manager+cache, como libs online do Blender 5.2 [V]), servidor p/ multiusuário, revisões/diff, exposição via `anigo-mcp`.

**Riscos e mitigação**
1. Miniaturas competem com a GPU da viewport → fila de 1 job, throttle por ms, fallback ícone; testar wgpu 30 em máquina fraca. 2. Invalidação de cache errada → chave hash+`pipeline_ver` + testes. 3. SQLite/WAL em rede (SMB/NFS) corrompe/trava → índice sempre local; só roots linked em rede. 4. Watchers perdem eventos / limite inotify no Linux → rescan incremental (mtime+size) no start e botão manual. 5. Arquivos maliciosos → limites + fuzz + worker isolado. 6. Crates jovens (`openusd` 0.x, `basisu` 0.1, `vrm-runtime` 0.1, `meshoptimizer-rs`) → atrás de feature flags + conformidade contra sample assets, ou variantes com C vendorizado (`meshopt`, `basis-universal`); `gltf` 1.4.1 sem release desde 05/2024 → manter parser próprio/pin. 7. Sem padrão toon aberto além do MToon → canônico próprio + export MToon, documentar perdas (ILM/SDF/ramp só em `extras`). 8. Quixel/Fab: licenças restritivas e layout instável [S] → guardar `license` por asset; importador Bridge só "linked", best-effort. 9. Migração do `AssetRecord` atual (`&'static str`) → adapter, não big-bang. 10. Memória da UI (10^5 texturas) → LRU rígido e tamanhos 64/128 em grade densa.

## Pendências de verificação
Schema JSON do Megascans/Fab; nome/formato de `blender_assets.cats.txt`; suporte a VRM no Unreal (VRM4U); convenção de V/UV no `Mesh::from_glb_file`; benchmarks reais de SQLite/FTS5 a 10^6 linhas; Unity Package Manager/Billfish (sem fonte lida).
