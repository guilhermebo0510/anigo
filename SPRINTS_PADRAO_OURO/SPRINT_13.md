# ANIGO — ESPECIFICAÇÃO DE ENGENHARIA DE ALTO PADRÃO

> [!CAUTION]
> **REGRA INVIOLÁVEL DESTA SPRINT**: Esta sprint NÃO será considerada concluída sem testes manuais e visuais reais executados via nigo-mcp com geração e inspeção crítica de frames de prova. É expressamente proibido o uso de mockups (como imagens base64 disfarçadas de 3D, SVGs fictícios ou emojis na interface). A interface deve ser de padrão comercial (ícones vetoriais profissionais) e o código limpo, robusto e sem atalhos.

# SPRINT 13: Linha do Tempo, Dopesheet e Animacao Escalonada (1s, 2s, 3s - Koma-uchi)

## 1. Objetivo e Escopo Industrial
Editor grafico de timeline com canais para ossos e blendshapes, interpolacao em degrau (stepped timing) simulando 8/12/24 FPS tradicionais.

## 2. Pacotes e Arquitetura Envolvida
- **Módulos / Crates**: crates/anigo-animation, src/components/timeline/Dopesheet.svelte
- **Padrão de Qualidade**: Código em Rust idiomático (zero unwrap em caminhos críticos), Svelte 5 com TypeScript estrito, shaders WGSL otimizados.

## 3. Protocolo Inviolável de Teste Visual via MCP
- **Ferramentas MCP a executar e auditar**: anigo_set_timeline_frame, anigo_play_animation, anigo_render_frame
- **Evidência Obrigatória**: Renderização offscreen na GPU real ou inspeção no canvas nativo. Relatório visual descritivo de cada frame capturado.
- **Autorização do Usuário**: Se a validação requerer controle de cursor/mouse, solicitar previamente autorização explícita e aguardar.

## 4. Critérios Estritos de Aceite (Definition of Done)
- Movimento rítmico autêntico de anime japones sem interpolacao linear excessivamente plastica.
- Zero implementação provisória (sem SVGs simulados, sem <img> como viewport, sem ícones de emoji).
- Teste executado e comprovado no relatório de inspeção visual.


---

## Checklist de Auto-Avaliação Obrigatório (7 Perguntas)
Antes de finalizar qualquer entrega nesta sprint, responda estritamente:
1. **Isso está tecnicamente correto?**
2. **Faz exatamente o que a especificação exige?**
3. **Precisa de melhorias ou refatoração?**
4. **Existem bugs, defeitos ou artefatos visuais?**
5. **Preciso buscar informações ou referências em pesquisas especializadas?**
6. **Está no padrão estético e técnico de um software comercial profissional?**
7. **Está moderno e com engenharia de alto padrão?**
*(Se houver um único NÃO, o código DEVE ser corrigido antes da validação).*

