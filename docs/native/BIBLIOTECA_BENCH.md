# Benchmark de escala da Biblioteca de ativos (T-5.10)

Backend: `crates/anigo-assets`. Medido com `examples/bench_scale.rs`, build **release**, nesta sessao.
Reproduzir:

```text
cargo run -p anigo-assets --release --example bench_scale -- 50000 /caminho/de/trabalho
```

## Ambiente (honestidade sobre o que foi medido)

- Sandbox de nuvem: Intel Xeon @ 2.10 GHz, **4 vCPUs**, 16 GiB de RAM, disco virtio (`/dev/vda`) com cache de pagina do SO
  quente (os arquivos acabaram de ser gravados). NAO e um SSD NVMe de desktop nem um HDD; leituras "frias" em disco rotativo
  serao bem mais lentas, e Windows/NTFS (Defender, indexacao) tem custo proprio de criacao de arquivos pequenos.
- Os ativos sao **sinteticos**: manifestos `asset.json` reais (um por pasta, layout `assets/ab/cd/<uuid>/`) com nomes, descricoes,
  2 a 4 tags hierarquicas + 1 tag de pacote, nota, favoritos (5 %) e ~1 % de conteudo duplicado. NAO ha arquivos de payload
  (GLB) por tras deles: por isso `verify` reporta "arquivo ausente" para todos (esperado) e nenhuma miniatura foi gerada.
- A distribuicao de palavras e artificial (20 adjetivos x 30 substantivos x 10 estilos): a seletividade das buscas FTS reais
  (vocabulario de nomes de arquivos de verdade) vai variar.
- **Variacao entre execucoes**: a mesma configuracao rodou duas vezes (50 k): gravar 50 k manifestos levou 4,4 s e 13,9 s;
  `rebuild_index` 4,7 s e 5,7 s. Trate numeros de disco como ordem de grandeza; os de consulta variam ~20 %.
- A UI (60 FPS de scroll, textura por celula) NAO foi medida: este benchmark e so do backend. Ver "O que nao foi validado".

## Resultado: 50 000 ativos (execucao final)

| Etapa | Tempo | Taxa |
|---|---|---|
| abrir biblioteca vazia | 5,5 ms | |
| gravar 50 000 `asset.json` (disco, fora da API) | 13,9 s | 3 604 arq/s |
| **`rebuild_index`** (insercao em massa: SQLite + FTS5 + tags, 1 transacao) | **5,65 s** | **8 843 ativos/s** |
| reabrir com indice pronto | 172 ms | |
| tamanho do indice (index.db + wal) | 86,9 MiB | 1 822 B/ativo |
| percorrer TODAS as paginas (keyset, 200/pag., 250 pags.) | 125 ms | mediana 0,41 ms / max 4,8 ms por pagina |
| 4 leitores concorrentes x 200 buscas FTS (limite 50) | 271 ms | 2 953 buscas/s |
| import real de 2 000 PNGs (copia + BLAKE3 + analise + indexacao) | 2,58 s | 775 arq/s |
| scan linked de 2 000 PNGs novos | 1,24 s | 1 618 arq/s |
| scan linked sem mudancas (pula por mtime+tamanho) | 30 ms | 2 000 pulados, 0 reprocessados |
| `verify` sem hash (existencia+tamanho) de 54 000 ativos | 1,09 s | |

Consultas (ms): mediana / p95 / max. Todas retornam a primeira pagina de 200 itens, salvo indicacao.

| Consulta | mediana | p95 | max |
|---|---|---|---|
| `COUNT(*)` sem filtro | 1,08 | 2,30 | 2,60 |
| pagina 1 (nome) | 0,29 | 0,36 | 0,39 |
| pagina 1 + total | 1,40 | 1,80 | 2,06 |
| pagina PROFUNDA (keyset, ~50 % da lista) | 0,42 | 0,55 | 0,58 |
| tipo=hair, ordenado por nota desc | 1,03 | 1,26 | 2,41 |
| favoritos | 2,49 | 3,49 | 4,10 |
| ordenar por tamanho desc | 0,26 | 0,32 | 0,38 |
| FTS `cabelo` (prefixo) | 1,85 | 2,86 | 3,23 |
| FTS `cabelo longo` (2 termos) | 1,65 | 2,02 | 2,08 |
| FTS `cabelo longo` + total (623 resultados) | 4,57 | 7,14 | 7,22 |
| FTS `cab` (prefixo curto) | 1,68 | 2,01 | 2,52 |
| FTS termo raro | 2,74 | 3,15 | 3,26 |
| FTS `tag:noir -vestido` | 2,40 | 3,70 | 4,36 |
| FTS ordenado por **relevancia** `cabelo` (~3 % dos ativos) | 12,9 | 15,6 | 15,6 |
| FTS ordenado por **relevancia** `anime` (~10 % dos ativos) | 11,0 | 12,0 | 12,0 |
| tag hierarquica `estilo/cel` | 1,34 | 1,47 | 1,56 |
| tags `estilo` E `categoria/roupa` | 3,62 | 5,68 | 14,35 |
| combinada: texto + tipo + tag + nota >= 3 | 10,1 | 12,2 | 16,2 |
| `tags()` com contagens (67 tags) | 73,0 | 92,3 | 92,3 |
| colecao manual (5 000 itens) | 1,43 | 1,59 | 1,80 |
| colecao inteligente | 4,81 | 5,22 | 5,65 |
| `find_duplicates` (limite 1 000; 489 grupos) | 53,3 | 55,4 | 55,4 |
| `set_favorite` (reescreve `asset.json` + SQL) | 0,73 | 1,22 | 3,67 |
| `add_tags` (reescreve `asset.json` + reindexa) | 1,30 | 2,03 | 25,7 |
| `trash` + `restore` de 1 ativo | 3,42 | 4,97 | 24,8 |

Memoria do processo (VmRSS): 8 MiB com a biblioteca aberta e vazia; 57 MiB (pico 57) apos reconstruir o indice de 50 k;
220 MiB ao final de todo o benchmark (pico 249). Esse valor final INCLUI paginas do `index.db` mapeadas em memoria
(`mmap_size` = 128 MiB, reclamaveis pelo SO), caches de pagina do SQLite (32 MiB escritora + 8 MiB por leitora, ate 6 leitoras) e
as estruturas do proprio benchmark. A UI so precisa guardar a pagina visivel.

## Resultado: 200 000 ativos (4x, para ver a curva)

| Etapa | Tempo | Taxa |
|---|---|---|
| `rebuild_index` (200 000 ativos) | 23,0 s | 8 705 ativos/s |
| reabrir com indice pronto | 375 ms | |
| tamanho do indice | 352 MiB | 1 847 B/ativo |
| percorrer TODAS as paginas (1 000 pags.) | 622 ms | mediana 0,56 ms / max 5,0 ms |
| `verify` sem hash de 204 000 ativos | 3,97 s | |
| memoria apos `rebuild_index` | 113 MiB | pico 113 MiB |

| Consulta | mediana (ms) | p95 | max |
|---|---|---|---|
| `COUNT(*)` | 4,16 | 6,97 | 7,13 |
| pagina 1 (nome) | 0,46 | 0,54 | 0,54 |
| pagina PROFUNDA (keyset) | 0,29 | 0,42 | 0,54 |
| FTS `cabelo` | 5,65 | 12,1 | 12,7 |
| FTS `cabelo longo` | 3,83 | 5,39 | 5,48 |
| FTS termo raro | 9,52 | 16,5 | 18,1 |
| FTS relevancia `cabelo` | 47,6 | 65,2 | 65,2 |
| combinada texto + tipo + tag + nota | **55,8** | 93,5 | 94,3 |
| `tags()` com contagens | 556 | 695 | 695 |
| `find_duplicates` (limite 1 000) | 291 | 324 | 324 |

## Leitura honesta dos numeros

- **Meta "busca < 50 ms" (T-5.10)**: atendida a 50 k com folga (pior caso medido: 15,6 ms, FTS por relevancia; combinada 10 ms).
  A 200 k ela e atendida para tudo, exceto a consulta combinada (55,8 ms mediana) e a relevancia de termo comum (47,6 ms).
- **Paginacao keyset e O(pagina)**: a pagina profunda custa o mesmo que a primeira (0,3 a 0,5 ms) em 50 k e em 200 k.
- **O que cresce com o tamanho**: `COUNT(*)` (varre o indice; 4 ms a 200 k - cacheie o total na UI), FTS por relevancia (materializa
  todos os casamentos do `bm25` a cada pagina; a ordenacao por nome nao paga isso), `tags()` (agrega todas as ligacoes
  tag-ativo; 73 ms a 50 k, 556 ms a 200 k - chame ao abrir o painel/ao editar tags, nao a cada quadro) e `find_duplicates`.
- **Consulta combinada a 200 k**: o plano do SQLite varre a ordem de nome e testa cada candidato quando o filtro e muito
  seletivo. Otimizacao conhecida e NAO implementada: quando o texto casar poucas linhas, materializar o FTS primeiro e ordenar so
  esse conjunto.
- **Reconstruir o indice** (recuperacao de desastre) custa ~9 k ativos/s e e o gargalo de insercao em massa; em uso normal os
  ativos entram pelo `import` (775 a 1 600 arq/s com arquivos pequenos aqui; para GLB reais o custo e dominado por copia + BLAKE3).
- **Escrita por ativo** (favorito, tag, lixeira) e de ~1 a 4 ms mediana, mas com cauda de ~25 ms (fsync/`rename` no disco).
  Para editar centenas de ativos de uma vez a UI deve usar as variantes em lote (`add_tags_many`) ou enfileirar.

## O que este benchmark NAO mediu

- Scroll a 60 FPS, upload de texturas e preenchimento do cache de miniaturas (a UI egui ainda nao existe).
- Geracao de miniaturas 3D (renderer cel/GPU) em lote.
- Disco rotativo, NAS/SMB/NFS (o `index.db` nao deve ficar em rede; ver `OpenOptions::index_dir`), Windows/macOS.
- Frio de cache de pagina do SO (reabrir apos reiniciar a maquina), bibliotecas de 10^6 ativos, FTS com vocabulario real.
- Memoria total do app (so a do processo do benchmark).
