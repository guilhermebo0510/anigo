<script lang="ts">
  import UserIcon from "../icons/UserIcon.svelte";
  import FolderIcon from "../icons/FolderIcon.svelte";
  import CloseIcon from "../icons/CloseIcon.svelte";

  interface Props {
    isOpen: boolean;
    onSelectPreset: (preset: "mannequin" | "sphere" | "cube") => void;
    onOpenProject: () => void;
    onImportMesh: () => void;
    onClose: () => void;
  }

  let {
    isOpen,
    onSelectPreset,
    onOpenProject,
    onImportMesh,
    onClose,
  }: Props = $props();

  let dontShowAgain = $state(false);

  function handleClose() {
    if (dontShowAgain && typeof localStorage !== "undefined") {
      localStorage.setItem("anigo_skip_quickstart", "true");
    }
    onClose();
  }

  function handleChoosePreset(preset: "mannequin" | "sphere" | "cube") {
    if (dontShowAgain && typeof localStorage !== "undefined") {
      localStorage.setItem("anigo_skip_quickstart", "true");
    }
    onSelectPreset(preset);
    onClose();
  }
</script>

{#if isOpen}
  <div class="modal-backdrop" onclick={handleClose} role="presentation">
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modal-container" onclick={(e) => e.stopPropagation()} role="dialog" aria-label="Início Rápido do ANIGO Studio" tabindex="-1">
      <!-- Titlebar -->
      <div class="modal-header">
        <div class="header-brand">
          <img src="/icon.png" alt="ANIGO" class="brand-img" />
          <div class="brand-titles">
            <span class="title-main">ANIGO STUDIO</span>
            <span class="title-sub">Início Rápido • Escolha um Modelo Base</span>
          </div>
        </div>
        <button class="btn-close" onclick={handleClose} title="Fechar e ir para viewport">
          <CloseIcon size={16} />
        </button>
      </div>

      <!-- Presets Grid -->
      <div class="modal-body">
        <div class="section-title">COMEÇAR NOVO PROJETO COM PRESET:</div>

        <div class="cards-grid">
          <button class="choice-card primary" onclick={() => handleChoosePreset("mannequin")}>
            <div class="card-icon">
              <UserIcon size={32} />
            </div>
            <span class="card-title">Manequim Anime</span>
            <span class="card-badge">Padrão da Indústria</span>
            <p class="card-desc">
              Base proporcional com réguas de cabeça (chibi 2.0x a heroico 8.5x). Pronto para modelagem de rosto, cabelo e vestuário.
            </p>
          </button>

          <button class="choice-card" onclick={() => handleChoosePreset("sphere")}>
            <div class="card-icon">
              <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
                <circle cx="12" cy="12" r="9"></circle>
                <path d="M3.6 9h16.8"></path>
                <path d="M3.6 15h16.8"></path>
                <ellipse cx="12" cy="12" rx="4.5" ry="9"></ellipse>
              </svg>
            </div>
            <span class="card-title">Esfera NPR</span>
            <span class="card-badge">Laboratório de Shading</span>
            <p class="card-desc">
              Curvatura suave ideal para calibrar bandas toon, limiares de sombra, reflexos de borda (Rim Light) e paletas de cor.
            </p>
          </button>

          <button class="choice-card" onclick={() => handleChoosePreset("cube")}>
            <div class="card-icon">
              <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6">
                <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"></path>
                <polyline points="3.27 6.96 12 12.01 20.73 6.96"></polyline>
                <line x1="12" y1="22.08" x2="12" y2="12"></line>
              </svg>
            </div>
            <span class="card-title">Cubo Unitário</span>
            <span class="card-badge">Greybox & Cenário</span>
            <p class="card-desc">
              Forma ortogonal perfeita para aferição métrica, escala modular de greybox e testes de iluminação planar.
            </p>
          </button>
        </div>

        <div class="section-divider">
          <span>OU CARREGUE UM ARQUIVO EXISTENTE</span>
        </div>

        <div class="external-actions">
          <button class="btn-external" onclick={() => { onOpenProject(); handleClose(); }}>
            <FolderIcon size={18} />
            <div class="ext-text">
              <span class="ext-title">Abrir Projeto Salvo</span>
              <span class="ext-sub">Carregar arquivo .anigo com histórico e parâmetros</span>
            </div>
          </button>

          <button class="btn-external" onclick={() => { onImportMesh(); handleClose(); }}>
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4"></path>
              <polyline points="17 8 12 3 7 8"></polyline>
              <line x1="12" y1="3" x2="12" y2="15"></line>
            </svg>
            <div class="ext-text">
              <span class="ext-title">Importar Malha Externa</span>
              <span class="ext-sub">Importar arquivo 3D (.OBJ, .VRM, .GLTF)</span>
            </div>
          </button>
        </div>
      </div>

      <!-- Footer -->
      <div class="modal-footer">
        <label class="check-container">
          <input type="checkbox" bind:checked={dontShowAgain} />
          <span>Não exibir esta janela de boas-vindas ao iniciar</span>
        </label>
        <button class="btn-proceed" onclick={handleClose}>
          Ir para o Studio
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: rgba(5, 7, 12, 0.75);
    backdrop-filter: blur(8px);
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
    animation: fadeIn 0.18s ease;
  }

  @keyframes fadeIn {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  .modal-container {
    width: 680px;
    max-width: 95vw;
    background: #0f131f;
    border: 1px solid #232d42;
    border-radius: 10px;
    box-shadow: 0 20px 50px rgba(0, 0, 0, 0.7), 0 0 2px rgba(56, 189, 248, 0.4);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 18px;
    background: #131826;
    border-bottom: 1px solid #1c2436;
  }

  .header-brand {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .brand-img {
    width: 28px;
    height: 28px;
    border-radius: 6px;
  }

  .brand-titles {
    display: flex;
    flex-direction: column;
  }

  .title-main {
    font-weight: 800;
    font-size: 0.95rem;
    letter-spacing: 1.2px;
    color: #c084fc;
  }

  .title-sub {
    font-size: 0.7rem;
    color: #94a3b8;
  }

  .btn-close {
    background: transparent;
    border: none;
    color: #64748b;
    padding: 6px;
    border-radius: 6px;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-close:hover {
    background: #1c2438;
    color: #f1f5f9;
  }

  .modal-body {
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-height: 75vh;
    overflow-y: auto;
  }

  .section-title {
    font-size: 0.68rem;
    font-weight: 700;
    color: #64748b;
    letter-spacing: 0.5px;
  }

  .cards-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 12px;
  }

  .choice-card {
    background: #141926;
    border: 1px solid #1e2638;
    border-radius: 8px;
    padding: 16px 12px;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    cursor: pointer;
    transition: all 0.18s ease;
  }

  .choice-card:hover {
    background: #1c2336;
    border-color: #38bdf8;
    transform: translateY(-2px);
    box-shadow: 0 6px 16px rgba(0, 0, 0, 0.4);
  }

  .choice-card.primary {
    border-color: #7e22ce;
    background: #1a162b;
  }

  .choice-card.primary:hover {
    border-color: #c084fc;
    background: #251c3c;
    box-shadow: 0 6px 20px rgba(192, 132, 252, 0.25);
  }

  .card-icon {
    color: #94a3b8;
    margin-bottom: 10px;
  }

  .choice-card.primary .card-icon {
    color: #c084fc;
  }

  .card-title {
    font-size: 0.85rem;
    font-weight: 700;
    color: #f1f5f9;
    margin-bottom: 4px;
  }

  .card-badge {
    font-size: 0.62rem;
    color: #38bdf8;
    background: rgba(56, 189, 248, 0.1);
    border: 1px solid rgba(56, 189, 248, 0.25);
    padding: 2px 6px;
    border-radius: 4px;
    margin-bottom: 8px;
    font-weight: 600;
  }

  .choice-card.primary .card-badge {
    color: #c084fc;
    background: rgba(192, 132, 252, 0.12);
    border-color: rgba(192, 132, 252, 0.3);
  }

  .card-desc {
    font-size: 0.68rem;
    color: #94a3b8;
    line-height: 1.35;
    margin: 0;
  }

  .section-divider {
    display: flex;
    align-items: center;
    text-align: center;
    margin: 6px 0;
    color: #475569;
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 0.5px;
  }

  .section-divider::before,
  .section-divider::after {
    content: "";
    flex: 1;
    border-bottom: 1px solid #1c2436;
  }

  .section-divider span {
    padding: 0 12px;
  }

  .external-actions {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }

  .btn-external {
    display: flex;
    align-items: center;
    gap: 12px;
    background: #141926;
    border: 1px solid #1e2638;
    border-radius: 6px;
    padding: 10px 14px;
    color: #cbd5e1;
    cursor: pointer;
    text-align: left;
    transition: all 0.15s ease;
  }

  .btn-external:hover {
    background: #1b2234;
    border-color: #38bdf8;
    color: #ffffff;
  }

  .ext-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .ext-title {
    font-size: 0.78rem;
    font-weight: 600;
    color: #f1f5f9;
  }

  .ext-sub {
    font-size: 0.66rem;
    color: #64748b;
  }

  .modal-footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 18px;
    background: #111522;
    border-top: 1px solid #1c2436;
  }

  .check-container {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.72rem;
    color: #94a3b8;
    cursor: pointer;
  }

  .btn-proceed {
    background: #7e22ce;
    border: none;
    color: #ffffff;
    padding: 7px 16px;
    border-radius: 5px;
    font-size: 0.78rem;
    font-weight: 700;
    cursor: pointer;
    transition: background 0.15s ease;
  }

  .btn-proceed:hover {
    background: #9333ea;
  }
</style>
