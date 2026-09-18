<script module lang="ts">
  export const CANONICAL_ASSET_CATEGORIES = [
    "Todos",
    "Personagens",
    "Roupas",
    "Penteados",
    "Acessórios",
    "Poses",
    "Materiais",
    "Ambientes",
    "Cenários",
  ] as const;

  export interface AssetRecord {
    id: string;
    name: string;
    category: string;
    tags: string[];
    thumbnail?: string;
    polyCount: number;
    vertexCount: number;
    format: "VRM" | "OBJ" | "GLB" | "ANIGO_MATERIAL";
    description?: string;
    author?: string;
  }

  export const assetCatalog: AssetRecord[] = [
    {
      id: "char_01",
      name: "Aoi Mannequin Base",
      category: "Personagens",
      tags: ["anime", "female", "base", "anatomia"],
      polyCount: 14200,
      vertexCount: 7800,
      format: "VRM",
      description: "Modelo base feminino canon anime com topologia limpa para rigging e deformação facial.",
      author: "ANIGO Studio Core",
    },
    {
      id: "char_02",
      name: "Ren Stylized Male",
      category: "Personagens",
      tags: ["anime", "male", "hero", "shonen"],
      polyCount: 16500,
      vertexCount: 8900,
      format: "VRM",
      description: "Base masculina estilizada para personagens shonen com proporção de 7.0 cabeças.",
      author: "ANIGO Studio Core",
    },
    {
      id: "outfit_01",
      name: "Seifuku School Uniform",
      category: "Roupas",
      tags: ["uniform", "school", "cloth", "saia"],
      polyCount: 6800,
      vertexCount: 3600,
      format: "GLB",
      description: "Uniforme escolar tradicional com fitas e pregas dinâmicas de tecido anime.",
      author: "ANIGO Asset Team",
    },
    {
      id: "outfit_02",
      name: "Battle Kimono Robe",
      category: "Roupas",
      tags: ["kimono", "fantasy", "cloth", "samurai"],
      polyCount: 9200,
      vertexCount: 4900,
      format: "GLB",
      description: "Manto estilizado com obi largo e mangas largas para animação dinâmica de batalha.",
      author: "ANIGO Asset Team",
    },
    {
      id: "hair_01",
      name: "Twin Tails Dynamic",
      category: "Penteados",
      tags: ["twintails", "ribbon", "hair", "dinamico"],
      polyCount: 4500,
      vertexCount: 2400,
      format: "GLB",
      description: "Penteado maria-chiquinha com física por molas e mechas em curvas bezier 3D.",
      author: "ANIGO Hair Lab",
    },
    {
      id: "hair_02",
      name: "Spiky Shonen Hair",
      category: "Penteados",
      tags: ["spiky", "shonen", "hair", "volumetrico"],
      polyCount: 3800,
      vertexCount: 2100,
      format: "GLB",
      description: "Cabelo volumoso pontiagudo com sombreamento cel-shade em degraus bem definidos.",
      author: "ANIGO Hair Lab",
    },
    {
      id: "prop_01",
      name: "Katana of the Dawn",
      category: "Acessórios",
      tags: ["sword", "weapon", "blade", "arma"],
      polyCount: 2200,
      vertexCount: 1200,
      format: "OBJ",
      description: "Katana tradicional com bainha (saya) e tsuba estilizada com contorno de traço fino.",
      author: "ANIGO Weaponry",
    },
    {
      id: "prop_02",
      name: "School Desk & Chair",
      category: "Acessórios",
      tags: ["desk", "school", "furniture", "sala"],
      polyCount: 1400,
      vertexCount: 800,
      format: "GLB",
      description: "Carteira e cadeira escolar clássica de anime para composição de cenários de sala de aula.",
      author: "ANIGO Props",
    },
    {
      id: "pose_01",
      name: "Dynamic Mid-Air Jump",
      category: "Poses",
      tags: ["action", "jump", "hero", "batalha"],
      polyCount: 0,
      vertexCount: 0,
      format: "VRM",
      description: "Pose acrobática aérea em arco dinâmico para tomadas de ação heroica.",
      author: "ANIGO Posing Lab",
    },
    {
      id: "pose_02",
      name: "Standing Confident Studio",
      category: "Poses",
      tags: ["idle", "studio", "fashion", "postura"],
      polyCount: 0,
      vertexCount: 0,
      format: "VRM",
      description: "Pose de pé equilibrada com distribuição de peso contrapposto sutil.",
      author: "ANIGO Posing Lab",
    },
    {
      id: "mat_01",
      name: "Cel-Shader Anime Skin Ramp",
      category: "Materiais",
      tags: ["cel-shading", "skin", "toon", "pele"],
      polyCount: 0,
      vertexCount: 0,
      format: "ANIGO_MATERIAL",
      description: "Gradiente de cel-shading suave com hue-shift avermelhado para pele anime de alta fidelidade.",
      author: "ANIGO Shading Core",
    },
    {
      id: "mat_02",
      name: "Metallic Inverted Hull Rim",
      category: "Materiais",
      tags: ["metallic", "rim", "outline", "metal"],
      polyCount: 0,
      vertexCount: 0,
      format: "ANIGO_MATERIAL",
      description: "Material metálico com reflexo anisotrópico e rim light de alto contraste.",
      author: "ANIGO Shading Core",
    },
    {
      id: "env_01",
      name: "Sunset Anime Sky Dome",
      category: "Ambientes",
      tags: ["sunset", "clouds", "sky", "por-do-sol"],
      polyCount: 1200,
      vertexCount: 650,
      format: "GLB",
      description: "Doma celeste com gradiente dourado-púrpura e nuvens procedurais estilizadas Makoto Shinkai.",
      author: "ANIGO Sky Dome",
    },
    {
      id: "env_02",
      name: "Cyber Tokyo Neon Atmosphere",
      category: "Ambientes",
      tags: ["cyberpunk", "night", "city", "neon"],
      polyCount: 2500,
      vertexCount: 1400,
      format: "GLB",
      description: "Atmosfera noturna com névoa volumétrica e iluminação de néon azul ciano e magenta.",
      author: "ANIGO Sky Dome",
    },
    {
      id: "stage_01",
      name: "Classroom Modular Greybox",
      category: "Cenários",
      tags: ["classroom", "interior", "modular", "sala"],
      polyCount: 18500,
      vertexCount: 9800,
      format: "GLB",
      description: "Kit modular de paredes, janelas grandes com luz difusa e lousa escolar.",
      author: "ANIGO Stage Builder",
    },
    {
      id: "stage_02",
      name: "Anime Rooftop Stage",
      category: "Cenários",
      tags: ["rooftop", "urban", "stage", "telhado"],
      polyCount: 14200,
      vertexCount: 7600,
      format: "GLB",
      description: "Terraço de escola com grade de proteção metálica, caixas de água e piso de concreto.",
      author: "ANIGO Stage Builder",
    },
  ];
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import SearchIcon from "../icons/SearchIcon.svelte";
  import CloseIcon from "../icons/CloseIcon.svelte";
  import BoxIcon from "../icons/BoxIcon.svelte";
  import UserIcon from "../icons/UserIcon.svelte";
  import ShirtIcon from "../icons/ShirtIcon.svelte";
  import ScissorsIcon from "../icons/ScissorsIcon.svelte";
  import PaletteIcon from "../icons/PaletteIcon.svelte";
  import CloudIcon from "../icons/CloudIcon.svelte";
  import BotIcon from "../icons/BotIcon.svelte";
  import LayersIcon from "../icons/LayersIcon.svelte";
  import CheckIcon from "../icons/CheckIcon.svelte";

  interface Props {
    selectedAsset?: AssetRecord | null;
    onUseAsset?: (asset: AssetRecord) => void;
    onSelectAsset?: (asset: AssetRecord) => void;
  }

  let {
    selectedAsset = $bindable(null),
    onUseAsset,
    onSelectAsset,
  }: Props = $props();

  // Filters State
  let searchQuery = $state("");
  let selectedCategory = $state("Todos");
  let selectedFormat = $state("TODOS");
  let isInspectModalOpen = $state(false);
  let feedbackMessage = $state<string | null>(null);

  export function openInspectModal(asset?: AssetRecord) {
    if (asset) selectedAsset = asset;
    isInspectModalOpen = true;
  }

  onMount(() => {
    if (!selectedAsset && assetCatalog.length > 0) {
      selectedAsset = assetCatalog[0];
      if (onSelectAsset) onSelectAsset(assetCatalog[0]);
    }
  });

  function handleSelect(asset: AssetRecord) {
    selectedAsset = asset;
    if (onSelectAsset) onSelectAsset(asset);
  }

  // Filtered Assets
  let filteredAssets = $derived(
    assetCatalog.filter((asset) => {
      const matchesCategory =
        selectedCategory === "Todos" || asset.category === selectedCategory;
      const matchesFormat =
        selectedFormat === "TODOS" || asset.format === selectedFormat;
      const q = searchQuery.trim().toLowerCase();
      const matchesSearch =
        !q ||
        asset.name.toLowerCase().includes(q) ||
        asset.category.toLowerCase().includes(q) ||
        asset.tags.some((t) => t.toLowerCase().includes(q)) ||
        asset.format.toLowerCase().includes(q);

      return matchesCategory && matchesFormat && matchesSearch;
    })
  );

  function handleCategoryClick(cat: string) {
    selectedCategory = cat;
  }

  export function handleUse(asset: AssetRecord) {
    if (onUseAsset) onUseAsset(asset);
    feedbackMessage = `Ativo "${asset.name}" vinculado ao projeto com sucesso!`;
    setTimeout(() => {
      feedbackMessage = null;
    }, 3500);
  }

  function handleInspect(asset: AssetRecord) {
    selectedAsset = asset;
    isInspectModalOpen = true;
    if (onSelectAsset) onSelectAsset(asset);
  }

  function getCategoryIcon(cat: string) {
    switch (cat) {
      case "Personagens":
        return UserIcon;
      case "Roupas":
        return ShirtIcon;
      case "Penteados":
        return ScissorsIcon;
      case "Acessórios":
        return BoxIcon;
      case "Poses":
        return BotIcon;
      case "Materiais":
        return PaletteIcon;
      case "Ambientes":
      case "Cenários":
        return CloudIcon;
      default:
        return LayersIcon;
    }
  }
</script>

<div class="asset-browser-container">
  <!-- Top Navigation & Search Bar -->
  <header class="browser-header">
    <div class="browser-info">
      <div class="browser-title-row">
        <h2 class="browser-title">BIBLIOTECA DE ATIVOS</h2>
        <span class="catalog-badge">{filteredAssets.length} de {assetCatalog.length} itens</span>
      </div>
      <p class="browser-subtitle">
        Catálogo central de malhas base, roupas, penteados, poses, materiais toon e cenários.
      </p>
    </div>

    <!-- Quick Search Input (Centered with clearance from top-right inspector collapse toggle) -->
    <div class="search-bar-center">
      <div class="search-bar">
        <SearchIcon size={16} />
        <input
          type="text"
          class="search-input"
          placeholder="Buscar por nome, tag ou formato (ex: kimono, vrm, toon, escola)..."
          bind:value={searchQuery}
        />
        {#if searchQuery}
          <button class="btn-clear" onclick={() => (searchQuery = "")} title="Limpar busca">
            <CloseIcon size={13} />
          </button>
        {/if}
      </div>
    </div>
  </header>

  <!-- Notification Banner -->
  {#if feedbackMessage}
    <div class="feedback-banner">
      <CheckIcon size={16} />
      <span>{feedbackMessage}</span>
    </div>
  {/if}

  <!-- Body with Category Sidebar & Cards Grid -->
  <div class="browser-body">
    <!-- Category Sidebar -->
    <aside class="category-sidebar" aria-label="Categorias da Biblioteca">
      <div class="sidebar-heading">CATEGORIAS</div>
      <div class="category-list">
        {#each CANONICAL_ASSET_CATEGORIES as cat}
          {@const count = cat === "Todos" ? assetCatalog.length : assetCatalog.filter((a) => a.category === cat).length}
          <button
            class="cat-item"
            class:active={selectedCategory === cat}
            onclick={() => handleCategoryClick(cat)}
          >
            <span class="cat-name">{cat}</span>
            <span class="cat-count">{count}</span>
          </button>
        {/each}
      </div>

      <div class="format-filter-section">
        <div class="sidebar-heading">FORMATO DE ARQUIVO</div>
        <div class="format-chips">
          {#each ["TODOS", "VRM", "GLB", "OBJ", "ANIGO_MATERIAL"] as fmt}
            <button
              class="chip-btn"
              class:selected={selectedFormat === fmt}
              onclick={() => (selectedFormat = fmt)}
            >
              {fmt}
            </button>
          {/each}
        </div>
      </div>
    </aside>

    <!-- Main Cards Grid -->
    <main class="assets-main">
      {#if filteredAssets.length === 0}
        <div class="empty-state">
          <div class="empty-icon">
            <SearchIcon size={42} />
          </div>
          <div class="empty-title">Nenhum ativo encontrado</div>
          <div class="empty-desc">
            Nenhum item corresponde à pesquisa "{searchQuery}" na categoria "{selectedCategory}".
          </div>
          <button
            class="btn-reset-filters"
            onclick={() => {
              searchQuery = "";
              selectedCategory = "Todos";
              selectedFormat = "TODOS";
            }}
          >
            Redefinir Filtros
          </button>
        </div>
      {:else}
        <div class="assets-grid">
          {#each filteredAssets as asset}
            {@const IconComponent = getCategoryIcon(asset.category)}
            <button
              type="button"
              class="asset-card"
              class:selected={selectedAsset?.id === asset.id}
              onclick={() => handleSelect(asset)}
              ondblclick={() => handleInspect(asset)}
              title="Clique para selecionar e ver detalhes no Inspetor. Duplo clique para inspecionar em janela 3D."
            >
              <!-- Card Header / Thumbnail Area with Format Badge on Top -->
              <div class="card-thumb">
                <div class="thumb-icon-wrap">
                  <IconComponent size={42} />
                </div>
                <div class="badge-format">{asset.format}</div>
              </div>

              <!-- Card Body: Clean Title -->
              <div class="card-body">
                <h3 class="card-title" title={asset.name}>{asset.name}</h3>
              </div>
            </button>
          {/each}
        </div>
      {/if}
    </main>
  </div>

  <!-- Asset Inspection Modal -->
  {#if isInspectModalOpen && selectedAsset}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="modal-backdrop" onclick={() => (isInspectModalOpen = false)}></div>

    <div class="inspect-dialog" role="dialog" aria-modal="true" aria-label="Ficha Técnica do Ativo">
      <header class="dialog-header">
        <div class="dialog-title-info">
          <span class="dialog-cat">{selectedAsset.category}</span>
          <h3 class="dialog-title">{selectedAsset.name}</h3>
        </div>
        <button class="dialog-close btn-close-modal" onclick={() => (isInspectModalOpen = false)} title="Fechar">
          <CloseIcon size={16} />
        </button>
      </header>

      <div class="dialog-body">
        <div class="dialog-thumb-hero">
          {#if selectedAsset.category === "Personagens"}
            <UserIcon size={64} />
          {:else if selectedAsset.category === "Roupas"}
            <ShirtIcon size={64} />
          {:else if selectedAsset.category === "Penteados"}
            <ScissorsIcon size={64} />
          {:else if selectedAsset.category === "Acessórios"}
            <BoxIcon size={64} />
          {:else if selectedAsset.category === "Poses"}
            <BotIcon size={64} />
          {:else if selectedAsset.category === "Materiais"}
            <PaletteIcon size={64} />
          {:else}
            <CloudIcon size={64} />
          {/if}
          <span class="hero-format-tag">{selectedAsset.format}</span>
        </div>

        <p class="dialog-desc">{selectedAsset.description || "Sem descrição disponível."}</p>

        <div class="spec-grid">
          <div class="spec-box">
            <span class="box-label">Polígonos (Tris)</span>
            <span class="box-val">{selectedAsset.polyCount.toLocaleString()}</span>
          </div>
          <div class="spec-box">
            <span class="box-label">Vértices</span>
            <span class="box-val">{selectedAsset.vertexCount.toLocaleString()}</span>
          </div>
          <div class="spec-box">
            <span class="box-label">Formato Canônico</span>
            <span class="box-val">{selectedAsset.format}</span>
          </div>
          <div class="spec-box">
            <span class="box-label">Autor / Equipe</span>
            <span class="box-val">{selectedAsset.author || "ANIGO Studio"}</span>
          </div>
        </div>

        <div class="dialog-tags">
          <span class="tags-title">Tags Relacionadas:</span>
          <div class="tags-row">
            {#each selectedAsset.tags as tag}
              <span class="tag-pill modal-tag">#{tag}</span>
            {/each}
          </div>
        </div>
      </div>

      <footer class="dialog-footer">
        <button class="btn-secondary" onclick={() => (isInspectModalOpen = false)}>
          Fechar
        </button>
        <button
          class="btn-primary"
          onclick={() => {
            if (selectedAsset) handleUse(selectedAsset);
            isInspectModalOpen = false;
          }}
        >
          Carregar no Projeto Ativo
        </button>
      </footer>
    </div>
  {/if}
</div>

<style>
  .asset-browser-container {
    display: flex;
    flex-direction: column;
    width: 100%;
    height: 100%;
    background: #090c14;
    color: #e2e8f0;
    overflow: hidden;
    user-select: none;
  }

  /* Header */
  .browser-header {
    display: flex;
    align-items: center;
    padding: 14px 24px;
    padding-right: 74px; /* Clearance from collapsed inspector toggle button */
    background: #0d121f;
    border-bottom: 1px solid #1a2338;
    gap: 20px;
    flex-shrink: 0;
  }

  .browser-info {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex-shrink: 0;
  }

  .browser-title-row {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .browser-title {
    margin: 0;
    font-size: 1.15rem;
    font-weight: 800;
    letter-spacing: 0.8px;
    color: #f8fafc;
  }

  .catalog-badge {
    font-size: 0.72rem;
    font-weight: 700;
    background: rgba(192, 132, 252, 0.12);
    color: #c084fc;
    border: 1px solid rgba(192, 132, 252, 0.28);
    padding: 2px 8px;
    border-radius: 999px;
  }

  .browser-subtitle {
    margin: 0;
    font-size: 0.78rem;
    color: #94a3b8;
  }

  /* Search Bar Center Wrapper */
  .search-bar-center {
    flex: 1;
    display: flex;
    justify-content: center;
    max-width: 520px;
    margin: 0 auto;
  }

  .search-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    background: #151d2e;
    border: 1px solid #23304a;
    border-radius: 8px;
    padding: 0 12px;
    height: 38px;
    width: 100%;
    max-width: 480px;
    color: #94a3b8;
    transition: all 0.15s ease;
  }

  .search-bar:focus-within {
    border-color: #a855f7;
    background: #182236;
    box-shadow: 0 0 0 3px rgba(168, 85, 247, 0.15);
    color: #c084fc;
  }

  .search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: #f1f5f9;
    font-size: 0.82rem;
    font-family: inherit;
  }

  .search-input::placeholder {
    color: #64748b;
  }

  .btn-clear {
    background: transparent;
    border: none;
    color: #64748b;
    cursor: pointer;
    padding: 2px;
    display: flex;
    align-items: center;
    border-radius: 4px;
  }

  .btn-clear:hover {
    color: #f1f5f9;
    background: #24314c;
  }

  /* Feedback Banner */
  .feedback-banner {
    display: flex;
    align-items: center;
    gap: 10px;
    background: rgba(16, 185, 129, 0.12);
    border-bottom: 1px solid rgba(16, 185, 129, 0.25);
    color: #10b981;
    padding: 8px 24px;
    font-size: 0.8rem;
    font-weight: 600;
  }

  /* Browser Body */
  .browser-body {
    display: flex;
    flex: 1;
    overflow: hidden;
  }

  /* Sidebar */
  .category-sidebar {
    width: 220px;
    background: #0b0f1a;
    border-right: 1px solid #1a2338;
    display: flex;
    flex-direction: column;
    padding: 16px 12px;
    gap: 16px;
    flex-shrink: 0;
    overflow-y: auto;
  }

  .sidebar-heading {
    font-size: 0.65rem;
    font-weight: 800;
    letter-spacing: 1px;
    color: #64748b;
    margin-bottom: 6px;
    padding-left: 6px;
  }

  .category-list {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .cat-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 7px 10px;
    border-radius: 6px;
    background: transparent;
    border: 1px solid transparent;
    color: #94a3b8;
    cursor: pointer;
    font-size: 0.78rem;
    font-weight: 600;
    font-family: inherit;
    transition: all 0.12s ease;
  }

  .cat-item:hover {
    background: #141b2b;
    color: #f1f5f9;
  }

  .cat-item.active {
    background: #241c38;
    border-color: #8b5cf6;
    color: #f1f5f9;
  }

  .cat-count {
    font-size: 0.68rem;
    background: #162032;
    padding: 1px 6px;
    border-radius: 999px;
    color: #64748b;
  }

  .cat-item.active .cat-count {
    background: #4c1d95;
    color: #e9d5ff;
  }

  .format-filter-section {
    border-top: 1px solid #182236;
    padding-top: 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .format-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .chip-btn {
    background: #131b2c;
    border: 1px solid #1f2b42;
    color: #94a3b8;
    font-size: 0.68rem;
    font-weight: 700;
    padding: 3px 8px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.12s ease;
    font-family: inherit;
  }

  .chip-btn:hover {
    background: #1c273e;
    color: #f1f5f9;
  }

  .chip-btn.selected {
    background: #3b2857;
    border-color: #a855f7;
    color: #ffffff;
  }

  /* Main Assets Area */
  .assets-main {
    flex: 1;
    overflow-y: auto;
    padding: 20px;
    background: #080a11;
  }

  .assets-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 14px;
  }

  /* Asset Card */
  .asset-card {
    background: #101624;
    border: 1px solid #1b253b;
    border-radius: 8px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    transition: all 0.15s ease;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
    cursor: pointer;
    padding: 0;
    text-align: left;
    font-family: inherit;
  }

  .asset-card:hover {
    transform: translateY(-2px);
    border-color: #8b5cf6;
    box-shadow: 0 8px 24px rgba(139, 92, 246, 0.18);
  }

  .asset-card.selected {
    border-color: #c084fc;
    background: #161c2e;
    box-shadow: 0 0 0 2px rgba(192, 132, 252, 0.4);
  }

  .card-thumb {
    height: 110px;
    background: linear-gradient(145deg, #131a2c, #0a0e18);
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
    border-bottom: 1px solid #1b253b;
    width: 100%;
  }

  .asset-card.selected .card-thumb {
    background: linear-gradient(145deg, #182038, #0d1322);
    border-bottom-color: rgba(192, 132, 252, 0.3);
  }

  .thumb-icon-wrap {
    color: #c084fc;
    opacity: 0.85;
    transition: transform 0.15s ease;
  }

  .asset-card:hover .thumb-icon-wrap {
    transform: scale(1.1);
    color: #d8b4fe;
    opacity: 1;
  }

  .asset-card.selected .thumb-icon-wrap {
    color: #e9d5ff;
    opacity: 1;
    transform: scale(1.05);
  }

  .badge-format {
    position: absolute;
    top: 6px;
    right: 6px;
    font-size: 0.62rem;
    font-weight: 800;
    background: rgba(15, 23, 42, 0.85);
    border: 1px solid #334155;
    color: #38bdf8;
    padding: 2px 6px;
    border-radius: 4px;
    backdrop-filter: blur(4px);
    letter-spacing: 0.5px;
  }

  .card-body {
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    background: #101624;
  }

  .asset-card.selected .card-body {
    background: #161c2e;
  }

  .card-title {
    margin: 0;
    font-size: 0.78rem;
    font-weight: 700;
    color: #f1f5f9;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .asset-card.selected .card-title {
    color: #ffffff;
  }

  .tag-pill {
    font-size: 0.64rem;
    font-weight: 600;
    color: #a855f7;
    background: rgba(168, 85, 247, 0.1);
    border: 1px solid rgba(168, 85, 247, 0.2);
    padding: 1px 5px;
    border-radius: 4px;
  }

  /* Empty State */
  .empty-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 380px;
    gap: 12px;
    color: #64748b;
    text-align: center;
  }

  .empty-title {
    font-size: 1.05rem;
    font-weight: 700;
    color: #cbd5e1;
  }

  .empty-desc {
    font-size: 0.8rem;
    max-width: 360px;
  }

  .btn-reset-filters {
    margin-top: 8px;
    background: #241c38;
    border: 1px solid #8b5cf6;
    color: #c084fc;
    font-size: 0.78rem;
    font-weight: 700;
    padding: 6px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-family: inherit;
  }

  /* Modal Styles */
  .modal-backdrop {
    position: fixed;
    top: 0;
    left: 0;
    width: 100vw;
    height: 100vh;
    background: rgba(4, 6, 11, 0.75);
    backdrop-filter: blur(8px);
    z-index: 500;
  }

  .inspect-dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    width: 480px;
    max-width: 90vw;
    background: #0f1523;
    border: 1px solid #23304c;
    border-radius: 12px;
    box-shadow: 0 16px 40px rgba(0, 0, 0, 0.6);
    z-index: 510;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .dialog-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 18px;
    background: #141c2e;
    border-bottom: 1px solid #1f2b44;
  }

  .dialog-title-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .dialog-cat {
    font-size: 0.65rem;
    font-weight: 800;
    letter-spacing: 0.5px;
    color: #8b5cf6;
    text-transform: uppercase;
  }

  .dialog-title {
    margin: 0;
    font-size: 0.96rem;
    font-weight: 700;
    color: #f8fafc;
  }

  .dialog-close {
    background: transparent;
    border: none;
    color: #64748b;
    cursor: pointer;
    padding: 4px;
    display: flex;
    align-items: center;
    border-radius: 4px;
  }

  .dialog-close:hover {
    color: #ffffff;
    background: #23304c;
  }

  .dialog-body {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    overflow-y: auto;
    max-height: 60vh;
  }

  .dialog-thumb-hero {
    height: 120px;
    background: linear-gradient(135deg, #182238, #0b0f19);
    border-radius: 8px;
    border: 1px solid #22304d;
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
    color: #c084fc;
  }

  .hero-format-tag {
    position: absolute;
    bottom: 8px;
    right: 8px;
    font-size: 0.65rem;
    font-weight: 800;
    background: #0f1523;
    color: #38bdf8;
    border: 1px solid #334155;
    padding: 2px 8px;
    border-radius: 4px;
  }

  .dialog-desc {
    margin: 0;
    font-size: 0.8rem;
    line-height: 1.45;
    color: #cbd5e1;
  }

  .spec-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .spec-box {
    background: #141c2e;
    border: 1px solid #1f2b44;
    border-radius: 6px;
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .box-label {
    font-size: 0.65rem;
    color: #64748b;
    font-weight: 600;
  }

  .box-val {
    font-size: 0.82rem;
    color: #f1f5f9;
    font-weight: 700;
  }

  .dialog-tags {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .tags-title {
    font-size: 0.7rem;
    font-weight: 700;
    color: #64748b;
  }

  .tags-row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .modal-tag {
    padding: 2px 8px;
    font-size: 0.7rem;
  }

  .dialog-footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 10px;
    padding: 12px 18px;
    background: #111726;
    border-top: 1px solid #1f2b44;
  }

  .btn-secondary {
    background: #1c263c;
    border: 1px solid #2d3d60;
    color: #cbd5e1;
    font-size: 0.78rem;
    font-weight: 600;
    padding: 7px 14px;
    border-radius: 6px;
    cursor: pointer;
    font-family: inherit;
  }

  .btn-secondary:hover {
    background: #24314c;
    color: #ffffff;
  }

  .btn-primary {
    background: #7c3aed;
    border: 1px solid #8b5cf6;
    color: #ffffff;
    font-size: 0.78rem;
    font-weight: 700;
    padding: 7px 16px;
    border-radius: 6px;
    cursor: pointer;
    font-family: inherit;
  }

  .btn-primary:hover {
    background: #8b5cf6;
  }
</style>
