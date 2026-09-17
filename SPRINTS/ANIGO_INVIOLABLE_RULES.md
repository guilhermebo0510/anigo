# PROTOCOLO INVIOLÁVEL DE ENGENHARIA E PADRÃO PROFISSIONAL ANIGO
*Este documento estabelece as diretrizes obrigatórias e inegociáveis para o desenvolvimento de todas as 22 Sprints do ANIGO.*

---

## 1. A REGRA INVIOLÁVEL DE VALIDAÇÃO DE SPRINT

> [!CAUTION]
> **Nenhuma sprint será considerada validada ou concluída sem a execução de testes manuais e visuais reais através do MCP próprio (`anigo-mcp`).**
> É terminantemente proibido marcar uma sprint como concluída ou avançar para a próxima etapa sem antes apresentar as evidências práticas, visuais e auditáveis do funcionamento da funcionalidade.

### 1.1. Protocolo de Teste Visual via MCP
1. O servidor `anigo-mcp` deve ser aberto e conectado à instância do motor gráfico.
2. Os testes devem enviar comandos reais de controle (câmera, luz, deformação, malha, material) para o motor.
3. Para cada teste visual, um frame em alta definição renderizado pelo hardware real (Vulkan/DirectX 12/Metal) deve ser inspecionado criticamente pelo agente, gerando o relatório visual detalhado do que foi visto (silhueta, terminador de sombra, anti-aliasing, normais).
4. No aplicativo interativo, os comandos do MCP devem ser transmitidos via WebSocket/IPC para a janela aberta do usuário, permitindo validação simultânea.

### 1.2. Módulos Exclusivamente Backend
Quando uma etapa for de infraestrutura pura (ex: serializador binário glTF, buffers de memória, parsing de blendshapes, algoritmos matemáticos não visuais):
- O agente deve rodar uma suíte completa de testes unitários e de estresse (`cargo test`).
- Deve ser gerado um relatório de auditoria detalhando conformidade com as normas (ex: validador Khronos para glTF/VRM 1.0, tempos de execução e alocação de memória).
- Sempre que houver qualquer impacto visual indireto, o motor deve gerar imediatamente um teste de renderização offscreen comprovando a não-regressão.

### 1.3. Ações que Requerem Controle de Mouse/Teclado na Máquina do Usuário
- Caso um teste necessite movimentar o cursor do mouse, disparar cliques no sistema operacional ou capturar janelas fora do sandbox:
  - **O agente NUNCA agirá de surpresa.**
  - Deve solicitar expressamente autorização ao usuário, descrevendo o que será feito, e **aguardar a confirmação explícita** do usuário antes de realizar a ação.

---

## 2. TOLERÂNCIA ZERO A IMPLEMENTAÇÕES PROVISÓRIAS OU AMADORAS

> [!IMPORTANT]
> O ANIGO é concebido como um software comercial de nível industrial para superar o VRoid Studio e atender a estúdios de animação. Atalhos amadores e soluções temporárias são expressamente vetados.

1. **Proibido Mockups de Viewport:**
   - O Viewport 3D NUNCA deve ser uma tag `<img>` recebendo imagens base64, nem canvas 2D desenhando SVGs simulados.
   - O Viewport DEVE ser um `<canvas>` WebGPU nativo renderizando shaders WGSL diretamente na GPU a 60/120+ FPS, sincronizado com o motor em Rust.
2. **Proibido Emojis na Interface de Usuário:**
   - Nenhum botão, aba ou controle do software deve utilizar caracteres de emoji amadores como ícones.
   - Toda a iconografia deve utilizar padrões vetoriais profissionais (ícones SVG customizados de estúdio ou bibliotecas de design de ponta como Lucide / Phosphor Icons).
3. **Código Limpo, Robusto e Idiomático:**
   - Em Rust: Tipagem forte, zero `unwrap()` em caminhos críticos, tratamento de erros via `Result`/`thiserror`/`anyhow`, ausência de *leaks* de VRAM, e testes de unidade integrados.
   - No Frontend: TypeScript estrito, componentes Svelte 5 modulares, CSS limpo com design dark profissional (inspirado em Blender 4, Unreal Engine 5 e DaVinci Resolve).

---

## 3. PESQUISA CONTÍNUA E ESTADO DA ARTE DA INDÚSTRIA

Antes de iniciar o código de qualquer sprint:
1. O agente deve realizar pesquisas técnicas ativas em documentações oficiais (W3C WebGPU, Khronos glTF/VRM 1.0, W3C WGSL).
2. Deve consultar artigos acadêmicos (ACM SIGGRAPH, IEEE) e palestras de conferências técnicas (GDC, CEDEC) para garantir que o algoritmo escolhido seja a referência moderna da indústria (ex: SDF Face Maps do Genshin Impact, Inverted Hull do Guilty Gear Xrd, XPBD do Miles Macklin, Green Coordinates do Lipman).
3. Nenhuma tecnologia defasada ou abordagem ingênua deve ser adotada por conveniência de implementação.

---

## 4. O CHECKLIST DE AUTO-AVALIAÇÃO OBRIGATÓRIO (HEURÍSTICA DE 7 PERGUNTAS)

Antes de reportar qualquer tarefa como concluída, o agente é OBRIGADO a executar internamente esta sabatina:

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                 CHECKLIST DE AUTO-AVALIAÇÃO DO AGENTE                       │
├─────────────────────────────────────────────────────────────────────────────┤
│ 1. Isso está tecnicamente correto?                                          │
│ 2. Faz EXATAMENTE o que deveria fazer de acordo com a especificação?        │
│ 3. Precisa de melhorias ou refatoração?                                     │
│ 4. Existem bugs, defeitos, vazamentos de memória ou artefatos visuais?      │
│ 5. Preciso buscar mais informações ou artigos técnicos em pesquisas?        │
│ 6. Está profissional e pronto para um estúdio de animação real?             │
│ 7. Está moderno e segue o mais alto padrão estético e de engenharia?        │
├─────────────────────────────────────────────────────────────────────────────┤
│ SE HOUVER UM ÚNICO "NÃO", A TAREFA NÃO ESTÁ PRONTA E DEVE SER CORRIGIDA.    │
└─────────────────────────────────────────────────────────────────────────────┘
```
