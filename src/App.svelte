<script lang="ts">
  import Viewport from "./components/viewport/Viewport.svelte";
  import UserIcon from "./components/icons/UserIcon.svelte";
  import BoneIcon from "./components/icons/BoneIcon.svelte";
  import ScissorsIcon from "./components/icons/ScissorsIcon.svelte";
  import SmileIcon from "./components/icons/SmileIcon.svelte";
  import ShirtIcon from "./components/icons/ShirtIcon.svelte";
  import BrushIcon from "./components/icons/BrushIcon.svelte";
  import BotIcon from "./components/icons/BotIcon.svelte";
  import SunIcon from "./components/icons/SunIcon.svelte";

  let viewportRef: any = $state(null);
  let activeTab = $state("personagem");
  let lightAzimuth = $state(45);
  let lightElevation = $state(45);
  let lightIntensity = $state(1.0);
  let outlineWidth = $state(3.5);
  let shadowThreshold = $state(0.5);

  async function handlePreset(preset: string) {
    if (viewportRef && viewportRef.switchPreset) {
      await viewportRef.switchPreset(preset);
    }
  }

  async function updateLighting() {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      const radAz = (lightAzimuth * Math.PI) / 180;
      const radEl = (lightElevation * Math.PI) / 180;
      const x = Math.cos(radEl) * Math.cos(radAz);
      const y = Math.sin(radEl);
      const z = Math.cos(radEl) * Math.sin(radAz);

      await invoke("set_light_params", {
        direction: [x, y, z],
        intensity: lightIntensity,
      });
      if (viewportRef) {
        viewportRef.switchPreset("mannequin");
      }
    }
  }
</script>

<div class="app-layout">
  <!-- Top Application Bar -->
  <header class="app-header">
    <div class="brand">
      <span class="brand-name">ANIGO</span>
      <span class="brand-tag">STUDIO v0.1.0 • RUST + wgpu</span>
    </div>
    <div class="tabs">
      <button class="tab-btn" class:active={activeTab === "personagem"} onclick={() => activeTab = "personagem"}>
        PERSONAGEM
      </button>
      <button class="tab-btn" class:active={activeTab === "shader"} onclick={() => activeTab = "shader"}>
        CEL-SHADING & LUZ
      </button>
      <button class="tab-btn" class:active={activeTab === "mcp"} onclick={() => activeTab = "mcp"}>
        MCP AGENTES
      </button>
    </div>
    <div class="header-actions">
      <button class="btn-primary" onclick={() => handlePreset("mannequin")}>RESET POSE</button>
    </div>
  </header>

  <!-- Main Body -->
  <div class="workspace-body">
    <!-- Left Navigation Toolbar -->
    <aside class="left-toolbar">
      <button class="tool-btn active" title="Manequim e Proporções"><UserIcon /></button>
      <button class="tool-btn" title="Posing e Cinemática Inversa"><BoneIcon /></button>
      <button class="tool-btn" title="Cabelo Procedural Spline"><ScissorsIcon /></button>
      <button class="tool-btn" title="Expressões Faciais e Decalques"><SmileIcon /></button>
      <button class="tool-btn" title="Vestuário e Alfaiataria"><ShirtIcon /></button>
      <button class="tool-btn" title="Pintura de Textura 3D"><BrushIcon /></button>
      <div class="tool-spacer"></div>
      <button class="tool-btn mcp-indicator" title="Servidor anigo-mcp Nativo Ativo"><BotIcon /></button>
    </aside>

    <!-- Center 3D Viewport -->
    <main class="viewport-area">
      <Viewport bind:this={viewportRef} />
    </main>

    <!-- Right Inspector Panel -->
    <aside class="right-inspector">
      <div class="panel-header">PARÂMETROS & PROPRIEDADES</div>

      {#if activeTab === "personagem"}
        <div class="control-group">
          <label>PRESET DE TESTE</label>
          <div class="btn-row">
            <button class="btn-secondary" onclick={() => handlePreset("mannequin")}>Manequim</button>
            <button class="btn-secondary" onclick={() => handlePreset("sphere")}>Esfera NPR</button>
            <button class="btn-secondary" onclick={() => handlePreset("cube")}>Cubo</button>
          </div>
        </div>

        <div class="control-group">
          <label>PROPORÇÕES ANATÔMICAS</label>
          <div class="slider-row">
            <span>Escala da Cabeça</span>
            <input type="range" min="0.7" max="1.4" step="0.05" value="1.0" />
          </div>
          <div class="slider-row">
            <span>Régua de Cabeças</span>
            <input type="range" min="2" max="8.5" step="0.5" value="6.5" />
          </div>
        </div>

        <div class="control-group">
          <label>CONTOURS (INVERTED HULL)</label>
          <div class="slider-row">
            <span>Espessura do Traço</span>
            <input type="range" min="0.5" max="8.0" step="0.5" bind:value={outlineWidth} />
            <span class="val-tag">{outlineWidth}px</span>
          </div>
        </div>
      {:else if activeTab === "shader"}
        <div class="control-group">
          <label>ILUMINAÇÃO CEL-SHADING</label>
          <div class="slider-row">
            <span>Azimute da Luz</span>
            <input type="range" min="0" max="360" bind:value={lightAzimuth} oninput={updateLighting} />
            <span class="val-tag">{lightAzimuth}°</span>
          </div>
          <div class="slider-row">
            <span>Elevação da Luz</span>
            <input type="range" min="0" max="90" bind:value={lightElevation} oninput={updateLighting} />
            <span class="val-tag">{lightElevation}°</span>
          </div>
          <div class="slider-row">
            <span>Intensidade</span>
            <input type="range" min="0.1" max="2.5" step="0.1" bind:value={lightIntensity} oninput={updateLighting} />
            <span class="val-tag">{lightIntensity}x</span>
          </div>
          <div class="slider-row">
            <span>Limiar de Sombra</span>
            <input type="range" min="0.1" max="0.9" step="0.05" bind:value={shadowThreshold} />
            <span class="val-tag">{shadowThreshold}</span>
          </div>
        </div>
      {:else if activeTab === "mcp"}
        <div class="control-group">
          <label>ANIGO MCP SERVER</label>
          <p class="desc-text">
            O servidor <code>anigo-mcp</code> está conectado via stdio ao Antigravity. Ele permite inspeção de grafos, renderização headless e testes visuais autônomos.
          </p>
          <div class="mcp-card">
            <div class="status-dot"></div>
            <div>
              <div class="mcp-title">anigo-mcp v0.1.0</div>
              <div class="mcp-sub">WebGPU Headless Engine • 7 Ferramentas Ativas</div>
            </div>
          </div>
        </div>
      {/if}
    </aside>
  </div>

  <!-- Bottom Status Bar -->
  <footer class="app-footer">
    <div class="status-left">
      <span class="status-badge">● MOTOR PRONTO</span>
      <span>wgpu 24.0 (Vulkan / DX12)</span>
      <span>•</span>
      <span>Antigravity Autonomous MCP: CONECTADO</span>
    </div>
    <div class="status-right">
      <span>C:\ANIGO</span>
    </div>
  </footer>
</div>

<style>
  .app-layout {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    background-color: #0d0f15;
    color: #e2e8f0;
    font-size: 0.85rem;
  }
  .app-header {
    height: 42px;
    background: #141721;
    border-bottom: 1px solid #1e2433;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 16px;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .brand-name {
    font-weight: 800;
    letter-spacing: 2px;
    color: #c084fc;
    font-size: 1.1rem;
  }
  .brand-tag {
    font-size: 0.7rem;
    color: #64748b;
    font-weight: 600;
  }
  .tabs {
    display: flex;
    gap: 4px;
  }
  .tab-btn {
    background: transparent;
    border: none;
    color: #94a3b8;
    padding: 6px 14px;
    border-radius: 4px;
    cursor: pointer;
    font-size: 0.78rem;
    font-weight: 600;
    transition: all 0.15s ease;
  }
  .tab-btn:hover {
    color: #f1f5f9;
    background: rgba(255, 255, 255, 0.04);
  }
  .tab-btn.active {
    color: #c084fc;
    background: rgba(192, 132, 252, 0.12);
  }
  .btn-primary {
    background: #9333ea;
    color: white;
    border: none;
    padding: 5px 12px;
    border-radius: 4px;
    font-weight: 600;
    font-size: 0.75rem;
    cursor: pointer;
  }
  .btn-primary:hover {
    background: #a855f7;
  }
  .workspace-body {
    flex: 1;
    display: flex;
    overflow: hidden;
  }
  .left-toolbar {
    width: 48px;
    background: #11141c;
    border-right: 1px solid #1e2433;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 10px 0;
    gap: 8px;
  }
  .tool-btn {
    width: 36px;
    height: 36px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    cursor: pointer;
    font-size: 1.1rem;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: background 0.15s ease;
  }
  .tool-btn:hover {
    background: #1e2433;
  }
  .tool-btn.active {
    background: #282e42;
    border-color: #a855f7;
  }
  .tool-spacer {
    flex: 1;
  }
  .mcp-indicator {
    background: rgba(56, 189, 248, 0.1);
    border-color: rgba(56, 189, 248, 0.3);
  }
  .viewport-area {
    flex: 1;
    position: relative;
    background: #000;
  }
  .right-inspector {
    width: 300px;
    background: #141721;
    border-left: 1px solid #1e2433;
    padding: 16px;
    overflow-y: auto;
  }
  .panel-header {
    font-size: 0.75rem;
    font-weight: 700;
    letter-spacing: 1px;
    color: #64748b;
    margin-bottom: 16px;
  }
  .control-group {
    margin-bottom: 20px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .control-group label {
    font-size: 0.72rem;
    font-weight: 700;
    color: #94a3b8;
    letter-spacing: 0.5px;
  }
  .btn-row {
    display: flex;
    gap: 6px;
  }
  .btn-secondary {
    flex: 1;
    background: #1e2433;
    border: 1px solid #2e384d;
    color: #cbd5e1;
    padding: 6px;
    border-radius: 4px;
    font-size: 0.75rem;
    cursor: pointer;
  }
  .btn-secondary:hover {
    background: #2a3346;
    color: white;
  }
  .slider-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    color: #94a3b8;
    font-size: 0.78rem;
  }
  .slider-row input[type="range"] {
    flex: 1;
  }
  .val-tag {
    font-family: monospace;
    color: #38bdf8;
    min-width: 42px;
    text-align: right;
  }
  .desc-text {
    color: #94a3b8;
    font-size: 0.75rem;
    line-height: 1.4;
  }
  .mcp-card {
    display: flex;
    align-items: center;
    gap: 10px;
    background: #1c2130;
    border: 1px solid #283147;
    border-radius: 6px;
    padding: 10px;
    margin-top: 8px;
  }
  .status-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #10b981;
    box-shadow: 0 0 6px #10b981;
  }
  .mcp-title {
    font-weight: 700;
    color: #f1f5f9;
  }
  .mcp-sub {
    font-size: 0.7rem;
    color: #64748b;
  }
  .app-footer {
    height: 24px;
    background: #0d0f15;
    border-top: 1px solid #1a1e2b;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px;
    font-size: 0.7rem;
    color: #64748b;
  }
  .status-left {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .status-badge {
    color: #10b981;
    font-weight: bold;
  }
</style>
