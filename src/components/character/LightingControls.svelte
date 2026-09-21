<script lang="ts">

  interface Props {
    activeTool: string;
    lightAzimuth: number;
    lightElevation: number;
    lightIntensity: number;
    sunColor: string;
    shadowColorHex: string;
    hueShift: number;
    shadowSaturation?: number;
    ambientIntensity?: number;
    /** Issue #13: modo de projeção da câmera (`perspective`/`orthographic`). */
    projectionMode?: "perspective" | "orthographic";
    onProjectionChange?: (mode: "perspective" | "orthographic") => void;
    /** Issue #14: depth pre-pass do render graph (early-Z no passe cel). */
    depthPrepass?: boolean;
    onDepthPrepassChange?: (enabled: boolean) => void;
    onUpdate?: (params: {
      azimuth: number;
      elevation: number;
      intensity: number;
      sunColor: string;
      shadowColorHex: string;
      hueShift: number;
      shadowSaturation?: number;
      ambientIntensity?: number;
      isContinuous?: boolean;
    }) => void;
  }

  let {
    activeTool,
    lightAzimuth = $bindable(45),
    lightElevation = $bindable(45),
    lightIntensity = $bindable(1.0),
    sunColor = $bindable("#fff8e7"),
    shadowColorHex = $bindable("#9995be"),
    hueShift = $bindable(-15),
    shadowSaturation = $bindable(1.15),
    ambientIntensity = $bindable(0.35),
    projectionMode = "perspective",
    onProjectionChange = undefined,
    depthPrepass = true,
    onDepthPrepassChange = undefined,
    onUpdate = undefined,
  }: Props = $props();

  // Anime Solar Atmosphere Presets
  const solarPresets = [
    { name: "Meio-Dia", color: "#fff8e7", azimuth: 45, elevation: 60, intensity: 1.2, hue: -15, shadow: "#9995be" },
    { name: "Golden Hour", color: "#ffb37a", azimuth: 75, elevation: 18, intensity: 1.4, hue: -35, shadow: "#7e527f" },
    { name: "Luar Anime", color: "#b4d8e7", azimuth: 220, elevation: 40, intensity: 0.85, hue: 25, shadow: "#3a4168" },
    { name: "Estúdio High-Key", color: "#ffffff", azimuth: 0, elevation: 45, intensity: 1.0, hue: -10, shadow: "#888bb0" },
  ];

  // Shadow Tint Presets
  const shadowPresets = [
    { name: "Lavanda Fria", hex: "#9995be", hue: -15 },
    { name: "Pêssego Suave", hex: "#d89682", hue: 20 },
    { name: "Azul Noturno", hex: "#4b5680", hue: -40 },
    { name: "Ghibli Âmbar", hex: "#9e7d60", hue: 10 },
  ];

  function notifyChange(isContinuous = true) {
    onUpdate?.({
      azimuth: lightAzimuth,
      elevation: lightElevation,
      intensity: lightIntensity,
      sunColor,
      shadowColorHex,
      hueShift,
      shadowSaturation,
      ambientIntensity,
      isContinuous,
    });
  }

  function applySolarPreset(preset: typeof solarPresets[0]) {
    lightAzimuth = preset.azimuth;
    lightElevation = preset.elevation;
    lightIntensity = preset.intensity;
    sunColor = preset.color;
    hueShift = preset.hue;
    shadowColorHex = preset.shadow;
    notifyChange(false);
  }

  function applyShadowPreset(preset: typeof shadowPresets[0]) {
    shadowColorHex = preset.hex;
    hueShift = preset.hue;
    notifyChange(false);
  }
</script>

<div class="lighting-controls-container">
  {#if activeTool === "sun"}
  <!-- 1. Direção Solar e Intensidade -->
  <div class="control-group">
    <div class="group-title">DIREÇÃO</div>

    <div class="slider-row">
      <span class="label">Azimute</span>
      <input
        type="range"
        min="0"
        max="360"
        step="1"
        bind:value={lightAzimuth}
        oninput={() => notifyChange(true)}
        onchange={() => notifyChange(false)}
      />
      <span class="val-tag">{lightAzimuth}°</span>
    </div>

    <div class="slider-row">
      <span class="label">Elevação</span>
      <input
        type="range"
        min="-90"
        max="90"
        step="1"
        bind:value={lightElevation}
        oninput={() => notifyChange(true)}
        onchange={() => notifyChange(false)}
      />
      <span class="val-tag">{lightElevation > 0 ? `+${lightElevation}` : lightElevation}°</span>
    </div>
  </div>

  <div class="control-group">
    <div class="group-title">INTENSIDADE & COR</div>

    <div class="slider-row">
      <span class="label">Intensidade</span>
      <input
        type="range"
        min="0.0"
        max="3.0"
        step="0.05"
        bind:value={lightIntensity}
        oninput={() => notifyChange(true)}
        onchange={() => notifyChange(false)}
      />
      <span class="val-tag">{lightIntensity.toFixed(2)}x</span>
    </div>

    <div class="color-row">
      <span class="label">Cor do Sol</span>
      <div class="color-input-wrapper">
        <input
          type="color"
          bind:value={sunColor}
          oninput={() => notifyChange(true)}
          onchange={() => notifyChange(false)}
        />
        <span class="color-hex">{sunColor.toUpperCase()}</span>
      </div>
    </div>
  </div>

  <div class="control-group">
    <div class="group-title">PRESETS</div>
    <div class="btn-grid">
      {#each solarPresets as preset}
        <button
          type="button"
          class="btn-secondary"
          onclick={() => applySolarPreset(preset)}
          title="{preset.name}: {preset.azimuth}° az, {preset.elevation}° el"
        >
          <span class="preset-dot" style="background-color: {preset.color};"></span>
          <span>{preset.name}</span>
        </button>
      {/each}
    </div>
  </div>
  {/if}

  {#if activeTool === "shadows"}
  <div class="control-group">
    <div class="group-title">ESTILO & COR</div>

    <div class="slider-row">
      <span class="label">Matiz</span>
      <input
        type="range"
        min="-60"
        max="60"
        step="1"
        bind:value={hueShift}
        oninput={() => notifyChange(true)}
        onchange={() => notifyChange(false)}
      />
      <span class="val-tag">{hueShift > 0 ? `+${hueShift}` : hueShift}°</span>
    </div>

    <div class="slider-row">
      <span class="label">Saturação</span>
      <input
        type="range"
        min="0.0"
        max="2.5"
        step="0.05"
        bind:value={shadowSaturation}
        oninput={() => notifyChange(true)}
        onchange={() => notifyChange(false)}
      />
      <span class="val-tag">{shadowSaturation.toFixed(2)}x</span>
    </div>

    <div class="color-row">
      <span class="label">Cor da Sombra</span>
      <div class="color-input-wrapper">
        <input
          type="color"
          bind:value={shadowColorHex}
          oninput={() => notifyChange(true)}
          onchange={() => notifyChange(false)}
        />
        <span class="color-hex">{shadowColorHex.toUpperCase()}</span>
      </div>
    </div>
  </div>

  <div class="control-group">
    <div class="group-title">PRESETS</div>
    <div class="btn-grid" style="margin-top: 4px;">
      {#each shadowPresets as preset}
        <button
          type="button"
          class="btn-secondary"
          onclick={() => applyShadowPreset(preset)}
        >
          <span class="preset-dot" style="background-color: {preset.hex};"></span>
          <span>{preset.name}</span>
        </button>
      {/each}
    </div>
  </div>
  {/if}

  <!-- Issue #13: projeção da câmera — botões (e o atalho `O` no viewport).
       A troca preserva o enquadramento: o renderer casa os limites ortográficos
       com o volume perspectiva na distância do alvo. -->
  <div class="control-group">
    <div class="group-title">
      <span>PROJEÇÃO</span>
      <span class="val-tag">{projectionMode === "orthographic" ? "ORTO" : "PERSP"}</span>
    </div>
    <div class="btn-grid">
      <button
        type="button"
        class="btn-secondary"
        class:active={projectionMode === "perspective"}
        aria-pressed={projectionMode === "perspective"}
        onclick={() => onProjectionChange?.("perspective")}
      >
        Perspectiva
      </button>
      <button
        type="button"
        class="btn-secondary"
        class:active={projectionMode === "orthographic"}
        aria-pressed={projectionMode === "orthographic"}
        onclick={() => onProjectionChange?.("orthographic")}
      >
        Ortográfica
      </button>
    </div>
    <div class="hint-row">
      Silhueta e proporções na vista plana (conferência); atalho <kbd>O</kbd> no viewport.
    </div>
  </div>

  <div class="control-group">
    <div class="group-title">PASSES (RENDER GRAPH)</div>
    <label class="check-row">
      <input
        type="checkbox"
        checked={depthPrepass}
        aria-label="Depth pre-pass"
        onchange={(event) => onDepthPrepassChange?.((event.currentTarget as HTMLInputElement).checked)}
      />
      <span>
        Depth pre-pass (early-Z)
        <span class="val-tag">{depthPrepass ? "ON" : "OFF"}</span>
      </span>
    </label>
    <div class="hint-row">
      Desenha o z-buffer com o mesmo vertex shader do cel antes do sombreamento: o
      passe cel fica rejeitado por early-Z e o overdraw cai sem mudar um pixel.
    </div>
  </div>

  {#if activeTool === "ambient" || (!["sun", "shadows", "ambient"].includes(activeTool))}
  <div class="control-group">
    <div class="group-title">INTENSIDADE & COR</div>

    <div class="slider-row">
      <span class="label">Intensidade</span>
      <input
        type="range"
        min="0.0"
        max="1.5"
        step="0.05"
        bind:value={ambientIntensity}
        oninput={() => notifyChange(true)}
        onchange={() => notifyChange(false)}
      />
      <span class="val-tag">{ambientIntensity.toFixed(2)}</span>
    </div>
  </div>
  {/if}
</div>

<style>
  .lighting-controls-container {
    display: flex;
    flex-direction: column;
    gap: 16px;
    width: 100%;
  }

  .control-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: #151926;
    border: 1px solid #1e2538;
    border-radius: 6px;
    padding: 10px 12px;
  }

  .group-title {
    font-size: 0.7rem;
    font-weight: 700;
    color: #94a3b8;
    letter-spacing: 0.5px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 4px;
  }

  .slider-row {
    display: grid;
    grid-template-columns: 1fr auto;
    row-gap: 5px;
    column-gap: 8px;
    align-items: center;
    width: 100%;
  }

  .slider-row .label {
    grid-column: 1;
    font-size: 0.74rem;
    font-weight: 500;
    color: #94a3b8;
    white-space: normal;
    word-break: break-word;
  }

  .slider-row input[type="range"] {
    grid-column: 1 / -1;
    width: 100%;
    accent-color: #c084fc;
    cursor: pointer;
    margin: 1px 0;
  }

  .val-tag {
    grid-column: 2;
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    font-size: 0.72rem;
    color: #38bdf8;
    background: #0f172a;
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid #1e293b;
    min-width: 44px;
    text-align: right;
  }

  .color-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 0.74rem;
    color: #94a3b8;
    padding: 2px 0;
  }

  .color-input-wrapper {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .color-hex {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    font-size: 0.72rem;
    color: #94a3b8;
  }

  .color-row input[type="color"] {
    border: 1px solid #293246;
    background: transparent;
    width: 28px;
    height: 22px;
    border-radius: 4px;
    cursor: pointer;
  }

  .btn-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }

  .btn-secondary {
    background: #1a2030;
    border: 1px solid #283147;
    color: #cbd5e1;
    padding: 6px 8px;
    border-radius: 4px;
    font-size: 0.72rem;
    font-weight: 600;
    cursor: pointer;
    text-align: center;
    transition: all 0.15s ease;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    font-family: inherit;
  }

  .btn-secondary:hover {
    background: #242d44;
    border-color: #38bdf8;
    color: #f1f5f9;
  }

  .btn-secondary.active {
    background: #24405e;
    border-color: #38bdf8;
    color: #f1f5f9;
  }

  .check-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.74rem;
    color: #cbd5e1;
    cursor: pointer;
  }

  .check-row span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .hint-row {
    font-size: 0.68rem;
    color: #64748b;
    line-height: 1.35;
  }

  .hint-row kbd {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    font-size: 0.66rem;
    background: #0f172a;
    border: 1px solid #1e293b;
    border-radius: 3px;
    padding: 0 4px;
    color: #94a3b8;
  }

  .preset-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1px solid rgba(255, 255, 255, 0.3);
    flex-shrink: 0;
  }
</style>
