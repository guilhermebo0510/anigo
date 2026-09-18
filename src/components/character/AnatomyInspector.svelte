<script lang="ts">
  import SomatotypePad2D from "./SomatotypePad2D.svelte";
  import { CANONICAL_ZONES, type MorphSlider, type MorphZone } from "../../services/morph_catalog";
  import { CANONICAL_CHARACTER_PRESETS, type CharacterPreset } from "../../services/character_presets";

  let {
    viewportRef = null,
    onModelChange = undefined,
  }: {
    viewportRef?: any;
    onModelChange?: (gender: "male" | "female") => void;
  } = $props();

  let baseGender: "male" | "female" = $state("male");
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

  let somatotypeEndo = $state(0.0);
  let somatotypeMeso = $state(0.0);
  let somatotypeEcto = $state(0.0);
  let genderDimorphism = $state(0.0);
  let activePresetId = $state<string | null>("shonen_hero");

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

  function handleSliderInput(slider: MorphSlider, value: number) {
    sliderValues[slider.id] = value;
    if (viewportRef) {
      viewportRef.setMorphSlider(slider.id, value);
    }
  }

  async function handleGenderChange(gender: "male" | "female") {
    baseGender = gender;
    genderDimorphism = gender === "female" ? 1.0 : 0.0;
    if (viewportRef) {
      if (typeof viewportRef.loadCanonicalModel === "function") {
        await viewportRef.loadCanonicalModel(gender);
      }
      if (typeof viewportRef.setGenderDimorphism === "function") {
        viewportRef.setGenderDimorphism(genderDimorphism);
      }
    }
    onModelChange?.(gender);
  }

  function handleSomatotype(endo: number, meso: number, ecto: number, gender: number) {
    somatotypeEndo = endo;
    somatotypeMeso = meso;
    somatotypeEcto = ecto;
    genderDimorphism = gender;
    if (viewportRef) {
      if (typeof viewportRef.setSomatotype === "function") {
        viewportRef.setSomatotype(endo, meso, ecto);
      }
      if (typeof viewportRef.setGenderDimorphism === "function") {
        viewportRef.setGenderDimorphism(gender);
      }
    }
  }

  function handleApplyPreset(preset: CharacterPreset) {
    activePresetId = preset.id;
    if (preset.base_gender !== baseGender) {
      handleGenderChange(preset.base_gender);
    }
    somatotypeEndo = preset.somatotype[0];
    somatotypeMeso = preset.somatotype[1];
    somatotypeEcto = preset.somatotype[2];
    handleSomatotype(somatotypeEndo, somatotypeMeso, somatotypeEcto, preset.base_gender === "female" ? 1.0 : 0.0);

    // Apply preset morph parameters
    for (const [key, val] of Object.entries(preset.morph_parameters)) {
      if (sliderValues[key] !== undefined) {
        sliderValues[key] = val;
      }
      if (viewportRef) {
        viewportRef.setMorphSlider(key, val);
      }
    }
  }

  function resetAllSliders() {
    for (const zone of CANONICAL_ZONES) {
      for (const s of zone.sliders) {
        sliderValues[s.id] = s.defaultValue;
        if (viewportRef) {
          viewportRef.setMorphSlider(s.id, s.defaultValue);
        }
      }
    }
    somatotypeEndo = 0;
    somatotypeMeso = 0;
    somatotypeEcto = 0;
    if (viewportRef) {
      viewportRef.setSomatotype?.(0, 0, 0);
    }
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
        onclick={() => handleGenderChange("female")}
      >
        <span class="gender-icon">♀</span>
        <span>Feminino Base</span>
        <span class="poly-badge">4.070 verts</span>
      </button>
    </div>
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

  <!-- Slider Search Filter -->
  <div class="search-bar">
    <span class="search-icon">🔍</span>
    <input
      type="text"
      bind:value={searchQuery}
      placeholder="Filtrar 148 sliders anatômicos..."
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
                  <span class="slider-label" title={slider.id}>{slider.name}</span>
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
</style>
