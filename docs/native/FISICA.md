# ANIGO - Física de cabelo, roupa e acessórios (`anigo-physics`)

Estado: núcleo implementado e testado **sem GPU e sem olho humano** (F3 de `pesquisa/personagem_rig_cabelo_roupa.md`). Tudo que depende de "sensação" visual está listado em "O que NÃO foi validado" e em "Validação local".

Todos os números abaixo foram medidos nesta sessão (`cargo test`, `cargo run --release -p anigo-physics --example hair_skirt [-- bench]`), CPU única thread, build release. Nada é estimativa, exceto onde marcado.

## 1. Mapa dos crates

| Crate | Papel |
|---|---|
| `anigo-physics` (novo) | Solver puro (`glam`, `serde`; sem GPU, sem Bevy, sem rapier): spring bone VRM-exato, núcleo XPBD, cabelo, roupa por proxy, vínculos, colliders do corpo, `PhysicsWorld`, presets. |
| `anigo-hair` | Autoria: `chain_from_curve` (curva de `anigo-modeling::curve` vira `ChainGuide`), `hair_group_from_curves`; reexporta o solver. Sem dependência cíclica: `anigo-physics` não conhece `anigo-modeling`. |
| `anigo-cloth` | Autoria: máscaras de vértices fixos, geradores de saia/capa, malha antiga com `stiffness` convertida para compliance; reexporta o solver. |
| `anigo-vrm` | `VRMC_springBone` no schema **oficial 1.0** (parser, exportador, `extended_collider`), leitura tolerante do formato legado, e `spring_bridge` (`Vrm1SpringBone` e `SpringBoneDef` nos dois sentidos). |

Espelho TS (`src/services/vrm/*`) atualizado igual; `tests/character/vrm_interop.test.ts` cobre ida e volta, schema, legado e malformado.

## 2. VRMC_springBone (passo 0)

O formato anterior (`secondaryRig.groups[].joints[].springStiffness`, `colliders[].spheres[]/capsules[]`) **não era o da spec** e a extensão ia em `extensionsRequired`: nenhum player VRM animava o cabelo de um export do ANIGO. Agora:

- Emite `specVersion`, `colliders[].shape{sphere|capsule}`, `colliderGroups[].colliders`, `springs[].{name,joints[],colliderGroups,center}`, joints com `node, hitRadius, stiffness, gravityPower, gravityDir, dragForce` (defaults da spec: 0, 1, 0, [0,-1,0], 0.5).
- `VRMC_springBone` só em `extensionsUsed`. Colliders `inside` e `plane` usam `VRMC_springBone_extended_collider` (também só em `extensionsUsed`) **com collider de fallback na spec base** (esfera de raio 0 longe, ou esfera de raio 1000 que aproxima o plano), como a spec recomenda.
- Parser: erros estáveis `BAD_SPRINGBONE` para tipos/faixas inválidos; **avisos** (e descarte da referência) para nó fora da faixa, `colliders`/`colliderGroups` inexistentes, `joints < 2`, joint que não é descendente da anterior, joint duplicada em duas cadeias.
- Legado (`secondaryRig`, ou `VRMC_vrm.secondary`) ainda é lido: `springStiffness` vira `stiffness`, `springDamping` vira `dragForce`, ids livres de `group` viram índices densos de `colliderGroups` (ordem crescente de id), `center` (que era uma junta) vira o nó. Aviso "legado" na leitura; reexportar grava o formato novo. A `distance` legada não tem equivalente e se perde.
- Testes: ida e volta (Rust e TS), validador mínimo do JSON-schema oficial (chaves permitidas, tipos, `minItems 2`, faixas), 13 entradas malformadas por linguagem, determinismo byte a byte, arquivo `.vrm` legado reaberto e reexportado.

Não verificado: abrir o export em UniVRM/three-vrm/VSeeFace (ver "Validação local").

## 3. Algoritmos

### 3.1 Spring bone VRM-exato (`spring_bone.rs`)

Fórmulas literais da spec (seção "SpringBone Algorithm"): `inertia = (cur - prev)(1 - drag)`, `stiffness = dt * parentRot * initialLocalRot * boneAxis * stiffness`, `external = dt * gravityDir * gravityPower`, projeção no comprimento do osso, colisão (`nextTail -= dir * dist` quando `dist < 0`, reprojeta), e `rotation = initialLocalRot * fromTo(boneAxis, to)`.

- Ordem: as rotações das juntas voltam ao inicial, FK completa, colliders no mundo, depois cada par `(joint[i], joint[i+1])` por profundidade do nó (raiz antes de folha, inclusive entre cadeias); após cada rotação só a subárvore é recalculada.
- `center`: tails guardados no espaço do nó `center`; a inércia sai da diferença nesse espaço, gravidade fica no mundo.
- Colliders: esfera, cápsula, "inside" de ambos e plano (fórmulas da spec e do extended collider). `hitRadius` da junta-cabeça.
- Propriedade da spec: gravidade e rigidez são "deslocamento por segundo somado por passo" e o drag é por passo, logo o comportamento depende do passo. O `PhysicsWorld` roda sempre em **passo fixo de 1/60 s** (acumulador). Pêndulo livre: período `2 pi sqrt(L dt / g)`, ou seja, gravidade efetiva `g/dt`.
- Robustez: índice inválido na construção é `Err`; par não descendente ou de comprimento zero é pulado com aviso; estado não finito reinicia só aquela junta; `dt` inválido é ignorado; `dt` enorme (até 1e8) preserva o comprimento do osso.

### 3.2 Núcleo XPBD (`xpbd.rs`)

Macklin et al. 2016 / "Small Steps" 2019: restrições com **compliance** `alpha` (m/N), `alpha~ = alpha / h^2`, `dlambda = (-C - alpha~ lambda)/(w_sum + alpha~)`, sub-passos com 1 iteração, `lambda` zerado a cada sub-passo. Partículas fixas (`inv_mass = 0`) seguem o alvo animado interpolado dentro do passo.

Restrições: distância (estrutural/cisalhamento/flexão), **direção** (`x_filho - x_pai = alvo_filho - alvo_pai`: restauração de ângulo independente da translação do corpo), anexo (restauração de forma), **tether** (distância mín./máx. a uma âncora), **limites por partícula** (max-distance ao alvo + backstop pelo plano da normal animada, estilo Magica Cloth 2), e colisão com `ColliderSet` (esfera, cápsula, plano, inside; atrito de Coulomb simplificado; descarte por AABB). Limites e colisão rodam depois da última iteração do sub-passo (última palavra).

- Amortecimento exponencial `v *= exp(-damping h)` global e por partícula; velocidade máxima; vento uniforme; gravidade configurável.
- Sub-passos adaptativos: se `dt / substeps > 1/120 s` o número de sub-passos sobe (até 64) e o tempo que não couber é descartado (câmera lenta estável em vez de explosão).
- Teleporte: partícula fixa que salta mais de 1,5 m num passo reinicia o sistema no alvo (sem arrastar o cabelo/roupa pelo mapa).
- NaN/infinito: restaura o último quadro bom e conta (`nan_restores`).

### 3.3 Cabelo (`hair.rs`)

Cada fio é uma `ChainGuide` (pontos no espaço local do osso-raiz). Restrições: distância (compliance 1e-7, não estica), flexão (distância pulando uma partícula), direção com **rigidez decrescente raiz -> ponta** (`stiffness_root`, `stiffness_tip`, curva `t^power`; mapeamento artístico 0..1 -> compliance `1e-2 (1-s)^2`), tether de alongamento total, `fixed_count` partículas rígidas na raiz (equivalente ao "ponto fixo" do VRoid). Colisão das partículas (raio) com a cabeça/ombros/tronco. Saídas: posições, **rotações swing por segmento** (`segment_swings`: premultiplicar na rotação de mundo animada do osso), e `ChainFrames` (transporte paralelo) para deformar cards: `bind_to_chains`/`apply_chain_bindings` prendem cada vértice do card ao ponto mais próximo da cadeia + offset num frame local (reproduz o repouso com erro < 2e-4 m e preserva a largura do card numa dobra).

### 3.4 Roupa (`cloth.rs`, `binding.rs`)

Proxy pequena (centenas de vértices) com vértices **fixos** (skinning 4 influências a partir dos ossos) e móveis. `depth` 0..1 = distância geodésica (Dijkstra multi-fonte) ao fixo mais próximo / máxima. Restrições: distância estrutural, flexão (vértices opostos de triângulos vizinhos), **angle restoration** pai->filho na árvore geodésica com rigidez decrescente com o depth, **tether** (máx. `(1+0.05) * geodésica`, mín. `0.8 * distância de repouso`), max-distance por depth, backstop, colisão. Geradores `skirt_proxy` (cônica, normais para fora) e `cape_proxy`.

Transferência proxy -> malha de render: cada vértice guarda 3 vértices da proxy + baricêntricas + offset no frame do triângulo (modelo `.mhclo` do MakeHuman). `O(V*T)` na criação (proxy pequena). Erro no repouso 6.7e-8 m; acompanha movimento rígido e escala.

### 3.5 Colliders do corpo (`body.rs`)

20 cápsulas (cabeça, pescoço, 3 do tronco, quadril, linha dos ombros, braços, antebraços, mãos, coxas, canelas, pés) presas a ossos dos 24 canônicos (índices em `body::bone`, conferidos nome a nome contra `BondSyncManager` por teste). `radius_scale` (slider de corpo) escala os raios. Raios padrão são para ~1,65 m e **não foram ajustados à malha base final** (que ainda é placeholder).

### 3.6 `PhysicsWorld` (`world.rs`)

```rust
let mut world = PhysicsWorld::new(WorldConfig::default());      // 60 Hz, max_dt 0.1, max 8 passos
world.add_body_colliders(&rest_bones, 1.0, 0.2)?;               // cápsulas nos 24 ossos
let hair  = world.add_hair(&hair_def, &rest_bones)?;            // HairHandle
let skirt = world.add_cloth(&skirt_def, &rest_bones)?;          // ClothHandle
let acc   = world.add_accessory(&ribbon_def, &rest_bones)?;     // AccessoryHandle
let rig   = world.add_spring_rig(&spring_def)?;                 // SpringHandle (VRM-exato)
let report = world.step(dt, &bone_world);                       // StepReport; nunca entra em pânico
world.hair_positions(hair); world.hair_segment_swings(hair, chain);
world.cloth_positions(skirt); world.cloth(skirt)?.render_positions(&rest_render_vertices);
world.set_paused(true); world.reset(&bone_world); world.set_wind(v);
let setup = PhysicsSetup { .. }; let json = setup.to_json()?;   // serde do projeto
let world = PhysicsWorld::from_setup(&PhysicsSetup::from_json(&json)?, &rest_bones)?;
```

- Passo fixo com acumulador: 30, 60 e 144 fps dão o mesmo número de passos (+-1) em 2 s; `dt <= 0`/NaN é ignorado e reportado (`invalid_dt`); `dt` acima de `max_dt` é limitado (`clamped`).
- Pose com NaN mantém a última pose boa (`invalid_bones`). Salto grande da raiz vira teleporte automático (`teleports`); `reset` reinicia tudo na pose dada sem velocidades.
- `PhysicsSetup` (serde, `format_version = 1`): colliders, grupos de cabelo, acessórios, roupa, rigs VRM; limites infinitos viajam como `null`. JSON de versão futura é rejeitado; `{}` usa defaults.
- Presets (`PhysicsPreset`: `hair_short`, `hair_long`, `skirt`, `cape`, `accessory`).

## 4. Limites conhecidos

- **Sem auto-colisão** (nem partícula-partícula, nem camada sobre camada). Passo 8 de F3, deixado para depois.
- Colisão é **vértice (partícula) contra collider**. Numa proxy grossa uma aresta longa pode cruzar uma cápsula fina entre dois vértices; a proxy de saia usada nos testes tem aresta de 5 a 7 cm contra cápsulas de raio >= 7,5 cm, e nesse regime não houve cruzamento medido.
- Colliders do corpo são cápsulas rígidas por osso (sem elipse, sem SDF do corpo morfado). Gordo/magro só escala o raio (`radius_scale`); não há ainda ligação ao esqueleto morfado (passo 6 de F3).
- Spring bone: escala não uniforme do nó é aproximada (raio do collider usa o maior eixo). Cadeias ramificadas seguem a spec (tratadas como cadeias separadas).
- A apresentação roda a 60 Hz de simulação: em monitor de 144 Hz o cabelo se atualiza a 60 Hz (`PhysicsWorld::alpha()` existe, mas a interpolação de quadro **não** está implementada).
- Dentro de um quadro com vários passos fixos, a pose dos ossos é aplicada uma vez (o primeiro passo interpola o movimento nos seus sub-passos, os seguintes seguram a pose).
- XPBD é dissipativo: pêndulo rígido perde 6,1 % da energia em 4 s (20 sub-passos) e nunca ganha.
- Teleporte é detectado por salto de partícula fixa > 1,5 m/passo: um personagem que realmente corre a mais de 90 m/s dispararia o reset (não é um caso real).
- Parâmetros de preset **não foram calibrados por olho**.

## 5. Números medidos

### 5.1 Corretude (testes)

| Teste | Resultado |
|---|---|
| Pêndulo livre do spring bone, período vs `2 pi sqrt(L dt/g)` | 0,5799 s medido vs 0,5736 s: erro 1,10 % |
| Equilíbrio sob gravidade (`dir = normalize(s e + g gd)`), 3 combinações | desvio da posição < 3e-7 m |
| Spring bone, animação violenta do root (1200 passos, 6 ossos) | erro máx. de comprimento do osso 1,0e-7 m; FK vs tail simulado 4,6e-7 m |
| Spring bone, colisão com esfera (hit radius 2 cm) | penetração máx. 0,14 mm; folga final -0,09 mm; sem colisor o osso cai 13 cm a mais |
| Spring bone, `center` com personagem andando 2 m | desvio 0,80 m sem center, 0 com center |
| Spring bone, `dt` de 0,1 s a 1e8 s | sem NaN, comprimento do osso mantido (< 1e-3) |
| XPBD, queda livre 1 s | 4,947 m vs 4,905 m teórico (0,85 %; erro de quantização f32 da velocidade-por-posição) |
| XPBD, alongamento estático de corda pendurada vs `alpha m g` (substeps/iterações 10/1, 10/4, 20/1, 40/1) | erro relativo máx. 0,81 %, 0,72 %, 0,62 %, 2,30 %; as 4 configurações concordam entre si em < 4 % |
| XPBD, corda rígida (`alpha = 0`) | erro de distância máx. 0,24 mm em 10 partículas |
| XPBD, pêndulo rígido sem amortecimento | ganho de energia 0; perda 6,1 % em 4 s |
| XPBD, 9 partículas sobre esfera e plano | penetração 0 |
| XPBD, `dt` de 0,1 s a 100 s | sem NaN, posições limitadas |
| Cabelo, rigidez decrescente | desvio ao alvo por partícula: 0, 6, 25, 57, 98, 151, 215, 283 mm (raiz -> ponta) |
| Cabelo na cabeça (esfera) com forte rotação | com colisor: penetração 0, estiramento 1,6 %; sem colisor (controle): penetração 18,7 mm |
| Capa, 20 s (anda 2 s e para) | energia cinética final 8,6e-7 J, estiramento 0,37 % |
| Saia x perna chutando 30 graus | com colisão: penetração 0, estiramento 2,7 %; controle sem colisão: penetração 63,7 mm |
| Transferência proxy -> render | erro no repouso 6,7e-8 m |

### 5.2 Integração com o esqueleto canônico (`tests/canonical_skeleton.rs`, `--example hair_skirt`)

Quadril gira +-40 graus, pescoço e cabeça viram/acenam, coxas chutam até 51 graus, 12 s. Cabelo longo (18 fios x 12 pontos) e saia justa (24 x 8, hem 0,27 m) contra as 20 cápsulas do corpo:

| Métrica | Com colliders | Sem colliders (controle) |
|---|---|---|
| Penetração máx. do cabelo | 0,00 mm | 95,3 mm |
| Penetração máx. da saia | 0,00 mm | 68,5 mm |
| Estiramento máx. do cabelo | 2,98 % | |
| Estiramento máx. da saia (pior quadro, coxa a 51 graus numa saia justa) | 8,6 % | |

Teleporte do personagem inteiro (40 m): `teleports >= 1`, cabelo e saia terminam presos à cintura/cabeça novas sem esticar. Estado final idêntico em duas execuções (hash).

### 5.3 Desempenho (release, 1 thread, inclui a FK dos 24 ossos e a animação; `--example hair_skirt -- bench`)

| Cenário | Partículas | ms por passo (60 Hz) | partículas/ms |
|---|---|---|---|
| cabelo curto (~50 fios x 6), com colisores | 324 | 0,275 | 1180 |
| cabelo longo (~50 fios x 12), com colisores | 648 | 0,789 | 822 |
| cabelo longo, sem colisores | 648 | 0,277 | 2341 |
| saia 24 x 8, com colisores | 216 | 0,389 | 555 |
| saia 48 x 16, com colisores | 816 | 1,471 | 555 |
| cena completa (cabelo 648 + saia 216) | 864 | 1,094 | 790 |
| capa 16 x 16 | 272 | 0,205 | 1329 |
| spring bone VRM, 200 cadeias x 6 ossos | 1200 juntas | 0,398 | 3016 juntas/ms |

A cena do exemplo (432 partículas, 18 fios + saia) mede 0,61 ms por passo em média. Os tempos variam 20 a 40 % entre execuções na VM de nuvem usada (o custo dominante é a colisão com as 20 cápsulas; há descarte por AABB e a colisão roda uma vez por sub-passo).

Orçamento de 16,7 ms por quadro: a cena completa usa ~7 %. Medido numa máquina de nuvem, **não** numa máquina de usuário: refazer localmente.

## 6. O que NÃO foi validado visualmente

- A **sensação** de balanço (rigidez, amortecimento, gravidade) de todos os presets; os valores são pontos de partida coerentes com as faixas do VRoid e com a escala de compliance, validados só por estabilidade/não esticar/não atravessar.
- Naturalidade do cabelo em movimento, forma da saia/capa em pose, dobras, "gordura" de pano.
- Se os raios das cápsulas do corpo combinam com a malha base (ainda placeholder paramétrico) e com os sliders de corpo.
- Interoperabilidade real: nenhum player VRM (UniVRM, three-vrm, VSeeFace, `vrm-runtime`) foi usado para abrir o export. Só schema e ida e volta próprios.
- Aparência no renderer: nada disso foi ligado ao render (ver seção 7).

## 7. Integração com o renderer (nada foi alterado; contrato congelado)

Estado: a paleta de skinning é `array<mat4x4<f32>, 24>` num **uniform** (1536 B, `render_contract_v1.json`, `BonePaletteUniform`, `MAX_PALETTE_JOINTS = 24`). Decisão do usuário: não mudar a paleta de 24 ossos nem o `render_contract`. Por isso `anigo-physics` produz **posições de partícula e de vértice**, não ossos extras no render.

Caminho A, sem mudar o contrato (é o que o código já suporta): cabelo e roupa como malhas separadas com vertex buffer dinâmico. Por quadro: `world.step` -> `ClothSim::render_positions` (vértices de roupa) e `bind_to_chains/apply_chain_bindings` (vértices de cards) -> reescrever as posições no vertex buffer (layout de 72 B, joints = 0, pesos 1 no osso 0) -> recomputar normais (`vertex_normals` para a proxy; normais customizadas de card ficam do `hair_card`). Custo CPU: ~1 ms de simulação + cópia de alguns milhares de vértices.

Caminho B, para cabelo/saia por skinning de GPU com ossos secundários (pesquisa F1.4), exige mudar o contrato:
1. `bones` de `uniform array<mat4x4<f32>, 24>` para `var<storage, read> bones: array<mat4x4<f32>>` (N <= 256 por personagem no MVP); `skinning_palette_bytes` deixa de ser constante e o `render_contract_v1.json`/`render_contract_data.v1.ts` e os fixtures (`npm run fixtures:check`) precisam ser regenerados;
2. `BonePaletteUniform` vira um buffer de tamanho variável (`MAX_PALETTE_JOINTS` sobe ou some); o bind group ganha `min_binding_size` dinâmico;
3. Os shaders (bloco de skinning idêntico entre os WGSL, marcadores do contrato) trocam o acesso; `JOINTS_0` já é `u16x4`, então o layout de vértice de 72 B **não muda**;
4. `anigo-physics` já entrega o necessário: `segment_swings` por segmento de fio e matrizes de osso derivadas das partículas.

## 8. Decisões em aberto

1. Caminho A (vertex buffer dinâmico) ou B (storage buffer de N ossos)? Muda o contrato congelado.
2. Faixa final do slider de idade e do `radius_scale` por slider (raios dos colliders seguem o corpo morfado?).
3. Cloth de malha no export VRM: converter para cadeias de osso (`BoneCloth`, perde as restrições laterais) ou deixar fora do `.vrm`. Hoje só cadeias/colliders vão para `VRMC_springBone`.
4. Auto-colisão (hash espacial): vale o custo na saia longa?
5. Interpolação de apresentação (`alpha`) para monitores > 60 Hz.
6. Fonte de vento (global, por cena, por zona).
7. Ferramenta de cabelo (D/F4): `ChainGuide` e `hair_group_from_curves` já existem; falta o grupo VRoid-like (desenhar sobre a cabeça, auto-gerar grupos, recalcular ossos ao editar).

## 9. Validação local (checklist)

1. Abrir um `.vrm` exportado pelo ANIGO com cadeias de cabelo em UniVRM ou three-vrm e em VSeeFace; comparar a trajetória do tail com `SpringBoneSystem` para a mesma animação do root (tolerância visual).
2. Ajustar `PhysicsPreset::{HairShort, HairLong, Skirt, Cape, Accessory}` olhando o personagem em movimento; gravar os valores finais no preset.
3. Conferir os raios das cápsulas do corpo contra a malha base real e contra os extremos dos sliders de gordura/magreza.
4. Medir `ms/passo` na máquina do usuário (`cargo run --release -p anigo-physics --example hair_skirt -- bench`).
5. Em pose de braço levantado, perna para trás e agachamento: procurar aresta de saia cruzando a perna (limite "colisão por vértice").
6. Decidir A ou B (seção 7) antes de ligar ao render.

## 10. Como rodar

```
cargo test -p anigo-physics -p anigo-vrm -p anigo-cloth -p anigo-hair
cargo run --release -p anigo-physics --example hair_skirt            # métricas da cena
cargo run --release -p anigo-physics --example hair_skirt -- bench   # partículas/ms
cargo run --release -p anigo-physics --example hair_skirt -- json    # PhysicsSetup de exemplo
```
