# Original User Request

## Initial Request — 2026-09-17T23:55:35Z

Reestruturar e expandir a arquitetura de workspaces do ANIGO Studio para um pipeline canônico de 8 workspaces especializadas (Personagem, Posing, Shading, Iluminação, Cenário, Animação, Render e Biblioteca), implementando para a Biblioteca um Asset Browser em grade com tags e busca em substituição ao 3D viewport tradicional.

Working directory: c:\ANIGO
Integrity mode: development

## Requirements

### R1. Separação Canônica em 8 Workspaces Especializadas
Reconfigurar os identificadores de workspace, tipagens TypeScript (WorkspaceId), dicionários de tradução (src/i18n/index.svelte.ts) e barra de navegação superior (app-titlebar) na seguinte sequência canônica:
1. **Personagem**: Focado na criação anatômica e proporções do personagem (cabeça, réguas, rosto, cabelo procedural e vestuário).
2. **Posing**: Manipulação da estrutura de rig/esqueleto, poses pré-definidas de manequim e controles de cinemática inversa (IK/FK).
3. **Shading**: Criação e ajuste de Toon Shaders e Cel-Shading NPR, Toon Ramp, limiares de sombra, reflexos de borda de material e shader ball / preview de esfera.
4. **Iluminação**: Controle completo de luz de cena e sol (azimute, elevação, intensidade, sombras anime e cor da luz solar).
5. **Cenário**: Blocagem Greybox, stage modular, posicionamento de elementos de cenário e ambiente 3D.
6. **Animação**: Linha do tempo (timeline), keyframing, curvas de interpolação e sincronia labial do personagem posicionado no cenário.
7. **Render**: Câmeras de cena, lentes, composição de saída, passes toon e exportação final de imagem/vídeo.
8. **Biblioteca**: Gestão completa de ativos (Asset Browser) sem o viewport 3D tradicional, exibindo uma interface limpa com categorias, tags e busca.

### R2. Barra Lateral de Ferramentas e Painel de Propriedades Dedicados
- **Barra Lateral Esquerda**: Cada uma das 8 workspaces deve apresentar somente os ícones vetoriais SVG específicos de suas respectivas ferramentas, eliminando sobreposições ou ferramentas misturadas.
- **Painel Direito (Inspector)**: Deve apresentar os controles, sliders e seletores contextuais da ferramenta selecionada em cada workspace.
- **Sincronização 3D e Undo/Redo**: Para as workspaces com viewport 3D (1 a 7), as alterações de sliders e presets devem continuar sincronizadas em tempo real com o motor WebGPU e registradas no HistoryService.

### R3. Módulo de Biblioteca (Asset Browser Estilo Unreal Engine)
- Ao ativar a workspace biblioteca:
  - O viewport 3D WebGPU dá lugar a uma interface central de Asset Browser.
  - Painel lateral ou abas superiores de filtros: Personagens, Roupas, Penteados, Acessórios, Poses, Materiais, Ambientes e Cenários.
  - Barra de busca rápida por nome e tags.
  - Grade central com cards dos ativos, miniaturas estilizadas com ícones vetoriais, nomes, contagem de polígonos/metadados e ações ("Usar no Cenário", "Inspecionar").

## Acceptance Criteria

### Integridade Arquitetural e Tipagem
- [ ] O enum/tipo WorkspaceId em src/App.svelte e submódulos contém exatamente as 8 chaves (personagem, posing, shading, iluminacao, cenario, animacao, render, biblioteca).
- [ ] Dicionário i18n (src/i18n/index.svelte.ts) possui traduções completas para os rótulos de todas as 8 workspaces e suas ferramentas.
- [ ] Compilação do Vite (pnpm build) passa com 0 erros.
- [ ] Compilação do Rust (cargo check --package anigo-app) passa com 0 erros.

### Ergonomia e Navegação
- [ ] A troca entre as 8 abas no topo da aplicação é imediata e atualiza a barra lateral esquerda e o painel direito sem falhas visuais.
- [ ] Na workspace "Biblioteca", a área central exibe o Asset Browser completo com categorias, grade de cards e busca.
- [ ] Nas workspaces de 1 a 7, o viewport 3D WebGPU permanece interativo e responsivo.
- [ ] O sistema de Undo/Redo (Ctrl+Z, Ctrl+Y) opera sem regressões nos parâmetros do estúdio.
