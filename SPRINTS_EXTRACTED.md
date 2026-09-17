# ANIGO - Sprint 01_ Infraestrutura Básica, Shell e Viewport WebGPU.docx

ANIGO — Sprint 01: Infraestrutura Básica, Shell e Viewport WebGPU
1. Objetivo
Inicializar o repositório em C:\ANIGO, configurar a arquitetura Tauri 2.0 com backend em Rust e wgpu, e renderizar a primeira malha 3D com câmera orbital fluida a 120+ FPS.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-renderer: Instância wgpu, dispositivo, fila e swapchain.
src/components/viewport/Viewport.svelte: Canvas nativo com ponte IPC.
3. Tarefas Técnicas de Engenharia
Inicializar o workspace Cargo com crates modulares (core, renderer, ik, hair, cloth, vrm, ai).
Configurar pipeline básico de renderização wgpu com suporte a Vulkan, DirectX 12 e Metal.
Implementar câmera orbital baseada em quaternions (rotação, pan e zoom por scroll).
Criar carregador básico de malha base em formato glTF/OBJ para teste de viewport.
4. Critérios de Aceite (Definition of Done)
Aplicação inicializa em menos de 1,5 segundo consumindo menos de 50 MB de RAM base.
Viewport renderiza a malha de teste sem travamentos ou fugas de memória.

------------------------------------------------------------

# ANIGO - Sprint 02_ Pipeline Cel-Shading NPR Inicial e Luzes Toon.docx

ANIGO — Sprint 02: Pipeline Cel-Shading NPR Avançado e Sistema de Luzes Toon
1. Topologia de Arquivos e Arquitetura
shaders/cel_shading.wgsl: Vertex e fragment shaders com modelo Toon.
src-tauri/crates/anigo-renderer: Uniform buffers para luz direcional e ambiente.
src/components/character/LightingControls.svelte: Slider de direção e intensidade da luz.
2. Implementação de Shader WGSL (Cel-Shading)
// Implementar amostragem de textura 1D/2D para Toon Ramp
// Separação de área iluminada e sombra
// Lógica matemática de rotação de matiz (hue shifting) nas áreas sombreadas
3. Estruturas de Dados Rust (anigo-renderer)
// LightUniform e MaterialUniform com alinhamento WebGPU
// Buffers uniformes de parâmetros lumínicos atualizáveis via IPC
4. Comandos IPC Tauri 2.0 e Componente Svelte
O componente LightingControls.svelte gerencia o controle orbital de iluminação, enviando atualizações instantâneas via IPC sem recriar pipelines gráficos.
5. Checklist de Engenharia (Antigravity)
Configurar amostragem Toon Ramp (1D/2D).
Aplicar Hue Shifting matemático nas sombras.
Integrar buffers uniformes de luz atualizáveis.
Validar performance com anigo-test-mcp.
6. Critérios de Aceite (Definition of Done)
A malha exibe visual plano de anime com transição nítida entre luz e sombra em tempo real.
Ajustar o controle de luz na interface atualiza instantaneamente o modelo a 60+ FPS.

------------------------------------------------------------

# ANIGO - Sprint 03_ Manequim Anatômico e Caixas de Volume.docx

Especificação Técnica: Sprint 03 — Manequim Anatômico e Deformação por Caixas de Volume
1. Visão Geral e Algoritmos de Deformação
O sistema implementa a manipulação direta de volumes corporais através de caixas 3D (estilo Design Doll) com foco em preservação de curvatura. O motor geométrico utiliza Coordenadas Laplacianas (L V = Δ) com pesos cotangentes para garantir a suavidade da malha durante a escala de segmentos, enquanto o deformador principal baseia-se em Mean Value Coordinates (MVC) para o mapeamento eficiente entre as bounding cages e a malha humanoide de alta resolução, evitando descontinuidades nas juntas.
2. Topologia de Arquivos e Estruturas Rust (anigo-core)
src-tauri/crates/anigo-core/src/volumetric/: Implementação das estruturas BoundingCage, AnatomicalSegment e SegmentGizmo para gestão de caixas de volume.
src-tauri/crates/anigo-core/src/solvers/laplacian.rs: Solver para preservação de proporções e detalhes anatômicos via coordenadas diferenciais.
src/components/character/Proportions.svelte: Componente de UI para a régua interativa de cabeças (ajustável de 2 a 8.5).
3. Comunicação IPC (Tauri 2.0) e Validação
Os comandos IPC gerenciam a sincronização entre os gizmos Svelte/Three.js e o back-end Rust. O comando update_segment_transform envia matrizes de escala local para o solver MVC. A validação de integridade da malha e das proporções é automatizada via anigo-test-mcp, garantindo que deformações extremas (ex: Chibi 2.0) não causem colapso de vértices.
4. Checklist de Engenharia e Critérios de Aceite
Segmentação completa da malha em: tórax, pelve, braços, pernas e cabeça.Integração de gizmos 3D interativos para rotação e escala em eixos locais por segmento.Critério de Aceite: Alteração de silhueta em tempo real sem estiramento visível de texturas ou artefatos de curvatura, validado pela régua visual de proporção.

------------------------------------------------------------

# ANIGO - Sprint 04_ Cinemática Inversa e Poses com Pinagem.docx

ANIGO — Sprint 04: Cinemática Inversa e Poses com Pinagem
1. Objetivo
Construir o sistema de posing com cinemática inversa ultrarrápida (FABRIK), pinagem espacial de membros e modificador de perspectiva de mangá.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-ik: Solucionador FABRIK com restrições articulares.
src/components/posing/PoseEditor.svelte: Ferramenta de linha de ação e ancoragem.
3. Tarefas Técnicas de Engenharia
Implementar o algoritmo FABRIK iterativo com limites angulares anatômicos em Rust.
Adicionar sistema de pinagem universal (ancorar pés ao solo ou mãos a apoios no espaço 3D).
Implementar ferramenta gestual de Linha de Ação para arquear a coluna vertebral com um único traço.
Criar modificador de perspectiva local de câmera para escorço dinâmico (*foreshortening*).
4. Critérios de Aceite
Fixar ambos os pés no chão e puxar a pelve faz os joelhos dobrarem naturalmente sem deslocar os pés.

------------------------------------------------------------

# ANIGO - Sprint 05_ Normais Customizadas e Contorno Inverted Hull.docx

ANIGO — Sprint 05: Normais Customizadas e Contorno Inverted Hull
1. Objetivo
Implementar contornos nítidos estáveis (*lineart*) via casca invertida e transferir normais esféricas para a face, eliminando artefatos de sombreamento facial.
2. Arquitetura e Pacotes Envolvidos
shaders/inverted_hull.wgsl: Passe de extrusão de vértices para contorno.
src-tauri/crates/anigo-renderer: Algoritmo k-NN para transferência de normais elipsoidais.
3. Tarefas Técnicas de Engenharia
Implementar passe de renderização de casca invertida com espessura compensada pela distância da câmera.
Modular a espessura e a cor do traço através do canal de cores de vértices (*vertex color*).
Transferir normais de uma elipse suave para a malha facial para obter sombras faciais perfeitamente limpas.
Adicionar detecção de bordas internas por Sobel sobre G-Buffers de profundidade e normais.
4. Critérios de Aceite
O modelo 3D exibe traço de anime nítido sem serrilhado ao girar a câmera e o rosto mantém sombreamento contínuo.

------------------------------------------------------------

# ANIGO - Sprint 06_ Cabelo Procedural por Splines 3D Livres.docx

ANIGO — Sprint 06: Cabelo Procedural por Splines 3D Livres
1. Objetivo
Permitir o desenho de mechas de cabelo livres no espaço 3D via caneta gráfica, com extrusão procedural de fitas (*ribbon meshing*) e mapeamento UV contínuo.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-hair: Matemática de curvas B-Spline, tesselação e geração de fitas.
src/components/character/HairStudio.svelte: Ferramenta de caneta e perfil de mecha.
3. Tarefas Técnicas de Engenharia
Implementar desenho interativo de curvas com ancoragem no escalpo via raycasting.
Criar gerador procedural de fitas tridimensionais (perfis plano, triangular e tubular).
Adicionar controles por vértice de mecha: torção axial (*twist*), curvatura e afunilamento (*taper*).
Calcular coordenadas UV automáticas desdobradas linearmente da raiz à ponta da mecha.
4. Critérios de Aceite
O artista desenha mechas volumosas no espaço 3D livremente, gerando malhas limpas prontas para textura em tempo real.

------------------------------------------------------------

# ANIGO - Sprint 07_ Física Capilar com XPBD na GPU.docx

ANIGO — Sprint 07: Física Capilar com XPBD na GPU
1. Objetivo
Simular a movimentação dinâmica do cabelo em tempo real via Extended Position Based Dynamics (XPBD) em compute shaders WebGPU, com colisões analíticas estáveis.
2. Arquitetura e Pacotes Envolvidos
shaders/xpbd_simulation.wgsl: Solver de restrições de distância, curvatura e amortecimento.
src-tauri/crates/anigo-hair: Gerenciamento de partículas e colisores no crânio e ombros.
3. Tarefas Técnicas de Engenharia
Discretizar mechas de cabelo como correntes de partículas sob restrições XPBD na GPU.
Implementar campos de colisão analíticos (esferas e cápsulas) alinhados à cabeça e pescoço.
Adicionar controles de rigidez elástica, amortecimento e gravidade na interface.
4. Critérios de Aceite
O cabelo flui suavemente com o movimento da cabeça sem atravessar o crânio e sem instabilidade numérica.

------------------------------------------------------------

# ANIGO - Sprint 08_ Desacoplamento Facial, Decalques e Blendshapes.docx

ANIGO — Sprint 08: Desacoplamento Facial, Decalques e Blendshapes
1. Objetivo
Estruturar o rosto em camadas desacopladas (olhos e sobrancelhas como decalques sobrepostos ao cabelo), íris vetorial e suporte aos 52 blendshapes faciais.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-core: Malha de decalques flutuantes e blendshapes ARKit/VRM.
src/components/character/FaceEditor.svelte: Personalização de íris, pupilas e expressões.
3. Tarefas Técnicas de Engenharia
Implementar geometria de decalques projetados sobre a face com ordenação de profundidade Z-buffer.
Habilitar renderização de sobrancelhas sobre o cabelo (*eyebrows over hair*), convenção clássica de anime.
Criar gerador procedural de íris e brilhos oculares personalizáveis em vetor.
Configurar os 52 coeficientes faciais padrão ARKit/VRM 1.0 para lipsync e captura facial.
4. Critérios de Aceite
Expressões faciais complexas funcionam em tempo real com sobrancelhas visíveis através de mechas de cabelo.

------------------------------------------------------------

# ANIGO - Sprint 09_ Vestuário Paramétrico e Alfaiataria 2D_3D.docx

ANIGO — Sprint 09: Vestuário Paramétrico e Alfaiataria 2D/3D
1. Objetivo
Modelar roupas por meio de moldes de costura 2D drapeados em 3D sobre o manequim, com camadas anti-interpenetração e gerador de dobras estilizadas.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-cloth: Solver de costura 2D para drape 3D, detecção contínua de colisão.
src/components/character/ClothTailor.svelte: Interface de moldes de peças de roupa.
3. Tarefas Técnicas de Engenharia
Criar editor de moldes planos 2D (frente, costas, mangas, saia) que se unem por forças elásticas no espaço 3D.
Implementar hierarquia de camadas de roupas com campos de penalidade de contato (CCD) anti-interseção.
Adicionar shaders de vincos agudos e dobras estilizadas nas juntas anatômicas.
4. Critérios de Aceite
A roupa veste e se conforma automaticamente a qualquer formato de corpo sem atravessar tecidos adjacentes.

------------------------------------------------------------

# ANIGO - Sprint 10_ Interoperabilidade e Exportação VRM 1.0.docx

ANIGO — Sprint 10: Interoperabilidade e Exportação VRM 1.0
1. Objetivo
Concluir o Passo 1 permitindo exportar o personagem anime completo (malha, ossos, blendshapes, texturas e metadados) com conformidade total à norma VRM 1.0 / glTF 2.0.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-vrm: Serializador e validador de glTF 2.0 e extensões VRM 1.0.
src/components/character/ExportModal.svelte: Configurações de autor, licença e otimização.
3. Tarefas Técnicas de Engenharia
Implementar empacotador binário `.vrm` com hierarquia de esqueleto T-Pose padronizada.
Mapear curvas de expressões faciais, molas secundárias e metadados legais de licenciamento.
Validar o modelo exportado contra o validador oficial da Khronos Group.
4. Critérios de Aceite
O arquivo `.vrm` exportado carrega perfeitamente em players e softwares compatíveis da indústria sem erros estruturais.

------------------------------------------------------------

# ANIGO - Sprint 11_ Extração e Transferência de Poses via ControlNet - DWPose.docx

ANIGO — Sprint 11: Extração e Transferência de Poses via ControlNet / DWPose
1. Objetivo
Permitir a importação de fotos, vídeos e ilustrações 2D de referência com extração automática da estrutura esquelética e aplicação imediata ao manequim 3D, além de exportar passes para ComfyUI/ControlNet.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-ai: Runtime ONNX local para inferência de DWPose e OpenPose.
src/components/posing/AIPoseMatcher.svelte: Área de drag-and-drop de imagens de pose.
3. Tarefas Técnicas de Engenharia
Integrar ONNX Runtime em Rust com aceleração DirectML/CoreML para detecção esquelética rápida.
Algoritmo de retargeting mapeando pontos de articulação 2D para rotações locais dos ossos da hierarquia.
Gerador de passes de viewport: esqueleto OpenPose colorido, mapa de profundidade e mapa de normais.
4. Critérios de Aceite
Arrastar uma imagem de pose para a janela faz o manequim 3D assumir a mesma posição em menos de 2 segundos.

------------------------------------------------------------

# ANIGO - Sprint 12_ Geração e Ancoragem de Acessórios 3D com TRELLIS.docx

ANIGO — Sprint 12: Geração e Ancoragem de Acessórios 3D com TRELLIS
1. Objetivo
Gerar malhas 3D limpas para adereços complexos (espadas, capacetes, armaduras, asas) a partir de texto ou imagem única via modelo TRELLIS, ancorando-os diretamente ao esqueleto.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-ai: Cliente TRELLIS para decodificação de latentes estruturados (SLAT).
src/components/character/AIAccessories.svelte: Painel de prompt e geração de adereços.
3. Tarefas Técnicas de Engenharia
Configurar comunicação IPC com o modelo de geração 3D e decodificação de geometria poligonal.
Implementar algoritmo de quad-remeshing automático e eliminação de polígonos internos oclusos.
Criar sistema de pontos de encaixe (*sockets*) associando o adereço a nós ósseos com transformações locais.
4. Critérios de Aceite
Inserir um conceito de adereço gera um modelo 3D fechado em ~15s fixado rigidamente à mão ou cabeça.

------------------------------------------------------------

# ANIGO - Sprint 13_ Difusão Neural de Texturas e Seam Blending 3D.docx

ANIGO — Sprint 13: Difusão Neural de Texturas e Seam Blending 3D
1. Objetivo
Sintetizar e projetar texturas cel-shade diretamente na malha do personagem sem descontinuidades ou costuras aparentes no espaço UV.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-ai: Projeção ortogonal multicâmera com inpainting e difusão anime.
shaders/seam_blending.wgsl: Shader de mesclagem de bordas de textura no espaço 3D.
3. Tarefas Técnicas de Engenharia
Implementar projeção de imagens de textura condicionadas por mapas de profundidade.
Criar algoritmo de costura euclidiana 3D para fundir e suavizar junções entre ilhas UV.
Gerar automaticamente mapas auxiliares de oclusão estática e espessura de contorno.
4. Critérios de Aceite
Texturas de pele e tecidos são aplicadas sem costuras visíveis ou quebras de resolução em 360 graus.

------------------------------------------------------------

# ANIGO - Sprint 14_ Auto-Rigging Neural e Ferramenta Sketch-to-Model.docx

ANIGO — Sprint 14: Auto-Rigging Neural e Ferramenta Sketch-to-Model
1. Objetivo
Automatizar a atribuição de pesos de deformação (*skinning weights*) para novas roupas e converter esboços manuais 2D em geometria 3D paramétrica.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-core: Solver de Voxel Heat Diffuse Skinning acelerado em Rust.
src/components/character/SketchTool.svelte: Ferramenta de caneta para esboço 3D direto.
3. Tarefas Técnicas de Engenharia
Implementar transferência automática de pesos de influência óssea do corpo para novas roupas e acessórios.
Desenvolver rede leve de visão para extrair curvas B-Spline a partir de traços livres na viewport.
4. Critérios de Aceite
Peças de roupa anexadas deformam-se junto com o esqueleto sem necessidade de pintura manual de pesos.

------------------------------------------------------------

# ANIGO - Sprint 15_ Blocagem Modular de Cenários 3D (Greybox e Snapping).docx

ANIGO — Sprint 15: Blocagem Modular de Cenários 3D (Greybox & Snapping)
1. Objetivo
Montar ambientes arquitetônicos (escolas, quartos, ruas de anime) rapidamente com primitivas paramétricas e renderização instanciada de alto rendimento.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-renderer: Scene Graph em Rust com suporte a instanced rendering no wgpu.
src/components/environment/BlockoutEditor.svelte: Ferramentas de grid snapping e biblioteca de blocos.
3. Tarefas Técnicas de Engenharia
Implementar alinhamento magnético em grade (*grid snapping*) e duplicação matricial rápida de objetos.
Construir biblioteca inicial de blocagem paramétrica (paredes com vãos, portas, janelas, escadas).
Otimizar renderização em lote para suportar milhares de instâncias a 120+ FPS.
4. Critérios de Aceite
O usuário monta o volume de uma sala de aula ou rua residencial em menos de 10 minutos com controles ágeis.

------------------------------------------------------------

# ANIGO - Sprint 16_ Shaders Especializados para Background Art (Painterly).docx

ANIGO — Sprint 16: Shaders Especializados para Background Art (Painterly)
1. Objetivo
Desenvolver shaders WGSL que emulam o acabamento pictórico tradicional de fundos de anime (aquarela, guache de estúdio e perspectiva aérea).
2. Arquitetura e Pacotes Envolvidos
shaders/painterly_background.wgsl: Shader com granulação de pigmento e dispersão atmosférica.
src/components/environment/EnvironmentShading.svelte: Controles de névoa e paleta de cenário.
3. Tarefas Técnicas de Engenharia
Implementar efeito de acúmulo de pigmento em bordas (*edge darkening*) e textura procedural de papel aquarela.
Criar névoa de perspectiva aérea que resfria e dessatura objetos distantes gradualmente.
Desenvolver shader volumétrico estilizado para folhagens de árvores (*cloud foliage*).
4. Critérios de Aceite
A geometria 3D do cenário ganha aspecto de pintura tradicional japonesa harmonizada com os personagens.

------------------------------------------------------------

# ANIGO - Sprint 17_ Camera Projection Mapping e Cenários Híbridos.docx

ANIGO — Sprint 17: Camera Projection Mapping e Cenários Híbridos
1. Objetivo
Projetar ilustrações 2D conceituais sobre geometrias 3D simplificadas, viabilizando movimentos de câmera com paralaxe natural a partir de artes pintadas.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-renderer: Shader de projeção de textura de câmera com compensação de oclusão.
src/components/environment/CameraMapping.svelte: Ferramenta de alinhamento de pontos de fuga.
3. Tarefas Técnicas de Engenharia
Desenvolver ferramenta de calibração de perspectiva para alinhar a imagem 2D com a câmera 3D.
Implementar projeção de textura com correção de estiramento em faces oblíquas.
Adicionar suporte a luzes dinâmicas secundárias (postes, lâmpadas) iluminando a pintura projetada.
4. Critérios de Aceite
A câmera pode navegar sobre o cenário 2D projetado com paralaxe realista e sem artefatos visíveis de costura.

------------------------------------------------------------

# ANIGO - Sprint 18_ Iluminação Global Estilizada e Integração Personagem-Cenário.docx

ANIGO — Sprint 18: Iluminação Global Estilizada e Integração Personagem-Cenário
1. Objetivo
Harmonizar a iluminação e paleta cromática dos personagens com o ambiente circundante através de sondas de luz e sombras projetadas no piso.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-renderer: Sistema de Stylized Light Probes e shadow mapping cel-shade.
shaders/cel_shadows.wgsl: Sombras projetadas com terminador nítido no cenário.
3. Tarefas Técnicas de Engenharia
Calcular rebatimento de cor (*Color Bleeding*) indireta das paredes e piso sobre as sombras do personagem.
Implementar sombras projetadas do personagem sobre o chão e móveis com borda nítida.
Criar ferramenta de LUT de harmonização de cores unificando primeiro e segundo plano.
4. Critérios de Aceite
O personagem não parece destacado artificialmente do fundo; suas sombras refletem a luz e cor do ambiente.

------------------------------------------------------------

# ANIGO - Sprint 19_ Linha do Tempo, Dopesheet e Animação Escalonada.docx

ANIGO — Sprint 19: Linha do Tempo, Dopesheet e Animação Escalonada
1. Objetivo
Implementar a linha do tempo de animação com suporte nativo a keyframes em degrau (*stepped timing* em 2s e 3s / 12 e 8 FPS em base 24), mimetizando o ritmo tradicional de anime.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-core: Motor de interpolação não linear e avaliação escalonada de curvas.
src/components/timeline/Dopesheet.svelte: Linha do tempo reativa com marcadores de exposição de quadros.
3. Tarefas Técnicas de Engenharia
Construir editor gráfico de timeline com canais para ossos, blendshapes e câmeras.
Implementar interpolação em degrau que segura poses por 2 ou 3 quadros e troca instantaneamente.
Suporte a curvas de interpolação não lineares (Ease-in, Ease-out, Exponential).
4. Critérios de Aceite
A reprodução de poses na viewport exibe o impacto rítmico autêntico de anime sem a fluidez plástica do 3D contínuo.

------------------------------------------------------------

# ANIGO - Sprint 20_ Deformadores de Impacto (Smear Frames) e Inbetweening.docx

ANIGO — Sprint 20: Deformadores de Impacto (Smear Frames) e Inbetweening
1. Objetivo
Implementar deformações transitórias extremas para golpes rápidos e cálculo de quadros intermediários (*inbetweens*) assistido por computador.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-core: Modificador de deformação transitória de malha e interpolador de arcos.
src/components/timeline/InbetweenAssist.svelte: Painel de geração de quadros intermediários.
3. Tarefas Técnicas de Engenharia
Criar modificador de malha para *Smear Frames* (alongamento e arqueamento drástico em 1 único quadro).
Implementar algoritmo de interpolação inteligente que preserva arcos de movimento desenhados.
Estruturar biblioteca interna de ciclos reutilizáveis (caminhada, corrida ninja, saques de espada).
4. Critérios de Aceite
Ações de combate em alta velocidade ganham peso cinético convincente através de quadros de borrão estilizados.

------------------------------------------------------------

# ANIGO - Sprint 21_ Câmera Multiplano e Gerador de Efeitos Visuais Anime FX.docx

ANIGO — Sprint 21: Câmera Multiplano e Gerador de Efeitos Visuais Anime FX
1. Objetivo
Composição de tomadas em camadas hierárquicas de profundidade com paralaxe independente e gerador paramétrico de efeitos visuais clássicos de anime.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-renderer: Câmera Multiplano e gerador procedural de partículas/fitas FX.
shaders/anime_fx.wgsl: Shaders aditivos para lâminas energéticas, faíscas e fumaça em celuloide.
3. Tarefas Técnicas de Engenharia
Separar cena em planos: Primeiro plano desfocado, Personagens, Cenário médio e Céu.
Gerar efeitos paramétricos: linhas de velocidade (*speed lines*), cortes de espada luminosos e faíscas de impacto.
Adicionar controle independente de desfoque e curvas de saturação por camada.
4. Critérios de Aceite
A cena é renderizada em múltiplas camadas de câmera com efeitos gráficos de impacto perfeitamente sincronizados.

------------------------------------------------------------

# ANIGO - Sprint 22_ Mesa de Composição, Masterização e Exportação Final.docx

ANIGO — Sprint 22: Mesa de Composição, Masterização e Exportação Final
1. Objetivo
Finalizar a produção cinematográfica da cena com pós-processamento analógico de estúdio e exportação profissional multicanal em OpenEXR de 32 bits e ProRes 4444.
2. Arquitetura e Pacotes Envolvidos
src-tauri/crates/anigo-renderer: Renderizador em lote (Batch Renderer) acelerado por GPU.
shaders/post_process.wgsl: Bloom difuso, aberração cromática, grão analógico e LUTs.
3. Tarefas Técnicas de Engenharia
Construir pipeline final de pós-processamento: bloom suave, dispersão de lente e granulação de filme 35mm.
Implementar exportação de sequências de imagens OpenEXR de 32 bits (RGB, Alfa, Depth, Normals, IDs).
Gerar arquivos de vídeo Apple ProRes 4444 com canal alfa e timecode SMPTE para suítes de edição (NLEs).
4. Critérios de Aceite
O software gera a tomada final renderizada em alta definição com qualidade cinematográfica pronta para exibição ou edição.

------------------------------------------------------------