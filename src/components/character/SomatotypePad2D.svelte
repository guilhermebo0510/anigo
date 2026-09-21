<script lang="ts">
  import {
    NEUTRAL_SOMATOTYPE,
    clampGenderDimorphism,
    normalizeSomatotype,
    sanitizeFinite,
    type SomatotypeUpdate,
  } from "../../services/character_state";

  interface Props {
    endomorph?: number;
    mesomorph?: number;
    ectomorph?: number;
    genderDimorphism?: number;
    onUpdate?: (params: SomatotypeUpdate) => void;
  }

  let {
    endomorph = $bindable(NEUTRAL_SOMATOTYPE.endo),
    mesomorph = $bindable(NEUTRAL_SOMATOTYPE.meso),
    ectomorph = $bindable(NEUTRAL_SOMATOTYPE.ecto),
    genderDimorphism = $bindable(1.0),
    onUpdate = undefined,
  }: Props = $props();

  let padSvg: SVGSVGElement | null = $state(null);
  let isDragging = $state(false);

  // Pad geometry in SVG viewBox coordinate space [0, 200] x [0, 180]
  // Top: Meso, Bottom-Left: Endo, Bottom-Right: Ecto
  const V_MESO = { x: 100, y: 20 };
  const V_ENDO = { x: 25, y: 160 };
  const V_ECTO = { x: 175, y: 160 };

  // Convert (endo, meso, ecto) to SVG coordinates (px, py)
  function coordsToSvg(e: number, m: number, ec: number): { x: number; y: number } {
    // P0-02: sanitize at the boundary — NaN/Inf can never reach the puck.
    const n = normalizeSomatotype(e, m, ec);
    return {
      x: n.endo * V_ENDO.x + n.meso * V_MESO.x + n.ecto * V_ECTO.x,
      y: n.endo * V_ENDO.y + n.meso * V_MESO.y + n.ecto * V_ECTO.y,
    };
  }

  // Convert SVG coordinates (px, py) to barycentric (endo, meso, ecto)
  function svgToCoords(px: number, py: number): { endo: number; meso: number; ecto: number } {
    const denom = (V_MESO.y - V_ECTO.y) * (V_ENDO.x - V_ECTO.x) + (V_ECTO.x - V_MESO.x) * (V_ENDO.y - V_ECTO.y);
    if (Math.abs(denom) < 1e-6) {
      return { endo: 0.33, meso: 0.34, ecto: 0.33 };
    }

    const lambda0 = ((V_MESO.y - V_ECTO.y) * (px - V_ECTO.x) + (V_ECTO.x - V_MESO.x) * (py - V_ECTO.y)) / denom;
    const lambda1 = ((V_ECTO.y - V_ENDO.y) * (px - V_ECTO.x) + (V_ENDO.x - V_ECTO.x) * (py - V_ECTO.y)) / denom;
    const lambda2 = 1.0 - lambda0 - lambda1;

    const e = Math.max(0, lambda0);
    const m = Math.max(0, lambda1);
    const ec = Math.max(0, lambda2);
    const sum = (e + m + ec) || 1.0;

    return {
      endo: e / sum,
      meso: m / sum,
      ecto: ec / sum,
    };
  }

  let puckPos = $derived(coordsToSvg(endomorph, mesomorph, ectomorph));

  // P0-02: readouts never display NaN even if a parent binds garbage.
  let safeEndo = $derived(sanitizeFinite(endomorph, NEUTRAL_SOMATOTYPE.endo));
  let safeMeso = $derived(sanitizeFinite(mesomorph, NEUTRAL_SOMATOTYPE.meso));
  let safeEcto = $derived(sanitizeFinite(ectomorph, NEUTRAL_SOMATOTYPE.ecto));
  let safeGender = $derived(clampGenderDimorphism(genderDimorphism));

  function emitUpdate(isContinuous: boolean) {
    // P0-02: normalize + clamp before every emission; the callback contract
    // guarantees finite components summing to 1 and gender in [0, 1].
    const n = normalizeSomatotype(endomorph, mesomorph, ectomorph);
    endomorph = n.endo;
    mesomorph = n.meso;
    ectomorph = n.ecto;
    genderDimorphism = clampGenderDimorphism(genderDimorphism);
    onUpdate?.({
      endomorph,
      mesomorph,
      ectomorph,
      genderDimorphism,
      isContinuous,
    });
  }

  function updateFromSvgPoint(px: number, py: number, isContinuous = true) {
    const c = svgToCoords(px, py);
    endomorph = c.endo;
    mesomorph = c.meso;
    ectomorph = c.ecto;
    emitUpdate(isContinuous);
  }

  function handlePointerDown(e: PointerEvent) {
    if (!padSvg) return;
    try {
      padSvg.setPointerCapture(e.pointerId);
    } catch (_) {}
    isDragging = true;
    const rect = padSvg.getBoundingClientRect();
    const svgX = ((e.clientX - rect.left) / rect.width) * 200;
    const svgY = ((e.clientY - rect.top) / rect.height) * 180;
    updateFromSvgPoint(svgX, svgY, true);
  }

  function handlePointerMove(e: PointerEvent) {
    if (!isDragging || !padSvg) return;
    const rect = padSvg.getBoundingClientRect();
    const svgX = ((e.clientX - rect.left) / rect.width) * 200;
    const svgY = ((e.clientY - rect.top) / rect.height) * 180;
    updateFromSvgPoint(svgX, svgY, true);
  }

  function handlePointerUp(e: PointerEvent) {
    if (isDragging) {
      if (padSvg) {
        try {
          padSvg.releasePointerCapture(e.pointerId);
        } catch (_) {}
      }
      isDragging = false;
      emitUpdate(false);
    }
  }

  function handleGenderChange(e: Event) {
    const val = parseFloat((e.target as HTMLInputElement).value);
    genderDimorphism = clampGenderDimorphism(Number.isFinite(val) ? val : genderDimorphism);
    emitUpdate(false);
  }

  function applyPreset(endo: number, meso: number, ecto: number) {
    const n = normalizeSomatotype(endo, meso, ecto);
    endomorph = n.endo;
    mesomorph = n.meso;
    ectomorph = n.ecto;
    emitUpdate(false);
  }
</script>

<div class="somatotype-pad-container">
  <div class="header-row">
    <span class="title">PAD 2D SOMATÓTIPO MACRO</span>
    <span class="badge">Heath-Carter</span>
  </div>

  <div class="pad-wrapper">
    <svg
      bind:this={padSvg}
      viewBox="0 0 200 180"
      class="somatochart-svg"
      role="application"
      aria-label="Pad 2D Somatótipo Heath-Carter"
      onpointerdown={handlePointerDown}
      onpointermove={handlePointerMove}
      onpointerup={handlePointerUp}
      onpointercancel={handlePointerUp}
    >
      <defs>
        <!-- Triangle Gradient Fill -->
        <linearGradient id="triangleGrad" x1="0%" y1="0%" x2="0%" y2="100%">
          <stop offset="0%" stop-color="#8b5cf6" stop-opacity="0.25" />
          <stop offset="100%" stop-color="#00f0ff" stop-opacity="0.10" />
        </linearGradient>
        <!-- Radial Glow behind Puck -->
        <radialGradient id="puckGlow" cx="50%" cy="50%" r="50%">
          <stop offset="0%" stop-color="#00f0ff" stop-opacity="0.8" />
          <stop offset="40%" stop-color="#8b5cf6" stop-opacity="0.4" />
          <stop offset="100%" stop-color="#8b5cf6" stop-opacity="0" />
        </radialGradient>
      </defs>

      <!-- Outer Boundary & Fill -->
      <polygon
        points="{V_MESO.x},{V_MESO.y} {V_ENDO.x},{V_ENDO.y} {V_ECTO.x},{V_ECTO.y}"
        fill="url(#triangleGrad)"
        stroke="#2d334d"
        stroke-width="1.5"
      />

      <!-- Internal Reference Grid Lines -->
      <line x1={V_MESO.x} y1={V_MESO.y} x2={100} y2={160} stroke="#1f2438" stroke-width="1" stroke-dasharray="3,3" />
      <line x1={V_ENDO.x} y1={V_ENDO.y} x2={137.5} y2={90} stroke="#1f2438" stroke-width="1" stroke-dasharray="3,3" />
      <line x1={V_ECTO.x} y1={V_ECTO.y} x2={62.5} y2={90} stroke="#1f2438" stroke-width="1" stroke-dasharray="3,3" />

      <!-- Center Neutral Crosshair -->
      <circle cx="100" cy="113.3" r="3" fill="#3b4261" />

      <!-- Corner Vertex Labels -->
      <text x={V_MESO.x} y={V_MESO.y - 6} text-anchor="middle" class="svg-label-top">MESOMORFO</text>
      <text x={V_ENDO.x - 2} y={V_ENDO.y + 14} text-anchor="start" class="svg-label-left">ENDOMORFO</text>
      <text x={V_ECTO.x + 2} y={V_ECTO.y + 14} text-anchor="end" class="svg-label-right">ECTOMORFO</text>

      <!-- Corner Indicator Nodes -->
      <circle cx={V_MESO.x} cy={V_MESO.y} r="3" fill="#a78bfa" />
      <circle cx={V_ENDO.x} cy={V_ENDO.y} r="3" fill="#38bdf8" />
      <circle cx={V_ECTO.x} cy={V_ECTO.y} r="3" fill="#34d399" />

      <!-- Interactive Puck -->
      <circle cx={puckPos.x} cy={puckPos.y} r="14" fill="url(#puckGlow)" pointer-events="none" />
      <circle cx={puckPos.x} cy={puckPos.y} r="6" fill="#00f0ff" stroke="#ffffff" stroke-width="1.5" pointer-events="none" />
    </svg>
  </div>

  <!-- Realtime Coordinate Readouts -->
  <div class="readouts-grid">
    <div class="metric-card meso">
      <span class="m-label">Músculo</span>
      <span class="m-val">{(safeMeso * 100).toFixed(0)}%</span>
    </div>
    <div class="metric-card endo">
      <span class="m-label">Gordura</span>
      <span class="m-val">{(safeEndo * 100).toFixed(0)}%</span>
    </div>
    <div class="metric-card ecto">
      <span class="m-label">Magreza</span>
      <span class="m-val">{(safeEcto * 100).toFixed(0)}%</span>
    </div>
  </div>

  <!-- Continuous Gender Dimorphism Slider -->
  <div class="gender-section">
    <div class="gender-header">
      <span class="g-title">DIMORFISMO DE GÊNERO</span>
      <span class="g-status">
        {#if safeGender > 0.65}
          Masculino ({((safeGender) * 100).toFixed(0)}%)
        {:else if safeGender < 0.35}
          Feminino ({((1 - safeGender) * 100).toFixed(0)}%)
        {:else}
          Andrógino ({((safeGender) * 100).toFixed(0)}%)
        {/if}
      </span>
    </div>
    <div class="gender-slider-row">
      <span class="g-pole">Fem</span>
      <input
        type="range"
        min="0.0"
        max="1.0"
        step="0.01"
        value={genderDimorphism}
        oninput={handleGenderChange}
        class="gender-range"
      />
      <span class="g-pole">Masc</span>
    </div>
  </div>

  <!-- Somatotype Quick Presets -->
  <div class="presets-row">
    <button class="preset-btn" onclick={() => applyPreset(0.10, 0.80, 0.10)}>Atlético</button>
    <button class="preset-btn" onclick={() => applyPreset(0.75, 0.15, 0.10)}>Corpulento</button>
    <button class="preset-btn" onclick={() => applyPreset(0.10, 0.15, 0.75)}>Esguio</button>
    <button class="preset-btn" onclick={() => applyPreset(0.33, 0.34, 0.33)}>Equilibrado</button>
  </div>
</div>

<style>
  .somatotype-pad-container {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px;
    background: #111420;
    border: 1px solid #1e2438;
    border-radius: 8px;
    user-select: none;
  }

  .header-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .title {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: #94a3b8;
  }

  .badge {
    font-size: 9px;
    font-weight: 600;
    color: #00f0ff;
    background: rgba(0, 240, 255, 0.1);
    border: 1px solid rgba(0, 240, 255, 0.3);
    padding: 1px 6px;
    border-radius: 4px;
    text-transform: uppercase;
  }

  .pad-wrapper {
    position: relative;
    width: 100%;
    aspect-ratio: 200 / 180;
    background: #0d0f18;
    border: 1px solid #1a1e30;
    border-radius: 6px;
    overflow: hidden;
    cursor: crosshair;
  }

  .somatochart-svg {
    width: 100%;
    height: 100%;
    touch-action: none;
  }

  .svg-label-top {
    font-size: 8px;
    font-weight: 700;
    fill: #a78bfa;
    letter-spacing: 0.06em;
  }

  .svg-label-left {
    font-size: 8px;
    font-weight: 700;
    fill: #38bdf8;
    letter-spacing: 0.06em;
  }

  .svg-label-right {
    font-size: 8px;
    font-weight: 700;
    fill: #34d399;
    letter-spacing: 0.06em;
  }

  .readouts-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
  }

  .metric-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 4px 6px;
    background: #141828;
    border: 1px solid #1e2438;
    border-radius: 4px;
  }

  .metric-card.meso { border-color: rgba(167, 139, 250, 0.3); }
  .metric-card.endo { border-color: rgba(56, 189, 248, 0.3); }
  .metric-card.ecto { border-color: rgba(52, 211, 153, 0.3); }

  .m-label {
    font-size: 9px;
    color: #64748b;
    text-transform: uppercase;
    font-weight: 600;
  }

  .m-val {
    font-size: 13px;
    font-weight: 700;
    color: #f1f5f9;
  }

  .metric-card.meso .m-val { color: #c4b5fd; }
  .metric-card.endo .m-val { color: #7dd3fc; }
  .metric-card.ecto .m-val { color: #6ee7b7; }

  .gender-section {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px;
    background: #141828;
    border: 1px solid #1e2438;
    border-radius: 6px;
  }

  .gender-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .g-title {
    font-size: 10px;
    font-weight: 700;
    color: #94a3b8;
  }

  .g-status {
    font-size: 10px;
    font-weight: 600;
    color: #00f0ff;
  }

  .gender-slider-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .g-pole {
    font-size: 9px;
    font-weight: 700;
    color: #64748b;
    text-transform: uppercase;
  }

  .gender-range {
    flex: 1;
    accent-color: #00f0ff;
    cursor: pointer;
  }

  .presets-row {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 4px;
  }

  .preset-btn {
    font-size: 9px;
    font-weight: 600;
    padding: 4px 0;
    background: #181d30;
    border: 1px solid #242c48;
    color: #94a3b8;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .preset-btn:hover {
    background: #242c48;
    color: #ffffff;
    border-color: #00f0ff;
  }
</style>
