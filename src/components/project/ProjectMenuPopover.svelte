<script lang="ts">
  import FolderIcon from "../icons/FolderIcon.svelte";
  import CheckIcon from "../icons/CheckIcon.svelte";
  import CloseIcon from "../icons/CloseIcon.svelte";

  interface Props {
    isOpen: boolean;
    projectName: string;
    isDirty: boolean;
    onSave: () => void;
    onSaveAs: () => void;
    onOpen: () => void;
    onNew: () => void;
    onOpenFolder: () => void;
    onClose: () => void;
  }

  let {
    isOpen,
    projectName,
    isDirty,
    onSave,
    onSaveAs,
    onOpen,
    onNew,
    onOpenFolder,
    onClose,
  }: Props = $props();
</script>

{#if isOpen}
  <!-- Backdrop to close on outer click -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="popover-backdrop" onclick={onClose}></div>

  <div class="project-popover" role="dialog" aria-label="Gerenciamento de Projeto">
    <!-- Header -->
    <div class="popover-header">
      <div class="header-info">
        <span class="header-title">PROJETO ATUAL</span>
        <div class="project-name-row">
          <span class="file-name">{projectName}</span>
          {#if isDirty}
            <span class="badge-dirty" title="Alterações não salvas">● Modificado</span>
          {:else}
            <span class="badge-saved" title="Salvo em disco">✓ Salvo</span>
          {/if}
        </div>
      </div>
      <button class="btn-close" onclick={onClose} title="Fechar menu">
        <CloseIcon size={14} />
      </button>
    </div>

    <div class="popover-divider"></div>

    <!-- Actions List -->
    <div class="action-list">
      <button
        class="action-item primary"
        onclick={() => {
          onSave();
          onClose();
        }}
      >
        <div class="action-icon">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z"></path>
            <polyline points="17 21 17 13 7 13 7 21"></polyline>
            <polyline points="7 3 7 8 15 8"></polyline>
          </svg>
        </div>
        <div class="action-text">
          <span class="action-title">Salvar Projeto</span>
          <span class="action-hint">Gravar alterações atuais</span>
        </div>
        <span class="shortcut">Ctrl+S</span>
      </button>

      <button
        class="action-item"
        onclick={() => {
          onSaveAs();
          onClose();
        }}
      >
        <div class="action-icon">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path>
            <polyline points="14 2 14 8 20 8"></polyline>
            <line x1="12" y1="18" x2="12" y2="12"></line>
            <line x1="9" y1="15" x2="15" y2="15"></line>
          </svg>
        </div>
        <div class="action-text">
          <span class="action-title">Salvar Como...</span>
          <span class="action-hint">Escolher nome e destino</span>
        </div>
        <span class="shortcut">Ctrl+Shift+S</span>
      </button>

      <button
        class="action-item"
        onclick={() => {
          onOpen();
          onClose();
        }}
      >
        <div class="action-icon">
          <FolderIcon size={15} />
        </div>
        <div class="action-text">
          <span class="action-title">Abrir Projeto...</span>
          <span class="action-hint">Carregar arquivo .anigo</span>
        </div>
        <span class="shortcut">Ctrl+O</span>
      </button>

      <button
        class="action-item"
        onclick={() => {
          onNew();
          onClose();
        }}
      >
        <div class="action-icon">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="12" cy="12" r="10"></circle>
            <line x1="12" y1="8" x2="12" y2="16"></line>
            <line x1="8" y1="12" x2="16" y2="12"></line>
          </svg>
        </div>
        <div class="action-text">
          <span class="action-title">Novo Projeto</span>
          <span class="action-hint">Reiniciar cena com manequim limpo</span>
        </div>
        <span class="shortcut">Ctrl+N</span>
      </button>

      <div class="popover-divider"></div>

      <button
        class="action-item secondary"
        onclick={() => {
          onOpenFolder();
          onClose();
        }}
      >
        <div class="action-icon">
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"></path>
          </svg>
        </div>
        <div class="action-text">
          <span class="action-title">Abrir Pasta de Projetos</span>
          <span class="action-hint">Navegar em Documents/ANIGO/Projects</span>
        </div>
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

  .project-popover {
    position: fixed;
    bottom: 38px;
    left: 50%;
    transform: translateX(-65%);
    width: 320px;
    background: #111522;
    border: 1px solid #232c40;
    border-radius: 8px;
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.65), 0 0 1px rgba(56, 189, 248, 0.3);
    z-index: 100;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    animation: popoverFadeIn 0.15s cubic-bezier(0.16, 1, 0.3, 1);
  }

  @keyframes popoverFadeIn {
    from {
      opacity: 0;
      transform: translateX(-65%) translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateX(-65%) translateY(0);
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
    gap: 3px;
  }

  .header-title {
    font-size: 0.65rem;
    font-weight: 700;
    color: #64748b;
    letter-spacing: 0.5px;
  }

  .project-name-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .file-name {
    font-weight: 600;
    font-size: 0.82rem;
    color: #f1f5f9;
  }

  .badge-dirty {
    font-size: 0.65rem;
    color: #f59e0b;
    background: rgba(245, 158, 11, 0.12);
    padding: 1px 6px;
    border-radius: 4px;
    border: 1px solid rgba(245, 158, 11, 0.3);
    font-weight: 600;
  }

  .badge-saved {
    font-size: 0.65rem;
    color: #10b981;
    background: rgba(16, 185, 129, 0.12);
    padding: 1px 6px;
    border-radius: 4px;
    border: 1px solid rgba(16, 185, 129, 0.3);
    font-weight: 600;
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

  .action-list {
    display: flex;
    flex-direction: column;
    padding: 6px;
    gap: 2px;
  }

  .action-item {
    display: flex;
    align-items: center;
    gap: 10px;
    background: transparent;
    border: 1px solid transparent;
    color: #cbd5e1;
    padding: 8px 10px;
    border-radius: 6px;
    cursor: pointer;
    text-align: left;
    transition: all 0.12s ease;
    width: 100%;
  }

  .action-item:hover {
    background: #182032;
    border-color: #26334f;
    color: #ffffff;
  }

  .action-item.primary:hover {
    background: #1c2742;
    border-color: #38bdf8;
  }

  .action-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    color: #94a3b8;
    flex-shrink: 0;
  }

  .action-item:hover .action-icon {
    color: #38bdf8;
  }

  .action-text {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
  }

  .action-title {
    font-size: 0.78rem;
    font-weight: 600;
    color: #f1f5f9;
  }

  .action-hint {
    font-size: 0.66rem;
    color: #64748b;
  }

  .shortcut {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    font-size: 0.68rem;
    color: #64748b;
    background: #0f131f;
    padding: 2px 6px;
    border-radius: 4px;
    border: 1px solid #1e2638;
    flex-shrink: 0;
  }
</style>
