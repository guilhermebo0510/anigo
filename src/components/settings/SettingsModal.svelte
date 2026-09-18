<script lang="ts">
  import CloseIcon from "../icons/CloseIcon.svelte";
  import CheckIcon from "../icons/CheckIcon.svelte";
  import SettingsIcon from "../icons/SettingsIcon.svelte";
  import FolderIcon from "../icons/FolderIcon.svelte";
  import { t, getLanguage, setLanguage, type LanguageCode } from "../../i18n";
  import { autoSaveService } from "../../services/autosave_service";

  export interface StudioSettings {
    theme: "dark" | "light" | "system";
    language: LanguageCode;
    autoSave: boolean;
    autoSaveInterval: number; // minutos
    vsync: boolean;
    fpsCap: number; // 60, 120, 0 (0 = ilimitado)
    dpiScale: "1.0x" | "1.5x" | "2.0x";
    antiAliasing: "msaa4x" | "fxaa" | "none";
    defaultImageFormat: "png" | "jpeg" | "webp";
    defaultVideoFormat: "mp4" | "webm";
    defaultResolution: "1080p" | "1440p" | "4k";
    autoCheckUpdates: boolean;
  }

  export const DEFAULT_STUDIO_SETTINGS: StudioSettings = {
    theme: "dark",
    language: "pt-BR",
    autoSave: true,
    autoSaveInterval: 5,
    vsync: true,
    fpsCap: 120,
    dpiScale: "1.0x",
    antiAliasing: "msaa4x",
    defaultImageFormat: "png",
    defaultVideoFormat: "mp4",
    defaultResolution: "1080p",
    autoCheckUpdates: true,
  };

  interface Props {
    isOpen?: boolean;
    onClose?: () => void;
    onSave?: (settings: StudioSettings) => void;
    telemetryBackend?: string;
    telemetryAdapter?: string;
    telemetryFps?: number;
    initialSettings?: Partial<StudioSettings>;
  }

  let {
    isOpen = false,
    onClose,
    onSave,
    telemetryBackend = "WebGPU Nativo",
    telemetryAdapter = "Hardware GPU",
    telemetryFps = 120,
    initialSettings = {},
  }: Props = $props();

  type TabId = "geral" | "graficos" | "pastas" | "atalhos" | "sobre";
  let activeTab = $state<TabId>("geral");

  // Local settings state
  let theme = $state<"dark" | "light" | "system">(DEFAULT_STUDIO_SETTINGS.theme);
  let language = $state<LanguageCode>(getLanguage());
  let autoSave = $state<boolean>(DEFAULT_STUDIO_SETTINGS.autoSave);
  let autoSaveInterval = $state<number>(DEFAULT_STUDIO_SETTINGS.autoSaveInterval);

  let vsync = $state<boolean>(DEFAULT_STUDIO_SETTINGS.vsync);
  let fpsCap = $state<number>(DEFAULT_STUDIO_SETTINGS.fpsCap);
  let dpiScale = $state<"1.0x" | "1.5x" | "2.0x">(DEFAULT_STUDIO_SETTINGS.dpiScale);
  let antiAliasing = $state<"msaa4x" | "fxaa" | "none">(DEFAULT_STUDIO_SETTINGS.antiAliasing);

  let defaultImageFormat = $state<"png" | "jpeg" | "webp">(DEFAULT_STUDIO_SETTINGS.defaultImageFormat);
  let defaultVideoFormat = $state<"mp4" | "webm">(DEFAULT_STUDIO_SETTINGS.defaultVideoFormat);
  let defaultResolution = $state<"1080p" | "1440p" | "4k">(DEFAULT_STUDIO_SETTINGS.defaultResolution);
  let autoCheckUpdates = $state<boolean>(DEFAULT_STUDIO_SETTINGS.autoCheckUpdates);

  // Directory paths
  let projectsDir = $state<string>("C:/Users/.../Documents/ANIGO/Projects");
  let assetsDir = $state<string>("C:/Users/.../Documents/ANIGO/Assets");
  let autosaveDir = $state<string>("C:/Users/.../Documents/ANIGO/Autosave");
  let rendersDir = $state<string>("C:/Users/.../Documents/ANIGO/Renders");

  // Status & Feedback states
  let isSaved = $state(false);
  let manualSaveMsg = $state<string | null>(null);
  let copyFeedback = $state(false);
  let updateStatus = $state<"idle" | "checking" | "up_to_date">("idle");
  let lastAutosaveInfo = $state<string>("Nenhum salvamento nesta sessão.");

  // Dragging state
  let dragDeltaX = $state(0);
  let dragDeltaY = $state(0);
  let isDragging = $state(false);
  let dragStartX = 0;
  let dragStartY = 0;
  let initialOffsetX = 0;
  let initialOffsetY = 0;

  // Initialize directory paths on mount or open
  $effect(() => {
    if (isOpen) {
      if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
        import("@tauri-apps/api/core").then(({ invoke }) => {
          invoke<any>("get_studio_directories").then((dirs) => {
            if (dirs) {
              projectsDir = dirs.projects_dir;
              assetsDir = dirs.assets_dir;
              autosaveDir = dirs.autosave_dir;
              rendersDir = dirs.renders_dir;
            }
          }).catch(() => {});
        });
      }
    }
  });

  $effect(() => {
    if (initialSettings.theme !== undefined) theme = initialSettings.theme;
    if (initialSettings.language !== undefined) {
      language = initialSettings.language;
      setLanguage(language);
    }
    if (initialSettings.autoSave !== undefined) autoSave = initialSettings.autoSave;
    if (initialSettings.autoSaveInterval !== undefined) autoSaveInterval = initialSettings.autoSaveInterval;
    if (initialSettings.vsync !== undefined) vsync = initialSettings.vsync;
    if (initialSettings.fpsCap !== undefined) fpsCap = initialSettings.fpsCap;
    if (initialSettings.dpiScale !== undefined) dpiScale = initialSettings.dpiScale;
    if (initialSettings.antiAliasing !== undefined) antiAliasing = initialSettings.antiAliasing;
    if (initialSettings.defaultImageFormat !== undefined) defaultImageFormat = initialSettings.defaultImageFormat;
    if (initialSettings.defaultVideoFormat !== undefined) defaultVideoFormat = initialSettings.defaultVideoFormat;
    if (initialSettings.defaultResolution !== undefined) defaultResolution = initialSettings.defaultResolution;
    if (initialSettings.autoCheckUpdates !== undefined) autoCheckUpdates = initialSettings.autoCheckUpdates;
  });

  function handlePointerDown(e: PointerEvent) {
    if ((e.target as HTMLElement)?.closest("button") || (e.target as HTMLElement)?.closest("input") || (e.target as HTMLElement)?.closest("select")) {
      return;
    }
    isDragging = true;
    dragStartX = e.clientX;
    dragStartY = e.clientY;
    initialOffsetX = dragDeltaX;
    initialOffsetY = dragDeltaY;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function handlePointerMove(e: PointerEvent) {
    if (!isDragging) return;
    dragDeltaX = initialOffsetX + (e.clientX - dragStartX);
    dragDeltaY = initialOffsetY + (e.clientY - dragStartY);
  }

  function handlePointerUp(e: PointerEvent) {
    if (isDragging) {
      isDragging = false;
      try {
        (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
      } catch {}
    }
  }

  function handleLanguageChange(newLang: LanguageCode) {
    language = newLang;
    setLanguage(newLang);
  }

  async function handleManualSaveNow() {
    manualSaveMsg = "Gravando...";
    const path = await autoSaveService.saveNow();
    if (path) {
      const time = new Date().toLocaleTimeString();
      lastAutosaveInfo = `Salvo às ${time}`;
      manualSaveMsg = "Projeto gravado em disco com sucesso!";
    } else {
      manualSaveMsg = "Falha ao gravar arquivo.";
    }
    setTimeout(() => {
      manualSaveMsg = null;
    }, 2500);
  }

  async function openFolder(path: string) {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      invoke("open_directory_in_explorer", { path }).catch((e) => {
        console.error("Falha ao abrir Explorer:", e);
      });
    }
  }

  async function handleCheckUpdates() {
    updateStatus = "checking";
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        await invoke("check_app_updates");
      } catch (_) {}
    }
    setTimeout(() => {
      updateStatus = "up_to_date";
    }, 600);
  }

  function handleResetDefaults() {
    theme = DEFAULT_STUDIO_SETTINGS.theme;
    language = DEFAULT_STUDIO_SETTINGS.language;
    setLanguage(language);
    autoSave = DEFAULT_STUDIO_SETTINGS.autoSave;
    autoSaveInterval = DEFAULT_STUDIO_SETTINGS.autoSaveInterval;
    vsync = DEFAULT_STUDIO_SETTINGS.vsync;
    fpsCap = DEFAULT_STUDIO_SETTINGS.fpsCap;
    dpiScale = DEFAULT_STUDIO_SETTINGS.dpiScale;
    antiAliasing = DEFAULT_STUDIO_SETTINGS.antiAliasing;
    defaultImageFormat = DEFAULT_STUDIO_SETTINGS.defaultImageFormat;
    defaultVideoFormat = DEFAULT_STUDIO_SETTINGS.defaultVideoFormat;
    defaultResolution = DEFAULT_STUDIO_SETTINGS.defaultResolution;
    autoCheckUpdates = DEFAULT_STUDIO_SETTINGS.autoCheckUpdates;
  }

  function handleSave() {
    const settings: StudioSettings = {
      theme,
      language,
      autoSave,
      autoSaveInterval,
      vsync,
      fpsCap,
      dpiScale,
      antiAliasing,
      defaultImageFormat,
      defaultVideoFormat,
      defaultResolution,
      autoCheckUpdates,
    };
    onSave?.(settings);
    isSaved = true;
    setTimeout(() => {
      isSaved = false;
      onClose?.();
    }, 450);
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (!isOpen) return;
    if (e.key === "Escape") {
      onClose?.();
    }
  }

  async function copyDiagnostics() {
    const diagnostics = {
      app: "ANIGO Studio",
      version: "0.1.0",
      backend: telemetryBackend,
      adapter: telemetryAdapter,
      fps: telemetryFps,
      fpsCap,
      dpiScale,
      vsync,
      antiAliasing,
      language,
      theme,
      timestamp: new Date().toISOString(),
    };
    try {
      await navigator.clipboard.writeText(JSON.stringify(diagnostics, null, 2));
      copyFeedback = true;
      setTimeout(() => {
        copyFeedback = false;
      }, 2000);
    } catch (_) {}
  }
</script>

<svelte:window onkeydown={handleKeyDown} />

{#if isOpen}
  <div
    class="studio-settings-modal"
    role="dialog"
    aria-modal="true"
    aria-labelledby="settings-modal-title"
    style="transform: translate(calc(-50% + {dragDeltaX}px), calc(-50% + {dragDeltaY}px));"
  >
    <!-- Titlebar (Draggable Handle) -->
    <div
      class="modal-titlebar"
      role="toolbar"
      aria-label="Barra de título da janela"
      tabindex="-1"
      onpointerdown={handlePointerDown}
      onpointermove={handlePointerMove}
      onpointerup={handlePointerUp}
    >
      <div class="titlebar-left">
        <SettingsIcon size={16} />
        <h2 id="settings-modal-title">{t("settings.modal_title", "Configurações do Studio")}</h2>
      </div>
      <button
        type="button"
        class="close-btn"
        onclick={() => onClose?.()}
        title={t("btn.close", "Fechar")}
        aria-label="Fechar"
      >
        <CloseIcon size={16} />
      </button>
    </div>

    <!-- Tabs Navigation -->
    <nav class="tabs-nav" aria-label="Abas de Configuração">
      <button
        type="button"
        class="tab-item"
        class:active={activeTab === "geral"}
        onclick={() => (activeTab = "geral")}
      >
        {t("settings.tab_general", "Geral")}
      </button>
      <button
        type="button"
        class="tab-item"
        class:active={activeTab === "graficos"}
        onclick={() => (activeTab = "graficos")}
      >
        {t("settings.tab_graphics", "Gráficos")}
      </button>
      <button
        type="button"
        class="tab-item"
        class:active={activeTab === "pastas"}
        onclick={() => (activeTab = "pastas")}
      >
        {t("settings.tab_storage", "Pastas & Armazenamento")}
      </button>
      <button
        type="button"
        class="tab-item"
        class:active={activeTab === "atalhos"}
        onclick={() => (activeTab = "atalhos")}
      >
        {t("settings.tab_shortcuts", "Atalhos")}
      </button>
      <button
        type="button"
        class="tab-item"
        class:active={activeTab === "sobre"}
        onclick={() => (activeTab = "sobre")}
      >
        {t("settings.tab_about", "Ajuda & Atualizações")}
      </button>
    </nav>

    <!-- Modal Body Content -->
    <div class="modal-body">
      <!-- 1. ABA GERAL -->
      {#if activeTab === "geral"}
        <div class="section-group">
          <div class="section-header">{t("settings.theme_title", "TEMA DA INTERFACE")}</div>
          <div class="options-grid">
            <button
              type="button"
              class="opt-btn"
              class:selected={theme === "dark"}
              onclick={() => (theme = "dark")}
            >
              {t("settings.theme_dark", "Escuro (Dark Obsidian)")}
            </button>
            <button
              type="button"
              class="opt-btn"
              class:selected={theme === "light"}
              onclick={() => (theme = "light")}
            >
              {t("settings.theme_light", "Claro (Studio Light)")}
            </button>
            <button
              type="button"
              class="opt-btn"
              class:selected={theme === "system"}
              onclick={() => (theme = "system")}
            >
              {t("settings.theme_system", "Sistema (Automático)")}
            </button>
          </div>
        </div>

        <div class="section-group">
          <div class="section-header">{t("settings.lang_title", "IDIOMA DO SOFTWARE")}</div>
          <div class="options-grid">
            <button
              type="button"
              class="opt-btn"
              class:selected={language === "pt-BR"}
              onclick={() => handleLanguageChange("pt-BR")}
            >
              Português (Brasil)
            </button>
            <button
              type="button"
              class="opt-btn"
              class:selected={language === "en"}
              onclick={() => handleLanguageChange("en")}
            >
              English (US)
            </button>
            <button
              type="button"
              class="opt-btn"
              class:selected={language === "ja"}
              onclick={() => handleLanguageChange("ja")}
            >
              日本語 (Japanese)
            </button>
          </div>
        </div>

        <div class="section-group">
          <div class="section-header">{t("settings.autosave_title", "SALVAMENTO AUTOMÁTICO (AUTOSAVE REAL)")}</div>
          <label class="toggle-row">
            <input type="checkbox" bind:checked={autoSave} />
            <span class="toggle-label">{t("settings.autosave_enable", "Ativar salvamento periódico no disco")}</span>
          </label>

          {#if autoSave}
            <div class="slider-control">
              <div class="slider-info">
                <span>{t("settings.autosave_interval", "Intervalo de Salvamento")}</span>
                <span class="val-badge">{autoSaveInterval} {t("settings.autosave_minutes", "minutos")}</span>
              </div>
              <input
                type="range"
                min="1"
                max="30"
                step="1"
                bind:value={autoSaveInterval}
                class="settings-slider"
              />
            </div>
          {/if}

          <div class="action-row">
            <button type="button" class="btn-action-primary" onclick={handleManualSaveNow}>
              {t("settings.autosave_now", "Salvar Projeto Agora no Disco")}
            </button>
            {#if manualSaveMsg}
              <span class="save-msg-pill">{manualSaveMsg}</span>
            {/if}
          </div>
        </div>

      <!-- 2. ABA GRÁFICOS -->
      {:else if activeTab === "graficos"}
        <div class="section-group">
          <div class="section-header">{t("settings.gpu_backend", "BACKEND DE RENDERIZAÇÃO")}</div>
          <div class="hardware-chip">
            <span class="hw-icon">⚡</span>
            <div class="hw-details">
              <span class="hw-name">{telemetryAdapter}</span>
              <span class="hw-sub">{t("settings.gpu_active", "WebGPU Nativo (GPU Dedicada Ativa)")}</span>
            </div>
          </div>
        </div>

        <div class="section-group">
          <div class="section-header">{t("settings.fps_cap_title", "LIMITE DA TAXA DE QUADROS (FPS CAP)")}</div>
          <div class="options-grid">
            <button
              type="button"
              class="opt-btn"
              class:selected={fpsCap === 60}
              onclick={() => (fpsCap = 60)}
            >
              {t("settings.fps_60", "60 FPS (Econômico)")}
            </button>
            <button
              type="button"
              class="opt-btn"
              class:selected={fpsCap === 120}
              onclick={() => (fpsCap = 120)}
            >
              {t("settings.fps_120", "120 FPS (Fluido)")}
            </button>
            <button
              type="button"
              class="opt-btn"
              class:selected={fpsCap === 0}
              onclick={() => (fpsCap = 0)}
            >
              {t("settings.fps_unlimited", "Ilimitado")}
            </button>
          </div>
        </div>

        <div class="section-group">
          <div class="section-header">{t("settings.dpi_scale_title", "ESCALA DE DPI & RESOLUÇÃO")}</div>
          <div class="options-grid">
            <button
              type="button"
              class="opt-btn"
              class:selected={dpiScale === "1.0x"}
              onclick={() => (dpiScale = "1.0x")}
            >
              {t("settings.dpi_10", "1.0x (Nativo Padrão)")}
            </button>
            <button
              type="button"
              class="opt-btn"
              class:selected={dpiScale === "1.5x"}
              onclick={() => (dpiScale = "1.5x")}
            >
              {t("settings.dpi_15", "1.5x (Super-Amostrado)")}
            </button>
            <button
              type="button"
              class="opt-btn"
              class:selected={dpiScale === "2.0x"}
              onclick={() => (dpiScale = "2.0x")}
            >
              {t("settings.dpi_20", "2.0x (Ultra Nítido 4K)")}
            </button>
          </div>
        </div>

        <div class="section-group">
          <div class="section-header">{t("settings.vsync_title", "SINCRONIZAÇÃO VERTICAL (VSYNC)")}</div>
          <label class="toggle-row">
            <input type="checkbox" bind:checked={vsync} />
            <span class="toggle-label">{t("settings.vsync_desc", "Evita screen tearing e estabiliza quadros")}</span>
          </label>
        </div>

        <div class="section-group">
          <div class="section-header">{t("settings.aa_title", "ANTI-ALIASING")}</div>
          <div class="options-grid">
            <button
              type="button"
              class="opt-btn"
              class:selected={antiAliasing === "msaa4x"}
              onclick={() => (antiAliasing = "msaa4x")}
            >
              MSAA 4x (Alta Qualidade)
            </button>
            <button
              type="button"
              class="opt-btn"
              class:selected={antiAliasing === "fxaa"}
              onclick={() => (antiAliasing = "fxaa")}
            >
              FXAA (Rápido)
            </button>
            <button
              type="button"
              class="opt-btn"
              class:selected={antiAliasing === "none"}
              onclick={() => (antiAliasing = "none")}
            >
              Nenhum
            </button>
          </div>
        </div>

      <!-- 3. ABA PASTAS & ARMAZENAMENTO -->
      {:else if activeTab === "pastas"}
        <div class="section-group">
          <div class="section-header">{t("settings.paths_title", "DIRETÓRIOS PADRÃO DO ESTÚDIO")}</div>
          
          <div class="path-row">
            <div class="path-meta">
              <span class="path-title">{t("settings.path_projects", "Pasta de Projetos (.anigo)")}</span>
              <span class="path-val">{projectsDir}</span>
            </div>
            <button type="button" class="btn-open-dir" onclick={() => openFolder(projectsDir)} title="Abrir pasta no Windows Explorer">
              <FolderIcon size={14} />
              <span>Abrir</span>
            </button>
          </div>

          <div class="path-row">
            <div class="path-meta">
              <span class="path-title">{t("settings.path_assets", "Pasta de Biblioteca & Assets")}</span>
              <span class="path-val">{assetsDir}</span>
            </div>
            <button type="button" class="btn-open-dir" onclick={() => openFolder(assetsDir)} title="Abrir pasta no Windows Explorer">
              <FolderIcon size={14} />
              <span>Abrir</span>
            </button>
          </div>

          <div class="path-row">
            <div class="path-meta">
              <span class="path-title">{t("settings.path_autosave", "Pasta de Salvamento Automático")}</span>
              <span class="path-val">{autosaveDir}</span>
            </div>
            <button type="button" class="btn-open-dir" onclick={() => openFolder(autosaveDir)} title="Abrir pasta no Windows Explorer">
              <FolderIcon size={14} />
              <span>Abrir</span>
            </button>
          </div>

          <div class="path-row">
            <div class="path-meta">
              <span class="path-title">{t("settings.path_renders", "Pasta de Renders & Exportações")}</span>
              <span class="path-val">{rendersDir}</span>
            </div>
            <button type="button" class="btn-open-dir" onclick={() => openFolder(rendersDir)} title="Abrir pasta no Windows Explorer">
              <FolderIcon size={14} />
              <span>Abrir</span>
            </button>
          </div>
        </div>

        <div class="section-group">
          <div class="section-header">{t("settings.render_presets_title", "PADRÕES DE EXPORTAÇÃO")}</div>
          <div class="preset-controls-grid">
            <div class="preset-item">
              <span class="preset-label">{t("settings.image_format", "Formato de Fotografia")}</span>
              <select bind:value={defaultImageFormat} class="dark-dropdown">
                <option value="png">PNG (Sem Perda / Alpha)</option>
                <option value="jpeg">JPEG (Alta Qualidade)</option>
                <option value="webp">WebP (Otimizado Web)</option>
              </select>
            </div>
            <div class="preset-item">
              <span class="preset-label">{t("settings.video_format", "Formato de Vídeo")}</span>
              <select bind:value={defaultVideoFormat} class="dark-dropdown">
                <option value="mp4">MP4 (H.264 Universal)</option>
                <option value="webm">WebM (VP9 Aberto)</option>
              </select>
            </div>
            <div class="preset-item">
              <span class="preset-label">{t("settings.default_resolution", "Resolução Padrão")}</span>
              <select bind:value={defaultResolution} class="dark-dropdown">
                <option value="1080p">1920×1080 (FHD 1080p)</option>
                <option value="1440p">2560×1440 (2K QHD)</option>
                <option value="4k">3840×2160 (4K UHD)</option>
              </select>
            </div>
          </div>
        </div>

      <!-- 4. ABA ATALHOS -->
      {:else if activeTab === "atalhos"}
        <div class="section-group">
          <div class="section-header">{t("settings.nav_shortcuts_title", "ATALHOS DE NAVEGAÇÃO 3D")}</div>
          <div class="shortcut-list">
            <div class="shortcut-item">
              <span class="sc-desc">{t("settings.nav_orbit", "Giro orbital da câmera ao redor do modelo")}</span>
              <div class="sc-keys"><kbd>Botão Esquerdo (LMB)</kbd></div>
            </div>
            <div class="shortcut-item">
              <span class="sc-desc">{t("settings.nav_pan", "Translação lateral da visão (Pan)")}</span>
              <div class="sc-keys"><kbd>Shift + RMB</kbd> <kbd>RMB</kbd> <kbd>MMB</kbd></div>
            </div>
            <div class="shortcut-item">
              <span class="sc-desc">{t("settings.nav_zoom", "Zoom progressivo direcionado ao cursor")}</span>
              <div class="sc-keys"><kbd>Roda do Mouse (Scroll)</kbd></div>
            </div>
            <div class="shortcut-item">
              <span class="sc-desc">{t("settings.nav_focus", "Recentraliza e foca a câmera no modelo")}</span>
              <div class="sc-keys"><kbd>F</kbd></div>
            </div>
          </div>
        </div>

        <div class="section-group">
          <div class="section-header">{t("settings.app_shortcuts_title", "ATALHOS DO STUDIO & INTERFACE")}</div>
          <div class="shortcut-list">
            <div class="shortcut-item">
              <span class="sc-desc">{t("settings.sc_settings", "Abre ou fecha este painel de configurações")}</span>
              <div class="sc-keys"><kbd>Ctrl</kbd> + <kbd>,</kbd></div>
            </div>
            <div class="shortcut-item">
              <span class="sc-desc">{t("settings.sc_inspector", "Recolhe ou expande o painel de propriedades")}</span>
              <div class="sc-keys"><kbd>Ctrl</kbd> + <kbd>B</kbd></div>
            </div>
            <div class="shortcut-item">
              <span class="sc-desc">{t("settings.sc_workspaces", "Alterna diretamente entre os 8 workspaces")}</span>
              <div class="sc-keys"><kbd>1</kbd> .. <kbd>8</kbd></div>
            </div>
            <div class="shortcut-item">
              <span class="sc-desc">{t("settings.sc_play", "Reproduz ou pausa animações")}</span>
              <div class="sc-keys"><kbd>Espaço</kbd></div>
            </div>
          </div>
        </div>

      <!-- 5. ABA AJUDA & ATUALIZAÇÕES -->
      {:else if activeTab === "sobre"}
        <div class="section-group">
          <div class="section-header">{t("settings.about_title", "SOBRE O ANIGO STUDIO")}</div>
          <div class="about-card">
            <div class="about-line">
              <span class="about-key">{t("settings.version_label", "Versão Atual")}:</span>
              <span class="about-val bold">v0.1.0 Alpha (Gold Standard)</span>
            </div>
            <div class="about-line">
              <span class="about-key">{t("settings.engine_label", "Motor Gráfico")}:</span>
              <span class="about-val">{telemetryBackend} ({telemetryAdapter})</span>
            </div>
            <div class="about-line">
              <span class="about-key">Pipeline:</span>
              <span class="about-val">Anime NPR (Half-Lambert + Inverted Hull WGSL)</span>
            </div>
            <button type="button" class="btn-action-secondary" onclick={copyDiagnostics}>
              {copyFeedback ? "✓ Diagnóstico Copiado!" : "Copiar Diagnóstico Completo"}
            </button>
          </div>
        </div>

        <div class="section-group">
          <div class="section-header">{t("settings.updates_title", "SISTEMA DE ATUALIZAÇÃO AUTOMÁTICA")}</div>
          <div class="updater-box">
            <p class="updater-text">{t("settings.updates_desc", "Verifica novas versões e correções de bugs automaticamente.")}</p>
            <div class="updater-action">
              <button
                type="button"
                class="btn-action-primary"
                disabled={updateStatus === "checking"}
                onclick={handleCheckUpdates}
              >
                {#if updateStatus === "checking"}
                  {t("settings.checking_updates", "Verificando servidores...")}
                {:else}
                  {t("settings.btn_check_updates", "Verificar Atualizações Agora")}
                {/if}
              </button>
              {#if updateStatus === "up_to_date"}
                <span class="status-badge-ok">✓ {t("settings.latest_version_ok", "Versão mais recente ativa!")}</span>
              {/if}
            </div>
            <label class="toggle-row" style="margin-top: 10px;">
              <input type="checkbox" bind:checked={autoCheckUpdates} />
              <span class="toggle-label">{t("settings.update_auto_check", "Verificar atualizações automaticamente ao iniciar")}</span>
            </label>
          </div>
        </div>

        <div class="section-group">
          <div class="section-header">{t("settings.doc_links_title", "DOCUMENTAÇÃO & AJUDA")}</div>
          <div class="links-grid">
            <a href="https://github.com/leorcf/ANIGO" target="_blank" rel="noreferrer" class="doc-link">
              <span>📖 {t("settings.link_manual", "Manual do Usuário ANIGO")}</span>
            </a>
            <a href="https://github.com/leorcf/ANIGO" target="_blank" rel="noreferrer" class="doc-link">
              <span>⌨️ {t("settings.link_shortcuts", "Guia Visual de Atalhos")}</span>
            </a>
            <a href="https://github.com/leorcf/ANIGO" target="_blank" rel="noreferrer" class="doc-link">
              <span>💬 {t("settings.link_community", "Comunidade Discord e Suporte")}</span>
            </a>
          </div>
        </div>
      {/if}
    </div>

    <!-- Modal Footer Actions -->
    <div class="modal-footer">
      <button
        type="button"
        class="btn-secondary"
        onclick={handleResetDefaults}
      >
        {t("btn.defaults", "Restaurar Padrões")}
      </button>

      <div class="footer-right">
        <button
          type="button"
          class="btn-secondary"
          onclick={() => onClose?.()}
        >
          {t("btn.close", "Fechar")}
        </button>
        <button
          type="button"
          class="btn-primary"
          class:saved={isSaved}
          onclick={handleSave}
        >
          {#if isSaved}
            <CheckIcon size={14} />
            <span>{t("btn.saved", "Salvo!")}</span>
          {:else}
            <span>{t("btn.save", "Salvar Preferências")}</span>
          {/if}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .studio-settings-modal {
    position: fixed;
    top: 50%;
    left: 50%;
    z-index: 9999;
    width: 580px;
    height: 540px;
    background: #0d111a;
    border: 1px solid #1f293d;
    border-radius: 8px;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.75), 0 0 0 1px rgba(255, 255, 255, 0.05);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    user-select: none;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    color: #e2e8f0;
  }

  .modal-titlebar {
    height: 40px;
    background: #131824;
    border-bottom: 1px solid #1a2336;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 14px;
    cursor: grab;
    flex-shrink: 0;
  }

  .modal-titlebar:active {
    cursor: grabbing;
  }

  .titlebar-left {
    display: flex;
    align-items: center;
    gap: 8px;
    color: #c084fc;
  }

  .titlebar-left h2 {
    font-size: 0.84rem;
    font-weight: 700;
    margin: 0;
    color: #f1f5f9;
    letter-spacing: 0.3px;
  }

  .close-btn {
    background: transparent;
    border: none;
    color: #64748b;
    padding: 4px;
    border-radius: 4px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.12s ease;
  }

  .close-btn:hover {
    background: #ef4444;
    color: #ffffff;
  }

  .tabs-nav {
    display: flex;
    background: #090c13;
    border-bottom: 1px solid #1a2336;
    padding: 0 8px;
    gap: 2px;
    flex-shrink: 0;
    overflow-x: auto;
  }

  .tab-item {
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: #8292a8;
    padding: 8px 12px;
    font-size: 0.75rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
    white-space: nowrap;
  }

  .tab-item:hover {
    color: #f1f5f9;
  }

  .tab-item.active {
    color: #c084fc;
    border-bottom-color: #c084fc;
  }

  .modal-body {
    flex: 1 1 0%;
    min-height: 0;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    scrollbar-width: thin;
    scrollbar-color: #1f293d transparent;
  }

  .modal-body::-webkit-scrollbar {
    width: 6px;
  }
  .modal-body::-webkit-scrollbar-thumb {
    background: #1f293d;
    border-radius: 3px;
  }

  .section-group {
    background: #121723;
    border: 1px solid #1b2438;
    border-radius: 6px;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .section-header {
    font-size: 0.68rem;
    font-weight: 700;
    color: #94a3b8;
    letter-spacing: 0.5px;
    text-transform: uppercase;
  }

  .options-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(130px, 1fr));
    gap: 6px;
  }

  .opt-btn {
    background: #171e2e;
    border: 1px solid #232d42;
    border-radius: 5px;
    color: #cbd5e1;
    padding: 8px 10px;
    font-size: 0.74rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.12s ease;
    text-align: center;
  }

  .opt-btn:hover {
    background: #1f293e;
    border-color: #38bdf8;
    color: #f1f5f9;
  }

  .opt-btn.selected {
    background: #281a3d;
    border-color: #c084fc;
    color: #c084fc;
    font-weight: 600;
  }

  .toggle-row {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
    font-size: 0.76rem;
    color: #cbd5e1;
  }

  .toggle-row input[type="checkbox"] {
    accent-color: #c084fc;
    cursor: pointer;
    width: 15px;
    height: 15px;
  }

  .slider-control {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 4px;
  }

  .slider-info {
    display: flex;
    justify-content: space-between;
    font-size: 0.72rem;
    color: #94a3b8;
  }

  .val-badge {
    color: #38bdf8;
    font-weight: 600;
  }

  .settings-slider {
    width: 100%;
    accent-color: #c084fc;
    cursor: pointer;
  }

  .action-row {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 4px;
  }

  .btn-action-primary {
    background: #25173b;
    border: 1px solid #c084fc;
    color: #c084fc;
    border-radius: 4px;
    padding: 6px 12px;
    font-size: 0.72rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .btn-action-primary:hover:not(:disabled) {
    background: #c084fc;
    color: #0d111a;
  }

  .btn-action-primary:disabled {
    opacity: 0.5;
    cursor: wait;
  }

  .save-msg-pill {
    font-size: 0.7rem;
    color: #10b981;
    font-weight: 500;
  }

  .hardware-chip {
    display: flex;
    align-items: center;
    gap: 10px;
    background: #151d2c;
    border: 1px solid #222e44;
    border-radius: 6px;
    padding: 8px 12px;
  }

  .hw-icon {
    font-size: 1.2rem;
  }

  .hw-details {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .hw-name {
    font-size: 0.78rem;
    font-weight: 700;
    color: #f1f5f9;
  }

  .hw-sub {
    font-size: 0.68rem;
    color: #38bdf8;
  }

  .path-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 6px 0;
    border-bottom: 1px solid #1a2233;
    gap: 10px;
  }

  .path-row:last-child {
    border-bottom: none;
  }

  .path-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    overflow: hidden;
  }

  .path-title {
    font-size: 0.72rem;
    font-weight: 600;
    color: #cbd5e1;
  }

  .path-val {
    font-size: 0.66rem;
    color: #64748b;
    font-family: monospace;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .btn-open-dir {
    background: #171f2f;
    border: 1px solid #233047;
    color: #94a3b8;
    border-radius: 4px;
    padding: 4px 8px;
    font-size: 0.68rem;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 4px;
    cursor: pointer;
    transition: all 0.12s ease;
    flex-shrink: 0;
  }

  .btn-open-dir:hover {
    background: #222e46;
    color: #38bdf8;
    border-color: #38bdf8;
  }

  .preset-controls-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(140px, 1fr));
    gap: 8px;
  }

  .preset-item {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .preset-label {
    font-size: 0.68rem;
    color: #94a3b8;
  }

  .dark-dropdown {
    background: #151c2a;
    border: 1px solid #232e44;
    color: #cbd5e1;
    border-radius: 4px;
    padding: 6px 8px;
    font-size: 0.72rem;
    cursor: pointer;
  }

  .shortcut-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .shortcut-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.74rem;
    padding: 4px 0;
    border-bottom: 1px solid #182030;
    gap: 10px;
  }

  .shortcut-item:last-child {
    border-bottom: none;
  }

  .sc-desc {
    color: #cbd5e1;
  }

  .sc-keys {
    display: flex;
    gap: 4px;
    align-items: center;
  }

  kbd {
    background: #182133;
    border: 1px solid #2a3854;
    border-radius: 4px;
    box-shadow: 0 1px 2px rgba(0,0,0,0.4);
    color: #38bdf8;
    font-family: monospace;
    font-size: 0.68rem;
    padding: 2px 6px;
    white-space: nowrap;
  }

  .about-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .about-line {
    display: flex;
    justify-content: space-between;
    font-size: 0.74rem;
  }

  .about-key {
    color: #94a3b8;
  }

  .about-val {
    color: #cbd5e1;
  }

  .about-val.bold {
    color: #c084fc;
    font-weight: 700;
  }

  .btn-action-secondary {
    margin-top: 6px;
    background: #161e2d;
    border: 1px solid #243046;
    color: #94a3b8;
    border-radius: 4px;
    padding: 6px 12px;
    font-size: 0.72rem;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .btn-action-secondary:hover {
    background: #202b40;
    color: #f1f5f9;
  }

  .updater-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .updater-text {
    margin: 0;
    font-size: 0.72rem;
    color: #94a3b8;
  }

  .updater-action {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 4px;
  }

  .status-badge-ok {
    font-size: 0.72rem;
    color: #10b981;
    font-weight: 600;
  }

  .links-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: 6px;
  }

  .doc-link {
    display: flex;
    align-items: center;
    padding: 6px 10px;
    background: #151d2c;
    border: 1px solid #1f2a3f;
    border-radius: 4px;
    color: #cbd5e1;
    text-decoration: none;
    font-size: 0.72rem;
    transition: all 0.12s ease;
  }

  .doc-link:hover {
    background: #1e293e;
    color: #38bdf8;
    border-color: #38bdf8;
  }

  .modal-footer {
    height: 48px;
    background: #131824;
    border-top: 1px solid #1a2336;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 14px;
    flex-shrink: 0;
  }

  .footer-right {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .btn-secondary {
    background: #182030;
    border: 1px solid #26334d;
    color: #94a3b8;
    border-radius: 4px;
    padding: 6px 12px;
    font-size: 0.74rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .btn-secondary:hover {
    background: #222d42;
    color: #f1f5f9;
  }

  .btn-primary {
    background: #2a1b40;
    border: 1px solid #c084fc;
    color: #c084fc;
    border-radius: 4px;
    padding: 6px 14px;
    font-size: 0.74rem;
    font-weight: 600;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 6px;
    transition: all 0.15s ease;
  }

  .btn-primary:hover {
    background: #c084fc;
    color: #0d111a;
  }

  .btn-primary.saved {
    background: #064e3b;
    border-color: #10b981;
    color: #34d399;
  }
</style>
