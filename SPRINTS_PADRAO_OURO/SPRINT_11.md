# ANIGO — ESPECIFICAÇÃO DE ENGENHARIA DE ALTO PADRÃO

> [!CAUTION]
> **REGRA INVIOLÁVEL DESTA SPRINT**: Esta sprint NÃO será considerada concluída sem testes manuais e visuais reais executados via nigo-mcp com geração e inspeção crítica de frames de prova. É expressamente proibido o uso de mockups (como imagens base64 disfarçadas de 3D, SVGs fictícios ou emojis na interface). A interface deve ser de padrão comercial (ícones vetoriais profissionais) e o código limpo, robusto e sem atalhos.

# SPRINT 11: Interoperabilidade Total e Exportacao/Importacao VRM 1.0

## 1. Objetivo e Escopo Industrial
Serializador e deserializador bidirecional glTF 2.0 / VRM 1.0 (VRMC_vrm, VRMC_materials_mtoon, VRMC_springBone, VRMC_node_constraint).

## 2. Pacotes e Arquitetura Envolvida
- **Módulos / Crates**: crates/anigo-vrm
- **Padrão de Qualidade**: Código em Rust idiomático (zero unwrap em caminhos críticos), Svelte 5 com TypeScript estrito, shaders WGSL otimizados.

## 3. Protocolo Inviolável de Teste Visual via MCP
- **Ferramentas MCP a executar e auditar**: anigo_validate_vrm_file, anigo_export_vrm, anigo_import_vrm, anigo_render_frame
- **Evidência Obrigatória**: Renderização offscreen na GPU real ou inspeção no canvas nativo. Relatório visual descritivo de cada frame capturado.
- **Autorização do Usuário**: Se a validação requerer controle de cursor/mouse, solicitar previamente autorização explícita e aguardar.

## 4. Critérios Estritos de Aceite (Definition of Done)
- Arquivo VRM 1.0 exportado carrega sem avisos no validador oficial Khronos e em players comerciais.
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

