# Relatório final — migração nativa do ANIGO (sessão de nuvem)

Branch: `claude/brave-galileo-cwdfei` (PR #61). Tudo abaixo foi construído e validado **só em nuvem**: Linux, Xvfb, lavapipe (Vulkan em CPU), xdotool dentro de um display privado. Nenhuma GPU real, nenhum Windows, nenhum mouse/caneta reais, nenhum arquivo de artista real. Esse é o fato mais importante para ler o resto.

## 1. O que foi entregue

| Área | Estado |
|---|---|
| App nativo `anigo-studio` (egui 0.36 + wgpu 30, hot reload, 9 workspaces) | funcional; Tauri/Svelte segue compilando em paralelo |
| Motor de modelagem `anigo-modeling` (Blender como referência) | operadores, modificadores, escultura (20 pincéis), UV, normais anime, cabelo, shape keys |
| Motor cel (`anigo-renderer`) | DoF, rampa 2D, contorno com normais suavizadas/falloff/taper, face SDF espelhado, shadow map de 1 luz, paridade WGSL↔Rust |
| Biblioteca (`anigo-assets` + UI) | SQLite/FTS5, import GLB/glTF/VRM/OBJ/FBX, material toon canônico, miniaturas 3D, grade virtualizada, lixeira, duplicados, relink |
| Cenário (estilo UE5, Fase 1) | Content Drawer, gizmo W/E/R, snaps, outliner/details/histórico, câmera de cena |
| Personagem | sliders → comandos do núcleo → preview cel (sobre o manequim canônico antigo) |
| Posing | pose por osso, limites, IK, foot lock, espelhar, poses como ativos |
| Shading | editor de material toon, rampa por linha, paleta, presets na Biblioteca |
| Iluminação | gizmo do sol, ambiente, rigs, sombra projetada |
| Render | câmera, presets, supersampling, alfa, AOVs, fila real, histórico |
| Animação | timeline, dope sheet, graph editor, auto-key, onion skin, retarget BVH/glTF/VRMA, render de sequência |
| Física `anigo-physics` | spring bone VRM exato, XPBD, cabelo, saia/capa (núcleo testado; **não ligado ao renderer**) |
| Base de personagem v2 `anigo-charbase` | manequim anatômico masc/fem, 65 ossos, 64 morphs, contrato de topologia |
| Ferramenta de cabelo estilo VRoid | Modo Cabelo na Modelagem |
| Bridge MCP nativa `anigo-bridge` | `anigo-mcp` controla o studio pela mesma `CoreSession`, token + loopback |
| Layout dockável, atalhos configuráveis, menus | `egui_tiles`, 128 ações, layouts persistidos |

Números (nuvem): 2 305 testes Rust passando (workspace sem o app Tauri), 352 testes TS, fixtures/WGSL/sintaxe verdes, cerca de 190 commits novos, ~190 mil linhas Rust nos crates novos/alterados. Pesquisas com fontes em `docs/native/pesquisa/` (13 relatórios), um `*_UI.md` por workspace, capturas de referência em `docs/native/img/`.

Correções reais achadas no caminho (valem também para o app antigo): tabela de `componentType` do glTF estava errada em Rust e TS (índices u16 lidos como u32); `extensionsUsed/Required` ficavam em `asset` em vez da raiz; `VRMC_springBone` fora do schema oficial; `vs_dof` degenerado; Ctrl+Z do Cenário ia para a sessão errada; `ci.yml` nem parseava como YAML.

## 2. O que NÃO foi feito

- **Aposentar o app Svelte (T-2.14):** não. Falta uma auditoria de paridade com você (a lista do que o Svelte faz e o nativo não). O contrato TS espelha o núcleo e há 352 testes que dependem dele.
- **Migrar o Personagem para a base v2:** não. O Personagem ainda usa o mesh canônico antigo (4 patches abertos, ~4 mil vértices) com os 157 sliders; os GLBs de teste em `assets/models/` seguem no repo porque o core, os testes e os goldens dependem deles.
- **Roupa:** não existe ferramenta de roupa (painéis/costura/provador). Só o núcleo físico (`anigo-cloth` delega ao `anigo-physics`).
- **Pintura de textura**: nenhum workspace/ferramenta (aparece "Em breve" no Personagem).
- **Weight paint e rigging tools** na Modelagem/Posing: não há pintura de pesos nem edição de esqueleto.
- Cenário Fases 2–3 (vertex snap, sockets modulares, prefabs, instancing/scatter, terreno, splines, luzes extras).
- Luzes extras (ponto/spot/retangular), HDR/tonemap, EXR, bloom/grain/LUT (exigem contrato de render v2).
- T-1.9 booleanas/QuadriFlow, `.anigopack`, fontes remotas da Biblioteca, KTX2/Basis, Draco/meshopt/USD.

## 3. O que falta para chegar ao nível comercial

Prioridade: **P0** = bloqueia uma beta pública; **P1** = necessário para competir; **P2** = diferencial.

### P0 — Realidade (nada disto foi sentido ou medido fora da nuvem)
1. **Build e execução em Windows/MSVC com GPU real** (e Mac/Linux desktop): hoje só lavapipe. Medir: tempo de quadro, memória, rolagem da Biblioteca a 60 FPS, escultura em malha densa, render 4K×SS2 e A4 300 dpi, física, miniaturas 3D com viewport ativo. Os goldens de frame (hash) só valem em lavapipe: trocar por métrica perceptual em GPU real.
2. **Entrada humana:** sensação de mouse, trackpad e caneta (pressão não é lida em nenhuma ferramenta além de `Event::Touch.force` cru), fly/gizmo/snap do Cenário, pincéis de escultura, traço de cabelo, arrasto de timeline, IME/acentos, HiDPI, Wayland, Cmd vs Ctrl, teclados não-US. Tudo foi exercitado só por xdotool em passos discretos.
3. **Arquivos reais de artista:** nenhum VRoid/UniVRM, Blender (GLB/FBX), Sketchfab, Quixel/Fab, Mixamo/BVH de captura foi importado; nenhum FBX válido foi aberto; nenhum `.vrm` exportado foi carregado em UniVRM/three-vrm/VSeeFace. Os testes usam fixtures sintéticos.
4. **Robustez de produto:** um projeto único coerente (hoje o histórico é **fragmentado**: Modelagem, núcleo e Shading têm undo próprio; Ctrl+Z depende do workspace), salvar/abrir/autosave/recuperação de crash ponta a ponta incluindo malhas da Modelagem, versionamento e migração de arquivos de projeto, e diálogos nativos de arquivo (hoje: caminho digitado em vários fluxos).
5. **Personagem de verdade:** o manequim Gen-0 precisa ser esculpido (rosto anime, mãos, pés, silhueta) olhando as suas referências; depois migrar o CoreSession/157 sliders para a base v2, acrescentar o macro de **idade** (não existe no catálogo), re-derivar pesos e conferir deformação em poses extremas (ombro, quadril, axila, dedos). Sem isso o produto não tem o personagem-âncora.
6. **Contrato do renderer v2** (decisão sua): hoje congelado em 24 ossos, uma luz, MSAA 4× fixo, alvo 8 bits, sem texturas por material (só o mapa SDF facial), sem alpha/recorte, sem instancing. Passar a um buffer de N ossos (VRM completo + cabelo/saia/busto), texturas por material, luzes extras e HDR é pré-requisito de roupa, cabelo com física e materiais VRoid/MToon reais.

### P1 — Funcionalidade para competir
7. **Cabelo e roupa completos:** física ligada ao render (hoje só overlay/dado), cabelo com angel ring por tangente do fio, cabelo translúcido e hair-through, colisão cabelo×cabeça na ferramenta, **ferramenta de roupa** (painéis, costura, vínculo ao corpo, herança de pesos, provador), auto-colisão, calibração de parâmetros por artista, export `VRMC_springBone` validado em players reais.
8. **Rigging e pesos:** weight paint, auto-pesos de qualidade, ossos de dedo no esqueleto de render (hoje os dedos são deformação CPU), ossos de rosto/olhos/mandíbula no preview, correctivos, controle de twist.
9. **Animação profissional:** NLA/mistura de clips, constraints, modificadores de curva, lipsync/visemas e expressões animadas, áudio na timeline, retarget conferido com Mixamo/VRMA/BVH reais, física no playback, encode de vídeo (hoje só mostra a linha do ffmpeg; nunca executa).
10. **Render comercial:** EXR/HDR, tiles acima de 8192 px, bloom/grain/LUT, AOVs por passe real de GPU (hoje por rasterizador de software), tonemap, passes por objeto/camada, ffmpeg integrado, fila persistida em disco.
11. **Modelagem no nível do Blender:** booleanas, retopo/QuadriFlow, multires, curvas/NURBS, UV mais robusto em malhas grandes, edição de shape keys completa, Dyntopo escalável (hoje refaz tudo por quadro), modelagem com BVH incremental na pintura de atributos, bevel/loop cut em casos difíceis.
12. **Cenário no nível do UE5:** Fases 2–3 da pesquisa (vertex snap, sockets modulares, prefabs com variantes, instancing/scatter, terreno, splines, camadas, level instances), cenas com centenas/milhares de nós (hoje `CelPreview` reenvia vértices a cada quadro).
13. **Biblioteca de produção:** fontes remotas, `.anigopack`, KTX2/Basis, Draco/meshopt/USD, preferências persistidas, arrastar ativo para a viewport da Modelagem, benchmark real a frio/HDD/10⁶ ativos, ativos vinculados em rede (SMB corrompe SQLite).
14. **Iluminação:** luzes extras, sombra além de 1 mapa/7 m, contact shadows, volumes de look/pós-processo, luz por plano/cena.

### P2 — Acabamento e diferencial
15. **UX:** janelas flutuantes no dock (o `egui_tiles` não tem; avaliar `egui_dock`), chords e esquemas de keymap alternativos, onboarding/tutoriais, tooltips ricos, templates de projeto, tema claro, acessibilidade (leitor de tela e contraste não foram tratados), localização além de pt/en.
16. **Qualidade/engenharia:** CI com **GPU real** e runner Windows, testes automatizados de UI além de screenshots/smoke (poucos testes egui; o grosso é por captura), orçamentos de desempenho em CI (o teste `grid_frame_time_*` falha sob carga porque tem limite de tempo), fuzzing dos importadores, telemetria/relato de crashes opt-in, perfil de build de release otimizado (LTO, dependências em opt-level 3 no dev), tempo de compilação no Windows.
17. **Distribuição:** instalador assinado, atualização automática, versionamento semântico, política de licenças (o repo contém `modelos/feminino/animebasemesh.glb` e um zip CC-BY de terceiros que não devem ser redistribuídos: remover do repositório), SBOM e auditoria de licenças das dependências (rusqlite, ufbx, xatlas, meshopt, egui-phosphor).
18. **Segurança da bridge MCP:** testar no Windows (ACL do arquivo de descoberta), com cliente MCP real (Claude Desktop/Code), janela minimizada, `SET_SELECTION`, eventos empurrados do studio, undo da bridge só vale para a `CoreSession`.
19. **Dívida de contratos:** o núcleo Rust e o espelho TS convivem (COMMAND_KINDS, fixtures, 352 testes). Ou o Svelte é aposentado e o espelho some, ou o custo de manter os dois cresce a cada comando novo. O dialeto VRM próprio do `anigo-vrm` (MToon fora da spec, `VRMC_vrm` em `extensionsRequired`) precisa ser alinhado à spec 1.0.
20. **Bugs e limites conhecidos** registrados pelos agentes: `set_base_gender` aplicado pela bridge entra no histórico mas a viewport do Personagem não mudou (causa não investigada); o smoke da escultura falhou no sandbox no binário anterior também (sem causa confirmada); cel “muito branco” com materiais padrão; sombra projetada azul-marinho no material padrão; nós culados pela câmera não projetam sombra; Cenário usa sempre a revisão atual do ativo; pastas do Outliner só na raiz; histórico de undo não é restaurado entre execuções.

## 4. Próximos passos sugeridos (ordem)

1. No chat local, seguir `docs/native/VALIDACAO_LOCAL.md` (§0 preparar, §1 entrada, §2 GPU, §4 Windows/MSVC): compilar, abrir cada workspace e anotar o que quebra. É o maior retorno por hora.
2. Esculpir o rosto/corpo da base v2 com as imagens de `C:\ANIGO\IMAGENS` e validar com `cargo run -p anigo-charbase --example check_sculpt -- meu.glb`.
3. Decidir o **contrato de render v2** (N ossos, texturas por material, luzes extras, HDR) — destrava roupa, cabelo físico e materiais VRoid.
4. Auditar a paridade Svelte→nativo; só depois mover `src/` para um branch de arquivo (T-2.14).
5. Ligar o pipeline de física ao render e entregar a ferramenta de roupa.

Detalhes por funcionalidade: `VALIDACAO_LOCAL.md` §9, e cada `docs/native/*_UI.md` (seção "O que não foi validado").
