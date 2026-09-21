# ANIGO — Arquitetura Canônica e Diretrizes de Implementação

**Status:** Diretriz arquitetural aprovada  
**Versão:** 1.0  
**Data:** 2026-09-20  
**Escopo:** Núcleo do projeto, deformação, renderização, viewport, exportação, render headless, persistência, undo/redo e shaders.

---

## 1. Decisão arquitetural

O ANIGO continuará utilizando a seguinte arquitetura:

```text
┌─────────────────────────────────────────────────────────────┐
│ Tauri + Svelte/TypeScript                                   │
│ Interface, seleção, interação e apresentação                │
└──────────────────────────────┬──────────────────────────────┘
                               │ comandos tipados / eventos
┌──────────────────────────────▼──────────────────────────────┐
│ Estado canônico do projeto                                  │
│ ProjectState versionado + validação + migrações             │
└──────────────────────────────┬──────────────────────────────┘
                               │
┌──────────────────────────────▼──────────────────────────────┐
│ Núcleo Rust                                                  │
│ dados, morphs, deformação, skinning, cena, materiais,       │
│ animação, comandos transacionais e serialização             │
└──────────────────────────────┬──────────────────────────────┘
                               │ buffers e parâmetros canônicos
┌──────────────────────────────▼──────────────────────────────┐
│ Renderer canônico wgpu                                       │
│ passes, shaders, câmeras, iluminação e color management     │
└──────────────┬───────────────────────┬──────────────────────┘
               │                       │
       Viewport interativo       Exportação/render headless
```

Esta decisão substitui a abordagem de múltiplas implementações independentes de personagem e renderização.

O ANIGO **não deve criar uma engine paralela completa sem necessidade**, mas deve possuir um núcleo especializado em personagens anime, renderização NPR, edição paramétrica e interoperabilidade VRM.

---

## 2. Regras invioláveis

### 2.1 Fonte única de verdade

O estado autoritativo deve existir no núcleo Rust, representado por um `ProjectState` versionado.

A UI pode manter somente estado transitório de apresentação, como:

- ferramenta selecionada;
- painel aberto;
- item atualmente focado;
- estado de hover;
- posição temporária do cursor;
- filtro de busca;
- seleção visual ainda não confirmada.

A UI não pode ser fonte de verdade para:

- geometria;
- morphs;
- pesos de skinning;
- transforms persistentes;
- materiais;
- parâmetros de shading;
- luzes;
- câmeras de projeto;
- animações;
- assets referenciados;
- dados de exportação.

### 2.2 Uma implementação de deformação

A deformação deve ser implementada uma única vez no núcleo Rust e utilizada por:

- viewport;
- render headless;
- exportação;
- testes de regressão;
- geração de thumbnails;
- ferramentas MCP.

É proibido manter uma segunda implementação equivalente em TypeScript.

O TypeScript pode enviar comandos e receber snapshots, mas não deve recalcular a malha por conta própria.

### 2.3 Renderização determinística

O viewport e o render final devem consumir a mesma descrição de cena e o mesmo renderer canônico.

Devem ser idênticos entre os caminhos:

- shaders;
- buffers;
- layout de vértices;
- parâmetros de materiais;
- parâmetros de luz;
- câmeras;
- transformações;
- color management;
- ordem dos passes;
- resolução lógica;
- regras de alpha e blending;
- anti-aliasing, quando configurado.

A regra de aceitação é:

> Se o usuário vê uma coisa no viewport, o render final deve produzir a mesma coisa, salvo diferenças explicitamente documentadas de resolução ou apresentação.

### 2.4 Uma fonte canônica de shaders

Não deve haver shaders duplicados ou divergentes em:

- template literal TypeScript;
- arquivos WGSL duplicados;
- implementação independente no Rust;
- GLSL fallback sem paridade;
- variante específica para exportação;
- variante específica para headless.

A fonte canônica deve estar em um diretório de shaders versionado e ser consumida pelo renderer. Se for necessário gerar variantes, elas devem ser produzidas por uma ferramenta determinística e testável.

Qualquer backend alternativo deve:

1. possuir contrato de paridade documentado;
2. ser coberto por testes equivalentes;
3. não alterar a semântica visual dos parâmetros;
4. emitir erro observável caso não consiga suportar um recurso.

### 2.5 Operações transacionais

Toda alteração persistente deve ser executada por um comando tipado:

```text
Command
 ├── validate()
 ├── apply(&mut ProjectState)
 ├── undo(&mut ProjectState)
 ├── serialize()
 └── metadata()
```

O comando deve ser a unidade de:

- alteração de estado;
- undo;
- redo;
- autosave;
- log de operação;
- sincronização com a UI;
- teste automatizado.

Não é permitido alterar diretamente estruturas persistentes a partir de bindings da UI.

---

## 3. Modelo canônico do projeto

O formato de projeto deve ser versionado desde a primeira versão estável:

```json
{
  "schema_version": 1,
  "project_id": "uuid",
  "character": {},
  "scene": {},
  "materials": {},
  "animation": {},
  "render": {},
  "assets": {},
  "settings": {}
}
```

### 3.1 Requisitos do formato

O formato deve possuir:

- `schema_version` obrigatório;
- identificadores estáveis para entidades;
- referências por ID, não por posição de array;
- validação antes de carregar;
- migrações explícitas entre versões;
- rejeição segura de dados inválidos;
- preservação de campos desconhecidos quando possível;
- nenhuma dependência de estado da interface;
- compatibilidade com autosave e recuperação;
- possibilidade de serialização determinística para testes.

### 3.2 Separação entre projeto e cache

O projeto deve conter apenas dados autoritativos e referências necessárias. Dados derivados podem ser armazenados em cache, mas precisam ser reconstruíveis:

```text
Projeto autoritativo
  ├── ProjectState
  ├── assets e referências
  └── parâmetros persistentes

Cache derivado
  ├── buffers GPU
  ├── thumbnails
  ├── meshes deformadas
  ├── índices auxiliares
  └── resultados temporários de IA
```

A perda do cache não pode causar perda do trabalho do usuário.

---

## 4. Responsabilidades por camada

### 4.1 Svelte/TypeScript

Responsável por:

- interface;
- seleção;
- interação do usuário;
- comandos tipados;
- apresentação de snapshots;
- validação superficial de formulário;
- envio de parâmetros ao núcleo;
- feedback de progresso e erro.

Não deve:

- deformar a malha canônica;
- manter uma cópia autoritativa de morphs;
- possuir shaders de produção;
- fazer render final alternativo;
- decidir semântica de exportação;
- alterar o projeto sem comando.

### 4.2 Núcleo Rust

Responsável por:

- `ProjectState`;
- schema e migrações;
- catálogo de morphs;
- deformação;
- skinning;
- transforms;
- cena;
- materiais;
- câmeras;
- luzes;
- animação;
- comandos transacionais;
- undo/redo;
- autosave;
- importação e exportação;
- validação de assets.

### 4.3 Renderer wgpu

Responsável por:

- criação e atualização de buffers;
- execução dos passes;
- compilação dos shaders canônicos;
- renderização interativa;
- renderização headless;
- leitura dos mesmos parâmetros usados pela exportação;
- diagnóstico de falhas de pipeline.

O renderer não deve possuir uma cópia alternativa do estado do projeto. Ele deve manter apenas recursos derivados e sincronizados a partir do `ProjectState`.

### 4.4 MCP e ferramentas externas

MCP deve operar por comandos públicos e observáveis. Não deve editar estruturas internas diretamente nem publicar métricas fabricadas.

Toda telemetria deve refletir valores reais do estado e do frame renderizado.

---

## 5. Pipeline canônico de dados

O fluxo correto para alterar um morph é:

```text
Usuário move slider
        ↓
UI cria SetMorphValueCommand
        ↓
Rust valida ID, faixa e permissões
        ↓
Rust aplica o comando ao ProjectState
        ↓
Rust recalcula dados derivados da deformação
        ↓
Renderer recebe atualização canônica
        ↓
Viewport renderiza
        ↓
Comando entra no histórico
        ↓
Autosave/exportação usam o mesmo ProjectState
```

É proibido:

```text
Usuário move slider
        ↓
TypeScript altera vértices localmente
        ↓
Rust permanece desatualizado
```

---

## 6. Renderização e paridade viewport/exportação

O renderer deve definir um contrato comum para:

- `SceneSnapshot`;
- `CameraSnapshot`;
- `MaterialSnapshot`;
- `LightSnapshot`;
- `RenderSettings`;
- `MeshBufferSet`;
- `PassGraph`.

Viewport e headless devem receber esses mesmos tipos.

### 6.1 Pass graph

A ordem dos passes deve ser declarada em uma única estrutura, por exemplo:

```text
DepthPrepass
Opaque
FaceShadow
ToonLighting
HairAndCloth
Outline
Transparent
PostProcess
ColorConvert
PresentOrExport
```

Não deve existir uma ordem hardcoded diferente para exportação.

### 6.2 Color management

O projeto deve documentar explicitamente:

- espaço de entrada de texturas;
- espaço de trabalho;
- espaço dos parâmetros de cor;
- conversões linear/sRGB;
- tonemapping;
- formato do framebuffer;
- formato da imagem exportada.

A ausência de uma definição de color management é considerada falha de paridade visual.

---

## 7. Plano de correção prioritário

### P0 — Consolidar a autoridade dos dados

1. Definir `ProjectState` no Rust.
2. Criar IDs estáveis para personagem, morphs, cena, materiais e assets.
3. Criar contratos TypeScript/Rust versionados.
4. Remover deformação de produção do TypeScript.
5. Fazer o viewport consumir snapshots do núcleo.

### P0 — Corrigir persistência

1. Implementar schema versionado.
2. Implementar validação de carga.
3. Implementar migrações.
4. Incluir personagem, cena, materiais, iluminação, câmera e render no autosave.
5. Fazer recuperação de sessão ser realmente lida e validada.

### P0 — Corrigir undo/redo

1. Converter alterações em comandos.
2. Garantir `apply` e `undo` simétricos.
3. Persistir somente comandos válidos.
4. Testar sequência `apply → undo → redo`.

### P0 — Consolidar o renderer

1. Escolher uma fonte canônica de shaders.
2. Remover shaders inline de produção.
3. Unificar buffers e uniforms.
4. Unificar câmera e transforms.
5. Unificar pass graph.
6. Fazer headless e viewport usarem o mesmo caminho.
7. Criar teste de paridade visual.

### P1 — Robustez

1. Remover `unwrap`, `expect` e falhas silenciosas do caminho crítico.
2. Implementar diagnóstico de erro no renderer.
3. Validar GLB/glTF antes de criar buffers.
4. Implementar skinning real.
5. Recalcular normais após deformação quando necessário.
6. Corrigir telemetria para usar valores reais.
7. Criar CI para build, testes e typecheck.

---

## 8. Critérios de aceite arquitetural

A fundação será considerada consolidada somente quando:

- não houver segunda implementação de deformação em TypeScript;
- todos os morphs canônicos forem processados pelo núcleo Rust;
- viewport, exportação e headless usarem os mesmos contratos;
- não houver shaders de produção duplicados;
- projeto puder ser salvo e carregado sem perda de estado;
- schema inválido gerar erro explícito e recuperável;
- undo/redo abranger todas as alterações persistentes;
- autosave puder restaurar uma sessão completa;
- câmera, transforms, materiais e iluminação forem aplicados no render;
- viewport e exportação passarem teste de paridade;
- telemetria refletir dados reais;
- `cargo check`, `cargo test`, `pnpm build` e `svelte-check` passarem;
- houver pelo menos um teste visual golden para o personagem e um para o shading.

Nenhuma nova sprint de alta complexidade deve ser considerada concluída enquanto esses critérios P0 permanecerem quebrados.

---

## 9. Política para novas funcionalidades

Toda nova funcionalidade deve responder antes da implementação:

1. Qual parte do `ProjectState` ela altera?
2. Qual comando representa a alteração?
3. Como ela será desfeita?
4. Como será serializada?
5. Como será validada?
6. Como chegará ao viewport?
7. Como chegará ao export/headless?
8. Qual shader ou buffer canônico utiliza?
9. Como será testada sem depender somente da UI?
10. Como será migrada em versões futuras do schema?

Se essas respostas não existirem, a funcionalidade deve permanecer em protótipo e não ser incorporada ao caminho de produção.

---

## 10. Resumo executivo

O ANIGO continuará com **Tauri + Rust + wgpu** porque essa combinação oferece o melhor equilíbrio entre controle, desempenho, independência da Unity e capacidade de criar um produto especializado em personagens anime.

A condição para essa decisão é a consolidação de quatro princípios:

1. **Rust é a autoridade dos dados e da deformação.**
2. **wgpu é o renderer canônico para viewport e exportação.**
3. **O projeto possui schema versionado e operações transacionais.**
4. **Shaders, buffers, câmeras, parâmetros e passes possuem uma única definição de produção.**

O TypeScript permanece como camada de interface e interação. Ele não deve se tornar um segundo motor.

Esta documentação deve ser tratada como referência obrigatória para as próximas correções e revisões arquiteturais do ANIGO.
