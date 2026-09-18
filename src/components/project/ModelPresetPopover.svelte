<script lang="ts">
  import UserIcon from "../icons/UserIcon.svelte";
  import CloseIcon from "../icons/CloseIcon.svelte";
  import CheckIcon from "../icons/CheckIcon.svelte";

  interface Props {
    isOpen: boolean;
    currentPreset: "mannequin" | "sphere" | "cube";
    onSelectPreset: (preset: "mannequin" | "sphere" | "cube") => void;
    onImportCustomMesh: () => void;
    onClose: () => void;
  }

  let {
    isOpen,
    currentPreset,
    onSelectPreset,
    onImportCustomMesh,
    onClose,
  }: Props = $props();

  const presets = [
    {
      id: "mannequin" as const,
      name: "Manequim Anime",
      category: "ANATOMIA CANÔNICA",
      polyCount: "156 faces",
      desc: "Base anatômica proporções anime (cabeça/corpo escaláveis, 2.0x a 8.5x).",
      icon: "user",
    },
    {
      id: "sphere" as const,
      name: "Esfera NPR",
      category: "TESTE DE MATERIAIS",
      polyCount: "2.592 faces",
      desc: "Calibração de cel-shading suave, limiar de sombra e specular anime.",
      icon: "sphere",
    },
    {
      id: "cube" as const,
      name: "Cubo Unitário",
      category: "BLOCAGEM GREYBOX",
      polyCount: "12 faces",
      desc: "Referência de escala volumétrica e luz direcional ortogonal.",
      icon: "cube",
    },
  ];
</script>

{#if isOpen}
  <!-- Backdrop to close on click outside -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="popover-backdrop" onclick={onClose}></div>

  <div class="preset-popover" role="dialog" aria-label="Seletor de Modelos e Presets">
    <!-- Header -->
    <div class="popover-header">
      <div class="header-info">
        <span class="header-title">PRESETS DE MALHA 3D</span>
        <span class="header-subtitle">Escolha o modelo ativo na cena</span>
      </div>
      <button class="btn-close" onclick={onClose} title="Fechar seletor">
        <CloseIcon size={14} />
      </button>
    </div>

    <div class="popover-divider"></div>

    <!-- Cards Grid -->
    <div class="preset-cards">
      {#each presets as p (p.id)}
        <button
          class="preset-card"
          class:active={currentPreset === p.id}
          onclick={() => {
            onSelectPreset(p.id);
            onClose();
          }}
        >
          <div class="card-icon-container">
            {#if p.id === "mannequin"}
              <UserIcon size={24} />
            {:else if p.id === "sphere"}
              <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
                <circle cx="12" cy="12" r="9"></circle>
                <path d="M3.6 9h16.8"></path>
                <path d="M3.6 15h16.8"></path>
                <ellipse cx="12" cy="12" rx="4.5" ry="9"></ellipse>
              </svg>
            {:else}
              <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
                <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"></path>
                <polyline points="3.27 6.96 12 12.01 20.73 6.96"></polyline>
                <line x1="12" y1="22.08" x2="12" y2="12"></line>
              </svg>
            {/if}
          </div>

          <div class="card-body">
            <div class="card-title-row">
              <span class="card-name">{p.name}</span>
              {#if currentPreset === p.id}
                <span class="badge-active">
                  <CheckIcon size={12} />
                  Ativo
                </span>
              {/if}
            </div>
            <span class="card-cat">{p.category} • {p.polyCount}</span>
            <p class="card-desc">{p.desc}</p>
          </div>
        </button>
      {/each}
    </div>

    <div class="popover-divider"></div>

    <!-- Import Mesh Action -->
    <div class="popover-footer">
      <button
        class="btn-import"
        onclick={() => {
          onImportCustomMesh();
          onClose();
        }}
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path>
          <polyline points="17 8 12 3 7 8"></polyline>
          <line x1="12" y1="3" x2="12" y2="15"></line>
        </svg>
        <span>Importar Malha 3D Externa (.OBJ, .VRM, .GLTF)...</span>
      </button>
    </div>
  </div>
{/if}

<style>
  .popover-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: transparent;
    z-index: 90;
  }

  .preset-popover {
    position: fixed;
    bottom: 38px;
    left: 50%;
    transform: translateX(-35%);
    width: 360px;
    background: #111522;
    border: 1px solid #232c40;
    border-radius: 8px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.65), 0 0 1px rgba(192, 132, 252, 0.4);
    z-index: 100;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: popoverFadeIn 0.15s cubic-bezier(0.16, 1, 0.3, 1);
  }

  @keyframes popoverFadeIn {
    from {
      opacity: 0;
      transform: translateX(-35%) translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateX(-35%) translateY(0);
    }
  }

  .popover-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    background: #141928;
  }

  .header-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .header-title {
    font-size: 0.68rem;
    font-weight: 700;
    color: #c084fc;
    letter-spacing: 0.5px;
  }

  .header-subtitle {
    font-size: 0.66rem;
    color: #64748b;
  }

  .btn-close {
    background: transparent;
    border: none;
    color: #64748b;
    cursor: pointer;
    padding: 4px;
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s ease;
  }

  .btn-close:hover {
    color: #f1f5f9;
    background: #1e2638;
  }

  .popover-divider {
    height: 1px;
    background: #1e2638;
  }

  .preset-cards {
    display: flex;
    flex-direction: column;
    padding: 8px;
    gap: 6px;
    max-height: 380px;
    overflow-y: auto;
  }

  .preset-card {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    background: #151926;
    border: 1px solid #1e2638;
    border-radius: 6px;
    padding: 10px 12px;
    cursor: pointer;
    text-align: left;
    transition: all 0.15s ease;
    width: 100%;
  }

  .preset-card:hover {
    background: #1b2234;
    border-color: #38bdf8;
  }

  .preset-card.active {
    background: #251c35;
    border-color: #c084fc;
    box-shadow: 0 0 10px rgba(192, 132, 252, 0.2);
  }

  .card-icon-container {
    color: #94a3b8;
    padding-top: 2px;
    flex-shrink: 0;
  }

  .preset-card.active .card-icon-container {
    color: #c084fc;
  }

  .card-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .card-title-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }

  .card-name {
    font-size: 0.82rem;
    font-weight: 700;
    color: #f1f5f9;
  }

  .badge-active {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 0.64rem;
    color: #c084fc;
    background: rgba(192, 132, 252, 0.15);
    padding: 1px 6px;
    border-radius: 4px;
    font-weight: 700;
  }

  .card-cat {
    font-size: 0.65rem;
    font-weight: 600;
    color: #64748b;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .card-desc {
    font-size: 0.7rem;
    color: #94a3b8;
    margin: 2px 0 0 0;
    line-height: 1.35;
  }

  .popover-footer {
    padding: 8px 10px;
    background: #0f131f;
  }

  .btn-import {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    width: 100%;
    background: #171d2b;
    border: 1px dashed #29354d;
    color: #94a3b8;
    padding: 8px 12px;
    border-radius: 5px;
    font-size: 0.74rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-import:hover {
    background: #20283b;
    border-color: #38bdf8;
    color: #f1f5f9;
  }
</style>
