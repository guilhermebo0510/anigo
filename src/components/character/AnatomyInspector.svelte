<script lang="ts">
  import SomatotypePad2D from "./SomatotypePad2D.svelte";
  import { CANONICAL_SLIDERS, CANONICAL_ZONES, type MorphSlider, type MorphZone } from "../../services/morph_catalog";
  import { CANONICAL_CHARACTER_PRESETS, type CharacterPreset } from "../../services/character_presets";
  import {
    GENDER_MALE,
    NEUTRAL_SOMATOTYPE,
    clampCatalog,
    clampGenderDimorphism,
    createDefaultCharacterState,
    genderToDimorphism,
    getSliderDef,
    isKnownSliderId,
    normalizeSomatotype,
    type BaseGender,
    type CharacterState,
    type SomatotypeUpdate,
  } from "../../services/character_state";

  let {
    viewportRef = null,
    onModelChange = undefined,
    onCharacterChange = undefined,
    onCharacterCommit = undefined,
    onError = undefined,
    onProportionsChange = undefined,
    // Fase 2 (#43): motor de olhos/íris (estado pertence ao App).
    eyeEnabled = $bindable(false),
    eyeDepthScale = $bindable(0.0),
    eyeHighlightIntensity = $bindable(0.0),
    gazeTrackingEnabled = $bindable(false),
    gazeSaccadeAmplitude = $bindable(2.5),
    gazeDamping = $bindable(6.0),
    onEyeChange = undefined,
  }: {
    viewportRef?: any;
    onModelChange?: (gender: "male" | "female") => void;
    /** Fired on every mutation (continuous) with the full character state. */
    onCharacterChange?: (state: CharacterState) => void;
    /** Fired once per committed gesture for a single history entry. */
    onCharacterCommit?: (description: string) => void;
    /** User-visible failures (preset apply, model load). */
    onError?: (message: string) => void;
    /** Preset proportions (App owns the proportion mirrors). */
    onProportionsChange?: (p: CharacterPreset["proportions"]) => void;
    /** Fase 2 (#43): parâmetros do olho anime / solver de olhar. */
    eyeEnabled?: boolean;
    eyeDepthScale?: number;
    eyeHighlightIntensity?: number;
    gazeTrackingEnabled?: boolean;
    gazeSaccadeAmplitude?: number;
    gazeDamping?: number;
    onEyeChange?: (params: {
      eyeEnabled: boolean;
      eyeDepthScale: number;
      eyeHighlightIntensity: number;
      gazeTrackingEnabled: boolean;
      gazeSaccadeAmplitude: number;
      gazeDamping: number;
      isContinuous: boolean;
    }) => void;
  } = $props();

  let baseGender: BaseGender = $state("male");
  let searchQuery = $state("");
  let openZones: Record<string, boolean> = $state({
    GlobalSilhouette: true,
    Craniofacial: true,
  });

  // Reactive slider values map: id -> current value
  let sliderValues: Record<string, number> = $state(
    CANONICAL_ZONES.flatMap(z => z.sliders).reduce((acc, s) => {
      acc[s.id] = s.defaultValue;
      return acc;
    }, {} as Record<string, number>)
  );

  // P0-02/P1-10: canonical neutral domain (barycentric sum = 1), never (0,0,0).
  let somatotypeEndo = $state(NEUTRAL_SOMATOTYPE.endo);
  let somatotypeMeso = $state(NEUTRAL_SOMATOTYPE.meso);
  let somatotypeEcto = $state(NEUTRAL_SOMATOTYPE.ecto);
  // P0-03: canonical polarity — 1.0 = Male (matches Rust + presets + pad).
  let genderDimorphism = $state(GENDER_MALE);
  let activePresetId = $state<string | null>(null);
  let modelLoading = $state(false);
  let modelError: string | null = $state(null);

  const SLIDER_COUNT = CANONICAL_SLIDERS.length;

  // Filtering
  let filteredZones = $derived(
    CANONICAL_ZONES.map(z => {
      const filteredSliders = z.sliders.filter(s => {
        // Search query filter
        if (searchQuery.trim() !== "") {
          const q = searchQuery.toLowerCase();
          return s.name.toLowerCase().includes(q) || s.id.toLowerCase().includes(q);
        }
        return true;
      });

      return {
        ...z,
        sliders: filteredSliders,
      };
    }).filter(z => z.sliders.length > 0)
  );

  function toggleZone(key: string) {
    openZones[key] = !openZones[key];
  }

  // -- P0-07: full character state export (history + project snapshots) --
  export function getCharacterState(): CharacterState {
    const somatotype = normalizeSomatotype(somatotypeEndo, somatotypeMeso, somatotypeEcto);
    const morphSliders: Record<string, number> = {};
    for (const [id, v] of Object.entries(sliderValues)) {
      const def = getSliderDef(id);
      if (!def) continue;
      if (v !== def.defaultValue && Number.isFinite(v)) morphSliders[id] = v;
    }
    const base = createDefaultCharacterState();
    return {
      ...base,
      baseGender,
      activePresetId,
      somatotype,
      genderDimorphism: clampGenderDimorphism(genderDimorphism),
      morphSliders,
    };
  }

  // -- App → inspector sync (undo/redo, project load, tactile, MCP) --
  export function setCharacterState(state: CharacterState) {
    baseGender = state.baseGender === "female" ? "female" : "male";
    activePresetId = state.activePresetId;
    somatotypeEndo = state.somatotype.endo;
    somatotypeMeso = state.somatotype.meso;
    somatotypeEcto = state.somatotype.ecto;
    genderDimorphism = state.genderDimorphism;
    for (const zone of CANONICAL_ZONES) {
      for (const s of zone.sliders) {
        const v = state.morphSliders[s.id];
        sliderValues[s.id] = v !== undefined ? v : s.defaultValue;
      }
    }
    pushAllToViewport();
  }

  /** External single-slider write (tactile drag, MCP) with catalog clamp. */
  export function applyExternalMorph(id: string, value: number): boolean {
    if (!isKnownSliderId(id)) return false;
    const clamped = clampCatalog(id, value);
    if (clamped === null) return false;
    sliderValues[id] = clamped;
    viewportRef?.setMorphSlider?.(id, clamped);
    activePresetId = null;
    return true;
  }

  export function applyExternalSomatotype(endo: number, meso: number, ecto: number, gender?: number) {
    const n = normalizeSomatotype(endo, meso, ecto);
    somatotypeEndo = n.endo;
    somatotypeMeso = n.meso;
    somatotypeEcto = n.ecto;
    if (gender !== undefined) genderDimorphism = clampGenderDimorphism(gender);
    viewportRef?.setSomatotype?.(somatotypeEndo, somatotypeMeso, somatotypeEcto);
    if (gender !== undefined) viewportRef?.setGenderDimorphism?.(genderDimorphism);
    activePresetId = null;
  }

  function pushAllToViewport() {
    if (!viewportRef) return;
    viewportRef.setSomatotype?.(somatotypeEndo, somatotypeMeso, somatotypeEcto);
    viewportRef.setGenderDimorphism?.(genderDimorphism);
    for (const zone of CANONICAL_ZONES) {
      for (const s of zone.sliders) {
        viewportRef.setMorphSlider?.(s.id, sliderValues[s.id] ?? s.defaultValue);
      }
    }
  }

  function notifyChange() {
    onCharacterChange?.(getCharacterState());
  }

  function notifyCommit(description: string) {
    onCharacterCommit?.(description);
  }

  function handleSliderInput(slider: MorphSlider, value: number) {
    // P0-09: central clamp on every write path.
    const clamped = clampCatalog(slider.id, value);
    if (clamped === null) return;
    sliderValues[slider.id] = clamped;
    activePresetId = null;
    viewportRef?.setMorphSlider?.(slider.id, clamped);
    notifyChange();
  }

  function handleSliderCommit(slider: MorphSlider) {
    notifyCommit(`Ajustar ${slider.name}`);
  }

  async function handleGenderChange(gender: "male" | "female") {
    if (modelLoading) return;
    // P0-03: canonical polarity via the single shared helper.
    baseGender = gender;
    genderDimorphism = genderToDimorphism(gender);
    modelError = null;
    if (viewportRef) {
      if (typeof viewportRef.loadCanonicalModel === "function") {
        // P2-04: awaited + guarded load with visible state (was fire-and-forget).
        modelLoading = true;
        try {
          await viewportRef.loadCanonicalModel(gender);
        } catch (e) {
          const msg = `Falha ao carregar modelo ${gender}: ${e instanceof Error ? e.message : String(e)}`;
          modelError = msg;
          onError?.(msg);
        } finally {
          modelLoading = false;
        }
      }
      if (typeof viewportRef.setGenderDimorphism === "function") {
        viewportRef.setGenderDimorphism(genderDimorphism);
      }
    }
    activePresetId = null;
    onModelChange?.(gender);
    notifyChange();
    notifyCommit(`Trocar modelo base (${gender === "female" ? "Feminino" : "Masculino"})`);
  }

  // P0-02: object callback matching SomatotypePad2D's SomatotypeUpdate —
  // positional params were the NaN-corruption vector. All values sanitized.
  function handleSomatotype(update: SomatotypeUpdate) {
    const n = normalizeSomatotype(update.endomorph, update.mesomorph, update.ectomorph);
    somatotypeEndo = n.endo;
    somatotypeMeso = n.meso;
    somatotypeEcto = n.ecto;
    genderDimorphism = clampGenderDimorphism(update.genderDimorphism);
    activePresetId = null;
    if (viewportRef) {
      if (typeof viewportRef.setSomatotype === "function") {
        viewportRef.setSomatotype(somatotypeEndo, somatotypeMeso, somatotypeEcto);
      }
      if (typeof viewportRef.setGenderDimorphism === "function") {
        viewportRef.setGenderDimorphism(genderDimorphism);
      }
    }
    notifyChange();
    if (!update.isContinuous) notifyCommit("Ajustar somatótipo");
  }

  // P0-01: atomic preset application against the REAL CharacterPreset
  // contract (model / somatotype.{endo,meso,ecto} / proportions / sliders).
  async function handleApplyPreset(preset: CharacterPreset) {
    if (modelLoading) return;
    modelError = null;
    try {
      // (a) reset to canonical defaults first (atomic base)
      resetSlidersToDefaults(false);
      // (b) awaited model swap with error surfacing
      if (preset.model !== baseGender) {
        baseGender = preset.model;
        modelLoading = true;
        try {
          if (typeof viewportRef?.loadCanonicalModel === "function") {
            await viewportRef.loadCanonicalModel(preset.model);
          }
        } finally {
          modelLoading = false;
        }
        onModelChange?.(preset.model);
      }
      // (c) somatotype (canonical polarity for gender)
      const n = normalizeSomatotype(
        preset.somatotype.endo,
        preset.somatotype.meso,
        preset.somatotype.ecto
      );
      somatotypeEndo = n.endo;
      somatotypeMeso = n.meso;
      somatotypeEcto = n.ecto;
      genderDimorphism = clampGenderDimorphism(preset.genderDimorphism);
      viewportRef?.setSomatotype?.(somatotypeEndo, somatotypeMeso, somatotypeEcto);
      viewportRef?.setGenderDimorphism?.(genderDimorphism);
      // (d) proportions (viewport + App mirrors stay in sync)
      viewportRef?.setProportions?.({ ...preset.proportions });
      onProportionsChange?.({ ...preset.proportions });
      // (e) sliders (catalog-clamped, unknown ids skipped loudly)
      for (const [key, val] of Object.entries(preset.sliders ?? {})) {
        const clamped = clampCatalog(key, val);
        if (clamped === null) {
          console.warn(`[AnatomyInspector] preset "${preset.id}": unknown slider "${key}" skipped`);
          continue;
        }
        sliderValues[key] = clamped;
        viewportRef?.setMorphSlider?.(key, clamped);
      }
      activePresetId = preset.id;
      // (f) ONE history entry for the whole atomic operation
      notifyChange();
      notifyCommit(`Aplicar preset ${preset.name}`);
    } catch (e) {
      const msg = `Falha ao aplicar preset ${preset.name}: ${e instanceof Error ? e.message : String(e)}`;
      modelError = msg;
      onError?.(msg);
    }
  }

  function resetSlidersToDefaults(withCommit: boolean) {
    for (const zone of CANONICAL_ZONES) {
      for (const s of zone.sliders) {
        sliderValues[s.id] = s.defaultValue;
        viewportRef?.setMorphSlider?.(s.id, s.defaultValue);
      }
    }
    somatotypeEndo = NEUTRAL_SOMATOTYPE.endo;
    somatotypeMeso = NEUTRAL_SOMATOTYPE.meso;
    somatotypeEcto = NEUTRAL_SOMATOTYPE.ecto;
    genderDimorphism = genderToDimorphism(baseGender);
    viewportRef?.setSomatotype?.(somatotypeEndo, somatotypeMeso, somatotypeEcto);
    viewportRef?.setGenderDimorphism?.(genderDimorphism);
    activePresetId = null;
    notifyChange();
    if (withCommit) notifyCommit("Resetar sliders anatômicos");
  }

  function resetAllSliders() {
    resetSlidersToDefaults(true);
  }

  // Fase 2 (#43): parâmetros do olho anime / solver de olhar (material).
  function notifyEyeChange(isContinuous = true) {
    onEyeChange?.({
      eyeEnabled,
      eyeDepthScale,
      eyeHighlightIntensity,
      gazeTrackingEnabled,
      gazeSaccadeAmplitude,
      gazeDamping,
      isContinuous,
    });
  }
</script>

<div class="anatomy-inspector">
  <!-- Header: Base Model Selector -->
  <div class="header-card">
    <div class="card-title">MODELO CANÔNICO BASE (SPRINT 03)</div>
    <div class="gender-toggle-row">
      <button
        type="button"
        class="gender-btn"
        class:active={baseGender === "male"}
        disabled={modelLoading}
        onclick={() => handleGenderChange("male")}
      >
        <span class="gender-icon">♂</span>
        <span>Masculino Base</span>
        <span class="poly-badge">4.070 verts</span>
      </button>
      <button
        type="button"
        class="gender-btn"
        class:active={baseGender === "female"}
        disabled={modelLoading}
        onclick={() => handleGenderChange("female")}
      >
        <span class="gender-icon">♀</span>
        <span>Feminino Base</span>
        <span class="poly-badge">4.070 verts</span>
      </button>
    </div>
    {#if modelLoading}
      <div class="model-status loading">Carregando modelo canônico…</div>
    {/if}
    {#if modelError}
      <div class="model-status error" role="alert">{modelError}</div>
    {/if}
  </div>

  <!-- Character Presets -->
  <div class="presets-card">
    <div class="card-header-row">
      <span class="card-title">PRESETS DE FÁBRICA</span>
      <button type="button" class="btn-reset-all" onclick={resetAllSliders} title="Restaurar padrão">
        Resetar Sliders
      </button>
    </div>
    <div class="presets-grid">
      {#each CANONICAL_CHARACTER_PRESETS as p (p.id)}
        <button
          type="button"
          class="preset-chip"
          class:active={activePresetId === p.id}
          onclick={() => handleApplyPreset(p)}
          title={p.description}
        >
          {p.name}
        </button>
      {/each}
    </div>
  </div>

  <!-- Somatotype 2D Pad -->
  <div class="somatotype-container">
    <SomatotypePad2D
      bind:endomorph={somatotypeEndo}
      bind:mesomorph={somatotypeMeso}
      bind:ectomorph={somatotypeEcto}
      bind:genderDimorphism={genderDimorphism}
      onUpdate={handleSomatotype}
    />
  </div>

  <!-- Fase 2 (#43): Motor de Olhos/Íris (Anime Eye) -->
  <div class="eyes-card">
    <div class="card-title">OLHOS & OLHAR (ANIME)</div>

    <div class="eye-row">
      <span class="eye-label">Olho Anime (Parallax + Highlights)</span>
      <input
        type="checkbox"
        bind:checked={eyeEnabled}
        oninput={() => notifyEyeChange(true)}
        onchange={() => notifyEyeChange(false)}
      />
    </div>

    <div class="eye-slider-row">
      <span class="eye-label">Profundidade da Íris</span>
      <input
        type="range"
        min="0"
        max="0.25"
        step="0.005"
        bind:value={eyeDepthScale}
        oninput={() => notifyEyeChange(true)}
        onchange={() => notifyEyeChange(false)}
      />
      <span class="eye-value">{eyeDepthScale.toFixed(3)}</span>
    </div>

    <div class="eye-slider-row">
      <span class="eye-label">Highlights</span>
      <input
        type="range"
        min="0"
        max="1.5"
        step="0.05"
        bind:value={eyeHighlightIntensity}
        oninput={() => notifyEyeChange(true)}
        onchange={() => notifyEyeChange(false)}
      />
      <span class="eye-value">{eyeHighlightIntensity.toFixed(2)}</span>
    </div>

    <div class="card-subtitle">TRACKING DO OLHAR</div>

    <div class="eye-row">
      <span class="eye-label">Seguir Câmera (Look-At)</span>
      <input
        type="checkbox"
        bind:checked={gazeTrackingEnabled}
        oninput={() => notifyEyeChange(true)}
        onchange={() => notifyEyeChange(false)}
      />
    </div>

    <div class="eye-slider-row">
      <span class="eye-label">Micro-Sacadas</span>
      <input
        type="range"
        min="0"
        max="5"
        step="0.1"
        bind:value={gazeSaccadeAmplitude}
        oninput={() => notifyEyeChange(true)}
        onchange={() => notifyEyeChange(false)}
      />
      <span class="eye-value">{gazeSaccadeAmplitude.toFixed(1)}°</span>
    </div>

    <div class="eye-slider-row">
      <span class="eye-label">Damping</span>
      <input
        type="range"
        min="1"
        max="15"
        step="0.5"
        bind:value={gazeDamping}
        oninput={() => notifyEyeChange(true)}
        onchange={() => notifyEyeChange(false)}
      />
      <span class="eye-value">{gazeDamping.toFixed(1)}</span>
    </div>

    <div class="eye-hint">
      Parallax: a íris se afunda conforme a câmera orbita (sem cavidade
      geométrica). Highlights: brilhos desenhados à mão que permanecem em
      sombra total. Tracking: o olhar segue a câmera com micro-sacadas (0–5°)
      aplicadas aos nós LeftEye/RightEye do modelo (VRM).
    </div>
  </div>

  <!-- Slider Search Filter -->
  <div class="search-bar">
    <span class="search-icon">🔍</span>
    <input
      type="text"
      bind:value={searchQuery}
      placeholder="Filtrar {SLIDER_COUNT} sliders anatômicos..."
      class="search-input"
    />
    {#if searchQuery}
      <button type="button" class="clear-search" onclick={() => searchQuery = ""}>✕</button>
    {/if}
  </div>

  <!-- 18 Anatomical Zones Accordion -->
  <div class="accordions-list">
    {#each filteredZones as zone (zone.key)}
      <div class="zone-accordion" class:open={openZones[zone.key] ?? false}>
        <button
          type="button"
          class="zone-header"
          onclick={() => toggleZone(zone.key)}
          aria-expanded={openZones[zone.key] ?? false}
        >
          <span class="chevron">{openZones[zone.key] ? "▼" : "▶"}</span>
          <span class="zone-name">{zone.name}</span>
          <span class="slider-count-badge">{zone.sliders.length}</span>
        </button>

        {#if openZones[zone.key]}
          <div class="zone-body">
            {#each zone.sliders as slider (slider.id)}
              <div class="slider-item">
                <div class="slider-meta">
                  <span class="slider-label" title={`${slider.id} · geometria canônica do núcleo · [${slider.min}, ${slider.max}]`}>{slider.name}</span>
                  <div class="slider-val-wrap">
                    {#if slider.dimorphism === "MaleOnly"}
                      <span class="tag-dimorphic male" title="Exclusivo Masculino">♂</span>
                    {:else if slider.dimorphism === "FemaleOnly"}
                      <span class="tag-dimorphic female" title="Exclusivo Feminino">♀</span>
                    {/if}
                    <span class="val-num">
                      {sliderValues[slider.id]?.toFixed(slider.step >= 1 ? 0 : 2)}
                    </span>
                  </div>
                </div>
                <div class="slider-control-row">
                  <input
                    type="range"
                    min={slider.min}
                    max={slider.max}
                    step={slider.step}
                    value={sliderValues[slider.id] ?? slider.defaultValue}
                    oninput={(e) => handleSliderInput(slider, parseFloat((e.target as HTMLInputElement).value))}
                    onchange={() => handleSliderCommit(slider)}
                    class="anigo-slider"
                  />
                  {#if (sliderValues[slider.id] ?? slider.defaultValue) !== slider.defaultValue}
                    <button
                      type="button"
                      class="btn-reset-slider"
                      title="Restaurar valor padrão"
                      onclick={() => handleSliderInput(slider, slider.defaultValue)}
                    >
                      ↺
                    </button>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    {/each}
  </div>
</div>

<style>
  .anatomy-inspector {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px;
    color: #e2e8f0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    font-size: 13px;
    box-sizing: border-box;
    width: 100%;
  }

  .header-card, .presets-card {
    background: #131722;
    border: 1px solid #232a3b;
    border-radius: 8px;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .card-title {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    color: #94a3b8;
    text-transform: uppercase;
  }

  .card-header-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .btn-reset-all {
    background: transparent;
    border: 1px solid #334155;
    color: #94a3b8;
    font-size: 10px;
    padding: 2px 6px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.15s ease;
  }
  .btn-reset-all:hover {
    border-color: #ef4444;
    color: #f87171;
  }

  .gender-toggle-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .gender-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px;
    background: #1a2234;
    border: 1px solid #2a364f;
    border-radius: 6px;
    color: #94a3b8;
    cursor: pointer;
    font-weight: 600;
    font-size: 12px;
    transition: all 0.15s ease;
  }
  .gender-btn:hover {
    background: #232e46;
    color: #f1f5f9;
  }
  .gender-btn.active {
    background: #8b5cf6;
    border-color: #a78bfa;
    color: #ffffff;
    box-shadow: 0 0 12px rgba(139, 92, 246, 0.35);
  }

  .gender-icon {
    font-size: 14px;
  }

  .poly-badge {
    font-size: 9px;
    background: rgba(0, 0, 0, 0.3);
    padding: 1px 4px;
    border-radius: 3px;
    opacity: 0.85;
  }

  .gender-btn:disabled {
    opacity: 0.5;
    cursor: wait;
  }

  .model-status {
    font-size: 11px;
    padding: 6px 8px;
    border-radius: 4px;
  }
  .model-status.loading {
    color: #7dd3fc;
    background: rgba(56, 189, 248, 0.08);
    border: 1px solid rgba(56, 189, 248, 0.3);
  }
  .model-status.error {
    color: #fca5a5;
    background: rgba(239, 68, 68, 0.08);
    border: 1px solid rgba(239, 68, 68, 0.4);
    word-break: break-word;
  }

  .presets-grid {
    display: grid;
    grid-template-columns: repeat(2, 1fr);
    gap: 6px;
  }

  .preset-chip {
    padding: 5px 8px;
    background: #1a2234;
    border: 1px solid #2a364f;
    border-radius: 4px;
    color: #cbd5e1;
    font-size: 11px;
    cursor: pointer;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: all 0.15s ease;
  }
  .preset-chip:hover {
    background: #232e46;
    border-color: #8b5cf6;
  }
  .preset-chip.active {
    background: #2e1065;
    border-color: #a855f7;
    color: #e9d5ff;
  }

  .somatotype-container {
    width: 100%;
  }

  .search-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    background: #131722;
    border: 1px solid #232a3b;
    border-radius: 6px;
    padding: 4px 8px;
  }
  .search-icon {
    font-size: 12px;
    opacity: 0.6;
  }
  .search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: #f1f5f9;
    font-size: 12px;
  }
  .clear-search {
    background: none;
    border: none;
    color: #64748b;
    cursor: pointer;
    font-size: 11px;
  }

  .accordions-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .zone-accordion {
    background: #131722;
    border: 1px solid #232a3b;
    border-radius: 6px;
    overflow: hidden;
  }
  .zone-accordion.open {
    border-color: #334155;
  }

  .zone-header {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    background: transparent;
    border: none;
    color: #cbd5e1;
    font-weight: 600;
    font-size: 12px;
    text-align: left;
    cursor: pointer;
    transition: background 0.15s ease;
  }
  .zone-header:hover {
    background: #1a2234;
    color: #ffffff;
  }

  .chevron {
    font-size: 9px;
    color: #8b5cf6;
  }

  .zone-name {
    flex: 1;
  }

  .slider-count-badge {
    font-size: 10px;
    background: #1e293b;
    color: #94a3b8;
    padding: 1px 6px;
    border-radius: 10px;
    font-weight: normal;
  }

  .zone-body {
    padding: 8px 10px 12px;
    border-top: 1px solid #1e293b;
    display: flex;
    flex-direction: column;
    gap: 10px;
    background: #0d111a;
  }

  .slider-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .slider-meta {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .slider-label {
    font-size: 11px;
    color: #94a3b8;
  }

  .slider-val-wrap {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .val-num {
    font-size: 11px;
    font-family: "JetBrains Mono", monospace;
    color: #cbd5e1;
    min-width: 36px;
    text-align: right;
  }

  .tag-dimorphic {
    font-size: 10px;
    font-weight: bold;
    padding: 0 4px;
    border-radius: 3px;
  }
  .tag-dimorphic.male {
    background: rgba(59, 130, 246, 0.2);
    color: #60a5fa;
  }
  .tag-dimorphic.female {
    background: rgba(236, 72, 153, 0.2);
    color: #f472b6;
  }

  .slider-control-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .anigo-slider {
    flex: 1;
    -webkit-appearance: none;
    appearance: none;
    height: 4px;
    border-radius: 2px;
    background: #1e293b;
    outline: none;
    cursor: pointer;
  }
  .anigo-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #8b5cf6;
    cursor: pointer;
    box-shadow: 0 0 6px rgba(139, 92, 246, 0.5);
    transition: transform 0.1s ease;
  }
  .anigo-slider::-webkit-slider-thumb:hover {
    transform: scale(1.2);
    background: #a78bfa;
  }

  .btn-reset-slider {
    background: transparent;
    border: none;
    color: #64748b;
    cursor: pointer;
    font-size: 12px;
    padding: 0 2px;
  }
  .btn-reset-slider:hover {
    color: #cbd5e1;
  }

  /* Fase 2 (#43): Motor de Olhos/Íris */
  .eyes-card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    background: #151926;
    border: 1px solid #242d44;
    border-radius: 10px;
  }

  .card-subtitle {
    margin-top: 6px;
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.1em;
    color: #64748b;
    text-transform: uppercase;
  }

  .eye-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px 0;
  }

  .eye-slider-row {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 8px;
  }

  .eye-label {
    font-size: 12px;
    color: #cbd5e1;
  }

  .eye-row input[type="checkbox"],
  .eye-slider-row input[type="range"] {
    accent-color: #38bdf8;
  }

  .eye-row input[type="checkbox"] {
    width: 16px;
    height: 16px;
    cursor: pointer;
  }

  .eye-value {
    font-size: 11px;
    color: #94a3b8;
    min-width: 34px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }

  .eye-hint {
    margin-top: 2px;
    font-size: 10.5px;
    line-height: 1.45;
    color: #64748b;
  }
</style>
