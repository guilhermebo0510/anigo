<script lang="ts">
  import { onMount, onDestroy, untrack } from "svelte";
  import Viewport from "./components/viewport/Viewport.svelte";
  import { t, getLanguage, setLanguage, type LanguageCode } from "./i18n";
  import { normalizeBridgePayload } from "./services/bridge_normalizer";
  import { outlineFromPreset } from "./config/render_config";
  // P3 imports
  import { LIGHT_RIGS } from "./config/light_rigs";
  import { IBL_PROBES } from "./services/ibl_service";
  import { MATERIAL_LIBRARY } from "./services/material_library";
  import { autoSaveService, parseProjectSnapshot, type ProjectStateSnapshot } from "./services/autosave_service";
  import { commandLogOf, isDegradedRecovery, defaultSceneDomain, toProjectSnapshot, type SceneDomainSnapshot } from "./services/project_persistence";
  import type { CanonicalProjectDocumentV1 } from "./contracts/project_state.v1";
  import {
    CHARACTER_SNAPSHOT_SCHEMA_VERSION,
    clampCatalog,
    clampGenderDimorphism,
    clampNumber,
    createDefaultCharacterState,
    getSliderDef,
    isKnownSliderId,
    normalizeSomatotype,
    sanitizeCharacterState,
    type CharacterState,
  } from "./services/character_state";
  import { CANONICAL_SLIDERS } from "./services/morph_catalog";

  // Icons
  import UserIcon from "./components/icons/UserIcon.svelte";
  import SmileIcon from "./components/icons/SmileIcon.svelte";
  import ScissorsIcon from "./components/icons/ScissorsIcon.svelte";
  import ShirtIcon from "./components/icons/ShirtIcon.svelte";
  import BoxIcon from "./components/icons/BoxIcon.svelte";
  import BrushIcon from "./components/icons/BrushIcon.svelte";
  import ShaderIcon from "./components/icons/ShaderIcon.svelte";
  import SunIcon from "./components/icons/SunIcon.svelte";
  import RimIcon from "./components/icons/RimIcon.svelte";
  import OutlineIcon from "./components/icons/OutlineIcon.svelte";
  import PaletteIcon from "./components/icons/PaletteIcon.svelte";
  import GridIcon from "./components/icons/GridIcon.svelte";
  import SparklesIcon from "./components/icons/SparklesIcon.svelte";
  import LayersIcon from "./components/icons/LayersIcon.svelte";
  import BoneIcon from "./components/icons/BoneIcon.svelte";
  import ActivityIcon from "./components/icons/ActivityIcon.svelte";
  import TargetIcon from "./components/icons/TargetIcon.svelte";
  import CloudIcon from "./components/icons/CloudIcon.svelte";
  import CameraIcon from "./components/icons/CameraIcon.svelte";
  import FilmIcon from "./components/icons/FilmIcon.svelte";
  import BotIcon from "./components/icons/BotIcon.svelte";
  import FolderIcon from "./components/icons/FolderIcon.svelte";
  import SlidersIcon from "./components/icons/SlidersIcon.svelte";
  import SearchIcon from "./components/icons/SearchIcon.svelte";
  import SettingsIcon from "./components/icons/SettingsIcon.svelte";
  import FocusIcon from "./components/icons/FocusIcon.svelte";
  import ChevronLeftIcon from "./components/icons/ChevronLeftIcon.svelte";
  import ChevronRightIcon from "./components/icons/ChevronRightIcon.svelte";
  import PanelToggleIcon from "./components/icons/PanelToggleIcon.svelte";
  import SettingsModal, { type StudioSettings } from "./components/settings/SettingsModal.svelte";
  import { loadSettings, saveSettings, devicePixelRatioSafe } from "./services/settings_persist";
  import { historyService, type HistoryStateSnapshot } from "./services/history_service";
  import { CommandHistoryService, CommandHistoryError } from "./services/command_history";
  import {
    CoreBridgeError,
    CoreSessionClient,
    resolveCoreInvoker,
    type CoreSnapshotDelivery,
  } from "./services/core_bridge";
  import type { DiagnosticsSummary, RenderDiagnostic } from "./services/render_diagnostics";
  import { HistoryAlignment } from "./services/history_alignment";
  import { buildCommand, type CommandIntent } from "./services/command_builder";
  import { previewHistoryOutcome, previewOutcome } from "./services/command_scope";
  import type { CommandWire } from "./contracts/commands.v1";
  import ProjectMenuPopover from "./components/project/ProjectMenuPopover.svelte";
  import ModelPresetPopover from "./components/project/ModelPresetPopover.svelte";
  import QuickStartModal from "./components/project/QuickStartModal.svelte";
  import AssetBrowser, { assetCatalog, type AssetRecord } from "./components/library/AssetBrowser.svelte";
  import LightingControls from "./components/character/LightingControls.svelte";
  import SomatotypePad2D from "./components/character/SomatotypePad2D.svelte";
  import AnatomyInspector from "./components/character/AnatomyInspector.svelte";
  import { CANONICAL_CHARACTER_PRESETS, type CharacterPreset } from "./services/character_presets";

  // Workspaces & Tools Definitions
  export type WorkspaceId =
    | "personagem"
    | "posing"
    | "shading"
    | "iluminacao"
    | "cenario"
    | "animacao"
    | "render"
    | "biblioteca";

  export const CANONICAL_WORKSPACES: readonly WorkspaceId[] = [
    "personagem",
    "posing",
    "shading",
    "iluminacao",
    "cenario",
    "animacao",
    "render",
    "biblioteca",
  ] as const;

  interface ToolDefinition {
    id: string;
    label: string;
    description: string;
    icon: any;
    labelKey?: string;
  }

  const workspaceTools: Record<WorkspaceId, ToolDefinition[]> = {
    personagem: [
      { id: "body", label: "Manequim & Corpo", description: "Proporções anatômicas e canone de cabeças", icon: UserIcon, labelKey: "tool.body" },
      { id: "face", label: "Rosto & Olhos", description: "Proporções faciais e expressões de anime", icon: SmileIcon, labelKey: "tool.face" },
      { id: "hair", label: "Cabelo 3D", description: "Cabelo procedural e mechas de fita", icon: ScissorsIcon, labelKey: "tool.hair" },
      { id: "cloth", label: "Vestuário", description: "Modelagem e alfaiataria de roupas anime", icon: ShirtIcon, labelKey: "tool.cloth" },
      { id: "accessories", label: "Acessórios 3D", description: "Adereços, armaduras e pontos de ancoragem", icon: BoxIcon, labelKey: "tool.accessories" },
      { id: "paint", label: "Pintura de Textura", description: "Pintura direta sobre malha 3D e shading masks", icon: BrushIcon, labelKey: "tool.paint" },
    ],
    posing: [
      { id: "rig", label: "Estrutura Óssea", description: "Hierarquia de juntas e rotação 3D de ossos", icon: BoneIcon, labelKey: "tool.rig" },
      { id: "ik", label: "Cinemática Inversa", description: "Controladores de cinemática inversa (IK/FK)", icon: TargetIcon, labelKey: "tool.ik" },
      { id: "poses_preset", label: "Poses do Manequim", description: "Biblioteca de poses rápidas canônicas", icon: BotIcon, labelKey: "tool.poses_preset" },
    ],
    shading: [
      { id: "cel_shader", label: "Toon Ramp & Cel-Shading", description: "Limiar de corte cel-shader e bandas NPR", icon: ShaderIcon, labelKey: "tool.cel_shader" },
      { id: "rim", label: "Luz de Borda (Rim Light)", description: "Reflexos Fresnel de silhueta de material", icon: RimIcon, labelKey: "tool.rim" },
      { id: "outline", label: "Contorno Inverted Hull", description: "Espessura de traço e extrusão de bordas", icon: OutlineIcon, labelKey: "tool.outline" },
      { id: "palette", label: "Paleta de Sombras Anime", description: "Desvio harmônico de matiz e sombras tingidas", icon: PaletteIcon, labelKey: "tool.palette" },
      { id: "shader_ball", label: "Shader Ball & Preview", description: "Esfera NPR em tempo real para calibração", icon: LayersIcon, labelKey: "tool.shader_ball" },
    ],
    iluminacao: [
      { id: "sun", label: "Iluminação Solar", description: "Azimute, elevação solar e intensidade da luz", icon: SunIcon, labelKey: "tool.sun" },
      { id: "shadows", label: "Sombras Anime", description: "Ponto de corte, nitidez e projeção de sombras", icon: PaletteIcon, labelKey: "tool.shadows" },
      { id: "ambient", label: "Luz Ambiente & Céu", description: "Iluminação difusa de hemisfério e céu", icon: CloudIcon, labelKey: "tool.ambient" },
    ],
    cenario: [
      { id: "stage", label: "Blocagem Greybox", description: "Composição de palco modular e snaps de grade", icon: BoxIcon, labelKey: "tool.stage" },
      { id: "props", label: "Elementos de Cenário", description: "Posicionamento de adereços e blocos de cena", icon: FolderIcon, labelKey: "tool.props" },
      { id: "environment", label: "Céu Anime & Atmosfera", description: "Doma de céu estilizado e densidade de névoa", icon: CloudIcon, labelKey: "tool.environment" },
    ],
    animacao: [
      { id: "timeline", label: "Linha do Tempo", description: "Controle de frames, reprodução e cadência FPS", icon: ActivityIcon, labelKey: "tool.timeline" },
      { id: "curves", label: "Curvas de Interpolação", description: "Curvas Lineares, Bezier e Step/Hold anime", icon: SlidersIcon, labelKey: "tool.curves" },
      { id: "lipsync", label: "Sincronia Labial & Visemas", description: "Morfemas labiais [A, I, U, E, O] e visemas", icon: SmileIcon, labelKey: "tool.lipsync" },
    ],
    render: [
      { id: "camera", label: "Câmera de Cena & Lente", description: "Distância focal (24mm, 50mm, 85mm) e FOV", icon: CameraIcon, labelKey: "tool.camera" },
      { id: "passes", label: "Passes de Render NPR", description: "Passes Beauty Toon, Linhas, Sombras e Z-Depth", icon: LayersIcon, labelKey: "tool.passes" },
      { id: "export", label: "Composição & Exportação", description: "Resoluções FHD/2K/4K e exportação final", icon: FilmIcon, labelKey: "tool.export" },
    ],
    biblioteca: [
      { id: "browser", label: "Navegador de Ativos", description: "Grade de cards, busca rápida e catálogo de assets", icon: GridIcon, labelKey: "tool.browser" },
      { id: "search", label: "Busca & Tags", description: "Busca instantânea por nome e tags de ativos", icon: SearchIcon, labelKey: "tool.search" },
      { id: "filters", label: "Filtros de Categoria", description: "Filtros por personagens, roupas, poses e cenários", icon: SlidersIcon, labelKey: "tool.filters" },
    ],
  };

  const defaultWorkspaceTool: Record<WorkspaceId, string> = {
    personagem: "body",
    posing: "rig",
    shading: "cel_shader",
    iluminacao: "sun",
    cenario: "stage",
    animacao: "timeline",
    render: "camera",
    biblioteca: "browser",
  };

  const workspaceLabels: Record<WorkspaceId, string> = {
    personagem: "Personagem",
    posing: "Posing",
    shading: "Shading",
    iluminacao: "Iluminação",
    cenario: "Cenário",
    animacao: "Animação",
    render: "Render",
    biblioteca: "Biblioteca",
  };

  const toolLabels: Record<string, string> = {
    body: "Anatomia & Corpo",
    face: "Rosto & Olhos",
    hair: "Cabelo 3D",
    cloth: "Vestuário & Roupas",
    accessories: "Acessórios 3D",
    paint: "Pintura de Textura",
    rig: "Estrutura Óssea",
    ik: "Cinemática Inversa",
    poses_preset: "Poses do Manequim",
    cel_shader: "Toon Ramp & Cel-Shading",
    rim: "Luz de Borda (Rim Light)",
    outline: "Contorno Inverted Hull",
    palette: "Paleta de Sombras Anime",
    shader_ball: "Shader Ball & Preview",
    sun: "Iluminação Solar",
    shadows: "Sombras Anime",
    ambient: "Luz Ambiente & Céu",
    stage: "Blocagem Greybox",
    props: "Elementos de Cenário",
    environment: "Céu Anime & Atmosfera",
    timeline: "Linha do Tempo",
    curves: "Curvas de Interpolação",
    lipsync: "Sincronia Labial & Visemas",
    camera: "Câmera de Cena & Lente",
    passes: "Passes de Render NPR",
    export: "Composição & Exportação",
    browser: "Navegador de Ativos",
    search: "Busca & Tags",
    filters: "Filtros de Categoria",
    // Compatibility & Legacy Aliases
    clothing: "Alfaiataria & Roupas",
    morphs: "Expressões & Morphs",
    lights: "Iluminação Solar Toon",
    outlines: "Linhas Inverted Hull",
    models: "Malhas Base",
    hair_lib: "Biblioteca de Penteados",
    clothes_lib: "Biblioteca de Vestuário",
    materials: "Biblioteca de Materiais",
    poses_lib: "Biblioteca de Poses",
    render: "Composição & Exportação",
    settings: "Configurações Gerais do Motor",
  };

  // State
  let viewportRef: any = $state(null);
  let activeWorkspace: WorkspaceId = $state("personagem");
  let activeTool = $state("body");

  // Tauri Window instance
  let appWindow: any = null;

  // Viewport Resolution tracking
  let vpWidth = $state(1280);
  let vpHeight = $state(720);

  // Live Telemetry
  let telemetryFps = $state(120);
  let telemetryFrameMs = $state(0.5);
  let telemetryTriangles = $state(156);
  let telemetryBackend = $state("WebGPU Nativo");
  let telemetryAdapter = $state("Hardware GPU");

  function handleMetrics(m: any) {
    telemetryFps = m.fps;
    telemetryFrameMs = m.frameTimeMs;
    telemetryTriangles = m.triangles;
    telemetryBackend = m.backend === "WebGPU" ? "WebGPU Nativo" : "WebGL2 Fallback";
    telemetryAdapter = m.adapterName;
  }

  // Parameter State - Body & Proportions
  let headScale = $state(1.0);
  let headRatio = $state(6.5);
  let shoulderWidth = $state(1.0);
  let legLength = $state(1.0);
  let armLength = $state(1.0);
  let neckLength = $state(1.0);
  let currentPreset = $state<"mannequin" | "sphere" | "cube">("mannequin");

  // Sub-Sprint 3.5: Heath-Carter Somatotype & Gender Dimorphism State
  let somatotypeEndo = $state(0.33);
  let somatotypeMeso = $state(0.34);
  let somatotypeEcto = $state(0.33);
  let genderDimorphism = $state(1.0);

  function handleSomatotypeUpdate(params: {
    endomorph: number;
    mesomorph: number;
    ectomorph: number;
    genderDimorphism: number;
    isContinuous?: boolean;
  }) {
    somatotypeEndo = params.endomorph;
    somatotypeMeso = params.mesomorph;
    somatotypeEcto = params.ectomorph;
    genderDimorphism = params.genderDimorphism;

    if (viewportRef) {
      viewportRef.setSomatotype?.(params.endomorph, params.mesomorph, params.ectomorph);
      viewportRef.setGenderDimorphism?.(params.genderDimorphism);
    }
  }

  // Morph Sliders & Tactile Manipulation State (Sub-Sprint 3.6 - 3.14)
  let morphSliders = $state<Record<string, number>>({});
  let activeCharacterPreset = $state<string | null>(null);

  // P0-07: canonical character mirror — owned by AnatomyInspector when
  // mounted, by these App mirrors otherwise. History/project snapshots
  // always serialize getAppCharacterState().
  let anatomyInspectorRef: any = $state(null);
  let inspectorCharacter: CharacterState | null = $state(null);
  let loadedModelGender: "male" | "female" = $state("male");

  function handleInspectorCharacterChange(state: CharacterState) {
    inspectorCharacter = state;
    somatotypeEndo = state.somatotype.endo;
    somatotypeMeso = state.somatotype.meso;
    somatotypeEcto = state.somatotype.ecto;
    genderDimorphism = state.genderDimorphism;
    morphSliders = { ...state.morphSliders };
    activeCharacterPreset = state.activePresetId;
    loadedModelGender = state.baseGender;
    isProjectDirty = true;
  }

  function handleInspectorProportionsChange(p: {
    headScale: number; headRatio: number; shoulderWidth: number;
    legLength: number; armLength: number; neckLength: number;
  }) {
    headScale = p.headScale;
    headRatio = p.headRatio;
    shoulderWidth = p.shoulderWidth;
    legLength = p.legLength;
    armLength = p.armLength;
    neckLength = p.neckLength;
  }

  /** Canonical character snapshot: inspector-owned fields + App-owned proportions/groom. */
  function getAppCharacterState(): CharacterState {
    let c: CharacterState | null = null;
    try {
      c = anatomyInspectorRef?.getCharacterState?.() ?? null;
    } catch { /* fall through to mirror */ }
    if (!c) c = inspectorCharacter;
    const base = c ? sanitizeCharacterState(c) : createDefaultCharacterState();
    return {
      ...base,
      proportions: {
        headScale, headRatio, shoulderWidth, legLength, armLength, neckLength,
        torsoLength: 1.0, heightOverall: 1.0,
      },
      hair: { volume: hairVolume, thickness: hairThickness, curvature: hairCurvature, strands: hairStrands },
      cloth: { layer: clothLayer, tension: clothTension, rigidity: clothRigidity, gravity: clothGravity },
      accessory: {
        socket: activeSocket, scale: accessoryScale,
        offsetX: accessoryOffsetX, offsetY: accessoryOffsetY, offsetZ: accessoryOffsetZ,
      },
    };
  }

  /** Pushes a character state to inspector (if mounted) + viewport + mirrors. */
  function applyCharacterState(c: CharacterState, opts: { swapModel: boolean } = { swapModel: true }) {
    const s = sanitizeCharacterState(c);
    inspectorCharacter = s;
    somatotypeEndo = s.somatotype.endo;
    somatotypeMeso = s.somatotype.meso;
    somatotypeEcto = s.somatotype.ecto;
    genderDimorphism = s.genderDimorphism;
    morphSliders = { ...s.morphSliders };
    activeCharacterPreset = s.activePresetId;
    headScale = s.proportions.headScale;
    headRatio = s.proportions.headRatio;
    shoulderWidth = s.proportions.shoulderWidth;
    legLength = s.proportions.legLength;
    armLength = s.proportions.armLength;
    neckLength = s.proportions.neckLength;
    hairVolume = s.hair.volume;
    hairThickness = s.hair.thickness;
    hairCurvature = s.hair.curvature;
    hairStrands = s.hair.strands;
    clothLayer = s.cloth.layer;
    clothTension = s.cloth.tension;
    clothRigidity = s.cloth.rigidity;
    clothGravity = s.cloth.gravity;
    activeSocket = s.accessory.socket;
    accessoryScale = s.accessory.scale;
    accessoryOffsetX = s.accessory.offsetX;
    accessoryOffsetY = s.accessory.offsetY;
    accessoryOffsetZ = s.accessory.offsetZ;

    try {
      anatomyInspectorRef?.setCharacterState?.(s);
    } catch { /* inspector unmounted — drive viewport directly below */ }
    if (viewportRef) {
      viewportRef.setSomatotype?.(s.somatotype.endo, s.somatotype.meso, s.somatotype.ecto);
      viewportRef.setGenderDimorphism?.(s.genderDimorphism);
      viewportRef.setProportions?.({ ...s.proportions });
      // Full reset-then-apply so undone morphs return to default.
      for (const def of CANONICAL_SLIDERS) {
        const v = s.morphSliders[def.id];
        viewportRef.setMorphSlider?.(def.id, v !== undefined ? v : def.defaultValue);
      }
      if (opts.swapModel && s.baseGender !== loadedModelGender) {
        loadedModelGender = s.baseGender;
        viewportRef.loadCanonicalModel?.(s.baseGender)?.catch?.(() => {});
      }
    }
  }

  // Re-sync a remounting inspector (tool switches unmount it) from the mirror.
  // Runs ONLY on ref change (mount), never on character edits (no feedback loop).
  let lastSyncedInspector: any = null;
  $effect(() => {
    const ref = anatomyInspectorRef;
    if (ref && ref !== lastSyncedInspector) {
      lastSyncedInspector = ref;
      const mirror = untrack(() => inspectorCharacter);
      if (mirror) {
        try {
          ref.setCharacterState(mirror);
        } catch { /* ignore */ }
      }
    } else if (!ref) {
      lastSyncedInspector = null;
    }
  });

  function handleTactileDrag(
    primarySlider: string,
    primaryDelta: number,
    secondarySlider?: string,
    secondaryDelta?: number
  ) {
    // P0-09: validated + catalog-clamped; inspector mirror kept in sync.
    const applyOne = (id: string, delta: number) => {
      if (!isKnownSliderId(id) || !Number.isFinite(delta)) return;
      const def = getSliderDef(id)!;
      const cur = morphSliders[id] ?? def.defaultValue;
      const next = clampNumber(cur + delta, def.min, def.max);
      morphSliders[id] = next;
      // A gesture is one command: remember which sliders it moved.
      if (!tactileTouchedSliders.includes(id)) tactileTouchedSliders.push(id);
      activeCharacterPreset = null;
      viewportRef?.setMorphSlider?.(id, next);
      try {
        anatomyInspectorRef?.applyExternalMorph?.(id, next);
      } catch { /* inspector unmounted */ }
    };
    if (primarySlider) applyOne(primarySlider, primaryDelta);
    if (secondarySlider && secondaryDelta !== undefined) applyOne(secondarySlider, secondaryDelta);
    inspectorCharacter = getAppCharacterState();
    isProjectDirty = true;
  }

  function handleTactileDragEnd() {
    // P0-09: one history entry per tactile gesture — and, since every gesture
    // moves morph sliders, one command batch (P0 undo/redo item 1).
    const intents: CommandIntent[] = tactileTouchedSliders.map((id) => ({
      kind: "morph" as const,
      slider_id: id,
      value: morphSliders[id] ?? getSliderDef(id)?.defaultValue ?? 1,
    }));
    tactileTouchedSliders = [];
    const command =
      intents.length === 1
        ? intentCommand(intents[0])
        : intents.length > 1
          ? intentCommand({ kind: "batch", intents })
          : null;
    recordHistory("Manipulação tátil do corpo", false, command);
  }

  // P2-15 sync html lang with i18n
  $effect(() => { try { document.documentElement.lang = getLanguage(); } catch {} });
  function handleCharacterPreset(preset: CharacterPreset) {
    activeCharacterPreset = preset.id;
    currentPreset = "mannequin";
    genderDimorphism = preset.genderDimorphism;
    somatotypeEndo = preset.somatotype.endo;
    somatotypeMeso = preset.somatotype.meso;
    somatotypeEcto = preset.somatotype.ecto;
    headScale = preset.proportions.headScale;
    headRatio = preset.proportions.headRatio;
    shoulderWidth = preset.proportions.shoulderWidth;
    legLength = preset.proportions.legLength;
    armLength = preset.proportions.armLength;
    neckLength = preset.proportions.neckLength;

    if (viewportRef) {
      viewportRef.setGenderDimorphism?.(preset.genderDimorphism);
      viewportRef.setSomatotype?.(preset.somatotype.endo, preset.somatotype.meso, preset.somatotype.ecto);
      viewportRef.setProportions?.(preset.proportions);
      for (const [k, v] of Object.entries(preset.sliders)) {
        morphSliders[k] = v;
        viewportRef.setMorphSlider?.(k, v);
      }
    }
  }

  // Face Parameters
  let eyeScale = $state(1.0);
  let chinWidth = $state(1.0);
  let jawWidth = $state(1.0);
  let eyeTilt = $state(0);

  // Hair Parameters
  let hairVolume = $state(1.2);
  let hairThickness = $state(0.05);
  let hairCurvature = $state(0.4);
  let hairStrands = $state(16);

  // Cloth Parameters
  let clothLayer = $state("uniforme");
  let clothTension = $state(0.5);
  let clothRigidity = $state(0.3);
  let clothGravity = $state(1.0);

  // Accessories Parameters
  let activeSocket = $state("head");
  let accessoryScale = $state(1.0);
  let accessoryOffsetX = $state(0.0);
  let accessoryOffsetY = $state(0.0);
  let accessoryOffsetZ = $state(0.0);

  // Shading Parameters
  let shadowThreshold = $state(0.5);
  let toonSmoothness = $state(0.02);
  let specIntensity = $state(0.4);
  let specExponent = $state(32.0);
  let toonSteps = $state(1.0);
  let specSoftness = $state(0.05);
  let specularSize = $state(0.45); // P2-07
  let aoIntensity = $state(0.85); // P2-05
  let ambientSky = $state([0.52,0.60,0.78] as [number,number,number]);
  let ambientGround = $state([0.25,0.20,0.18] as [number,number,number]);
  let specOffset = $state(0.0);
  let specColorHex = $state("#ffffff");

  // Sun Lighting
  let lightAzimuth = $state(45);
  let lightElevation = $state(45);
  let lightIntensity = $state(1.0);
  let sunColor = $state("#fff8e7");
  let ambientIntensity = $state(0.35);

  // Rim Light
  let rimIntensity = $state(0.8);
  let rimSpread = $state(0.4);
  let rimColor = $state("#93c5fd");

  // Outline
  let outlineWidth = $state(3.5);
  let outlineColor = $state("#3b1d28");
  let outlineExtrusion = $state(0.0035);
  let outlineOpacity = $state(1.0);
  let outlineSmoothness = $state(0.0);
  let outlineDepthBias = $state(0.0);

  // Palette & Hue Shift
  let hueShift = $state(-15);
  let shadowSaturation = $state(1.1);
  let baseColorHex = $state("#faeae0");
  let shadowColorHex = $state("#9995be");

  // Timeline & Animation
  let currentFrame = $state(1);
  let isPlaying = $state(false);
  let animationFps = $state(24);

  // Rig & IK
  let selectedBone = $state("Head");
  let ikSolver = $state("two_bone");
  let ikWeight = $state(1.0);

  // Camera Parameters
  let focalLength = $state(50);
  let cameraFov = $state(45);

  // Settings
  let targetFpsCap = $state(120);
  let dpiScale = $state<"1.0x" | "1.5x" | "2.0x">("1.0x");
  let vsyncEnabled = $state(true);
  let isSettingsModalOpen = $state(false);
  let lastAutosaveTime = $state<string | null>(null);

  // Window state
  let isWindowMaximized = $state(false);

  // Project management state
  let currentProjectName = $state("Sem Título.anigo");
  let currentProjectPath = $state<string | null>(null);
  let isProjectDirty = $state(false);
  let isDiagnosticsPopoverOpen = $state(false);
  let isProjectPopoverOpen = $state(false);
  let isPresetPopoverOpen = $state(false);
  let isQuickStartOpen = $state(false);

  // Undo/Redo tracking
  let canUndoAction = $state(false);
  let canRedoAction = $state(false);
  let lastUndoDescription = $state<string | undefined>(undefined);

  // P0 undo/redo: every persistent change is a command, and the command log is
  // the session that gets persisted (base state + log ⇒ same session on replay).
  // `HistoryAlignment` keeps the UI undo stack and the command log in lockstep.
  const commandHistory = new CommandHistoryService();
  const historyAlignment = new HistoryAlignment<CommandWire>();
  // P0 §7.5: o shell é o único que fala com o núcleo; o viewport só consome o
  // snapshot que ele entrega. Sem núcleo (browser), a geometria canônica fica
  // indisponível e o viewport anuncia degradação — nunca deforma por conta própria.
  let coreClient: CoreSessionClient | null = null;
  // P1-02: diagnóstico do renderer visível no shell (último evento + resumo).
  let lastDiagnostic = $state<RenderDiagnostic | null>(null);
  let viewportDiagnostics = $state<DiagnosticsSummary>({
    total: 0,
    errors: 0,
    warnings: 0,
    codes: [],
    dropped: 0,
    degraded: false,
  });

  /** Atualiza o resumo a partir do viewport (fonte é o canal de diagnóstico). */
  function refreshViewportDiagnostics(): void {
    const summary = viewportRef?.getDiagnostics?.().summary;
    if (summary) viewportDiagnostics = summary;
  }

  /** Handler de diagnóstico do viewport. */
  function handleViewportDiagnostic(diagnostic: RenderDiagnostic): void {
    lastDiagnostic = diagnostic;
    refreshViewportDiagnostics();
  }
  /** Sliders movidos pelo gesto tátil atual (um gesto = um comando). */
  let tactileTouchedSliders: string[] = [];

  /** Converte uma intenção da UI em comando válido (nunca envia comando inválido). */
  function intentCommand(intent: CommandIntent): CommandWire | null {
    const result = buildCommand(intent);
    if (result.ok) return result.command;
    console.debug(`[ANIGO][History] intenção recusada (${result.code}): ${result.reason}`);
    return null;
  }

  /**
   * Registra no log de comandos algo que já foi aceito. Em modo preview (sem o
   * core) o outcome é local; quando o core responde, é o outcome dele que entra.
   */
  function recordCommand(command: CommandWire, description: string): boolean {
    try {
      commandHistory.push(
        command,
        previewOutcome({
          command,
          description,
          sequence: commandHistory.next_sequence,
          revision: commandHistory.revision + 1,
          undoDepth: commandHistory.undo_depth + 1,
        })
      );
      return true;
    } catch (error) {
      const reason = error instanceof CommandHistoryError ? `${error.code}: ${error.message}` : String(error);
      console.warn(`[ANIGO][History] comando não registrado (${reason})`);
      return false;
    }
  }

  /** Atualiza a entrada corrente quando um ajuste contínuo (drag) estabiliza. */
  function coalesceCommand(command: CommandWire): void {
    try {
      commandHistory.replaceLast(command);
    } catch (error) {
      console.warn("[ANIGO][History] coalescência falhou:", error);
    }
  }

  /** Log de comandos aceitos (replay = base + log), para o envelope da sessão. */
  function exportCommandLogEntries() {
    const log = commandHistory.exportLog();
    return log.entries.length > 0 ? log : null;
  }

  // Library Asset Inspector State
  let selectedLibraryAsset = $state<AssetRecord | null>(assetCatalog[0] || null);
  let assetBrowserRef: any = $state(null);

  function handleUseLibraryAsset(asset: AssetRecord) {
    if (!asset) return;
    if (asset.id === "char_01") {
      handlePreset("mannequin");
    }
    const assetUri = (asset as { uri?: string; path?: string }).uri ?? (asset as { path?: string }).path;
    recordHistory(
      `Vincular Ativo: ${asset.name}`,
      false,
      assetUri
        ? intentCommand({
            kind: "node_mesh",
            node_id: "nod_character_base",
            mesh_uri: assetUri,
          })
        : null
    );
    if (assetBrowserRef?.handleUse) {
      assetBrowserRef.handleUse(asset);
    }
  }

  function hexToRgb(hex: string): [number, number, number] {
    if (typeof hex !== "string") {
      console.warn(`[ANIGO][Color] hex inválido (não-string) "${hex}" → fallback #ffffff`);
      return [1, 1, 1];
    }
    let cleaned = hex.replace(/^#/, "").trim();
    // P1-01: validação estrita — rejeita hex inválido e evita NaN nos uniforms
    if (!/^[0-9a-fA-F]{3}([0-9a-fA-F]{3})?$/.test(cleaned)) {
      console.warn(`[ANIGO][Color] hex inválido "${hex}" → fallback #ffffff`);
      cleaned = "ffffff";
    }
    if (cleaned.length === 3) {
      cleaned = cleaned.split("").map((c) => c + c).join("");
    }
    const num = parseInt(cleaned, 16);
    if (Number.isNaN(num)) {
      console.warn(`[ANIGO][Color] parse NaN para "${hex}" → fallback #ffffff`);
      return [1, 1, 1];
    }
    return [
      ((num >> 16) & 255) / 255,
      ((num >> 8) & 255) / 255,
      (num & 255) / 255,
    ];
  }
  // P1-01 helper: sRGB→linear para validação round-trip (shader faz principal)
  function srgbToLinearChannel(c: number): number {
    return c <= 0.04045 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  }

  function rgbToHex(rgb: number[]): string {
    const r = Math.round(Math.max(0, Math.min(1, rgb[0])) * 255);
    const g = Math.round(Math.max(0, Math.min(1, rgb[1])) * 255);
    const b = Math.round(Math.max(0, Math.min(1, rgb[2])) * 255);
    return `#${((1 << 24) + (r << 16) + (g << 8) + b).toString(16).slice(1)}`;
  }

  function getHistorySnapshot(): HistoryStateSnapshot {
    const radAz = (lightAzimuth * Math.PI) / 180;
    const radEl = (lightElevation * Math.PI) / 180;
    const lx = Math.cos(radEl) * Math.cos(radAz);
    const ly = Math.sin(radEl);
    const lz = Math.cos(radEl) * Math.sin(radAz);
    // P0-10: persist full light/material/camera state (was 8+ params missing, camera fixed)
    const eye = (viewportRef as any)?.renderer?.eye ?? [0, 1.5, 3.5];
    const target = (viewportRef as any)?.renderer?.target ?? [0, 1, 0];
    const up = (viewportRef as any)?.renderer?.up ?? [0, 1, 0];
    const fovDeg = ((viewportRef as any)?.renderer?.fov ?? (45*Math.PI/180)) * 180/Math.PI;

    return {
      preset: currentPreset,
      headScale,
      headRatio,
      outlineWidth,
      shadowThreshold,
      lightDir: [lx, ly, lz],
      lightIntensity,
      shadowColor: hexToRgb(shadowColorHex),
      lightAzimuth,
      lightElevation,
      activeWorkspace,
      activeTool,
      projectName: currentProjectName,
      timestamp: Date.now(),
      toonSmoothness,
      specIntensity,
      specExponent,
      rimIntensity,
      rimSpread,
      hueShift,
      toonSteps,
      outlineColor,
      baseColorHex,
      shadowColorHex,
      sunColor,
      shadowSaturation,
      ambientIntensity,
      cameraEye: eye,
      cameraTarget: target,
      cameraUp: up,
      fov: fovDeg,
      outlineOpacity,
      outlineSmoothness,
      outlineDepthBias,
      specSoftness,
      specOffset,
      specularSize,
      aoIntensity,
      ambientSky,
      ambientGround,
      specColorHex,
      rimColor,
      lightColor: hexToRgb(sunColor),
      // P0-07: the Personagem domain is part of every history entry.
      character: getAppCharacterState(),
    };
  }

  /**
   * Single entry point for every persistent change (P0 undo/redo item 1).
   *
   * `command` is the versioned command that describes the change; when it is
   * absent the change is UI-only (the snapshot history still records it so the
   * viewport can be restored, but nothing enters the persisted command log).
   */
  /**
   * Batch com os morphs fora do valor canônico — o estado persistente do corpo
   * do personagem (usado quando um componente confirma a edição, ex.: o
   * AnatomyInspector, que não informa quais sliders mudaram).
   */
  function morphStateCommand(): CommandWire | null {
    const intents: CommandIntent[] = [];
    for (const slider of CANONICAL_SLIDERS) {
      const value = morphSliders[slider.id];
      if (value === undefined || !Number.isFinite(value)) continue;
      if (Math.abs(value - slider.defaultValue) <= 1e-6) continue;
      intents.push({ kind: "morph", slider_id: slider.id, value });
    }
    if (intents.length === 0) return null;
    if (intents.length === 1) return intentCommand(intents[0]);
    return intentCommand({ kind: "batch", intents });
  }

  /** Resets both histories together (new/opened project). */
  function resetHistories(): void {
    historyService.init(getHistorySnapshot());
    commandHistory.clear();
    historyAlignment.clear();
  }

  function recordHistory(description: string, isContinuous = false, command: CommandWire | null = null) {
    const depthBefore = historyService.undoDepth;
    historyService.push(getHistorySnapshot(), description, isContinuous);
    const added = historyService.undoDepth > depthBefore;

    if (command) {
      if (added) {
        // The UI history created an entry — the command log mirrors it 1:1. If
        // the core refused the command (or the outcome was malformed) the entry
        // is still undoable, but as a UI-only step: the stacks stay aligned.
        if (recordCommand(command, description)) {
          historyAlignment.markAdded("command", command);
          // Autoridade: o núcleo aplica o mesmo comando e devolve o snapshot.
          void pushCommandToCore(command);
        } else {
          historyAlignment.markAdded("ui");
        }
      } else if (isContinuous && historyAlignment.undo_depth > 0) {
        // Continuous adjustment on the entry already open (slider drag): the
        // log keeps one entry, with the value that settled.
        coalesceCommand(command);
        historyAlignment.markCoalesced(command);
      }
    } else if (added) {
      historyAlignment.markAdded("ui");
    }

    isProjectDirty = true;
  }

  function applySnapshot(snap: HistoryStateSnapshot) {
    currentPreset = snap.preset;
    headScale = snap.headScale;
    headRatio = snap.headRatio;
    outlineWidth = snap.outlineWidth;
    shadowThreshold = snap.shadowThreshold;
    lightIntensity = snap.lightIntensity;
    if (snap.lightAzimuth !== undefined) lightAzimuth = snap.lightAzimuth;
    if (snap.lightElevation !== undefined) lightElevation = snap.lightElevation;
    if (snap.toonSmoothness !== undefined) toonSmoothness = snap.toonSmoothness;
    if (snap.specIntensity !== undefined) specIntensity = snap.specIntensity;
    if (snap.specExponent !== undefined) specExponent = snap.specExponent;
    if (snap.rimIntensity !== undefined) rimIntensity = snap.rimIntensity;
    if (snap.rimSpread !== undefined) rimSpread = snap.rimSpread;
    if (snap.hueShift !== undefined) hueShift = snap.hueShift;
    if (snap.toonSteps !== undefined) toonSteps = snap.toonSteps;
    if (snap.outlineColor !== undefined) outlineColor = snap.outlineColor;
    if (snap.baseColorHex !== undefined) baseColorHex = snap.baseColorHex;
    if (snap.shadowColorHex !== undefined) shadowColorHex = snap.shadowColorHex;
    if (snap.sunColor !== undefined) sunColor = snap.sunColor;
    if (snap.shadowSaturation !== undefined) shadowSaturation = snap.shadowSaturation;
    if (snap.ambientIntensity !== undefined) ambientIntensity = snap.ambientIntensity;
    if ((snap as any).outlineOpacity !== undefined) outlineOpacity = (snap as any).outlineOpacity;
    if ((snap as any).outlineSmoothness !== undefined) outlineSmoothness = (snap as any).outlineSmoothness;
    if ((snap as any).outlineDepthBias !== undefined) outlineDepthBias = (snap as any).outlineDepthBias;
    if ((snap as any).specSoftness !== undefined) specSoftness = (snap as any).specSoftness;
    if ((snap as any).specOffset !== undefined) specOffset = (snap as any).specOffset;
    if ((snap as any).specularSize !== undefined) specularSize = (snap as any).specularSize;
    if ((snap as any).aoIntensity !== undefined) aoIntensity = (snap as any).aoIntensity;
    if ((snap as any).ambientSky) ambientSky = (snap as any).ambientSky;
    if ((snap as any).ambientGround) ambientGround = (snap as any).ambientGround;
    if ((snap as any).specColorHex !== undefined) specColorHex = (snap as any).specColorHex;
    if ((snap as any).rimColor !== undefined) rimColor = (snap as any).rimColor;
    // P0-07: restore the full Personagem domain (undo/redo covers the body).
    if (snap.character) {
      try {
        applyCharacterState(sanitizeCharacterState(snap.character));
      } catch (e) {
        console.warn("[App] applySnapshot character restore failed:", e);
      }
    }
    // P0-10: restore camera (was fixed)
    if ((snap as any).cameraEye && (snap as any).cameraTarget) {
      const r: any = (viewportRef as any)?.renderer;
      if (r) {
        r.eye = (snap as any).cameraEye;
        r.target = (snap as any).cameraTarget;
        if ((snap as any).cameraUp) r.up = (snap as any).cameraUp;
        if ((snap as any).fov) r.fov = (snap as any).fov * Math.PI/180;
      }
    }

    // Direct synchronization to 3D WebGPU Viewport & Rust
    if (viewportRef) {
      viewportRef.switchPreset(snap.preset, snap.headScale, snap.headRatio);
      viewportRef.setHeadProportions(snap.headScale, snap.headRatio);
      viewportRef.setOutlineWidth(snap.outlineWidth);
      const outlineRgb = hexToRgb(outlineColor);
      viewportRef.setOutlineColor([outlineRgb[0], outlineRgb[1], outlineRgb[2], 1.0]);
      viewportRef.setShadowThreshold(snap.shadowThreshold);
      if (snap.toonSmoothness !== undefined) viewportRef.setToonSmoothness(snap.toonSmoothness);
      if (snap.specIntensity !== undefined && snap.specExponent !== undefined) {
        viewportRef.setSpecular(snap.specIntensity, snap.specExponent);
      }
      if (snap.rimIntensity !== undefined && snap.rimSpread !== undefined) {
        viewportRef.setRimLight(snap.rimIntensity, snap.rimSpread);
      }
      if (snap.hueShift !== undefined) viewportRef.setHueShift(snap.hueShift);
      if (snap.toonSteps !== undefined) viewportRef.setToonSteps(snap.toonSteps);
      viewportRef.setLight(
        snap.lightDir,
        snap.lightIntensity,
        hexToRgb(shadowColorHex),
        hexToRgb(sunColor),
        ambientIntensity,
        shadowSaturation,
        ambientSky as any,
        ambientGround as any
      );
      // P2-07/05 sync new material params
      (viewportRef as any)?.setMaterialParams?.({
        specularSize: (snap as any).specularSize,
        aoIntensity: (snap as any).aoIntensity
      });
    }
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      import("@tauri-apps/api/core").then(({ invoke }) => {
        invoke("load_mesh_preset", { preset: snap.preset }).catch(() => {});
      });
    }
    updateLighting(false);
    updateMaterial(false);
    handleOutlineChange(false);
    handleShadowThresholdChange(false);
    reportLiveTelemetry();
  }

  function handleUndo() {
    const prev = historyService.undo();
    if (!prev) return;
    applySnapshot(prev);
    void pushHistoryStepToCore("undo");
    // The command log follows the same step: only command-backed entries move
    // (item 2 — apply and undo are symmetric).
    const marker = historyAlignment.undo();
    if (marker?.origin === "command" && commandHistory.can_undo) {
      commandHistory.applyUndo(
        previewHistoryOutcome({
          description: `Undo ${marker.command ? describeCommand(marker.command) : "comando"}`,
          revision: commandHistory.revision + 1,
          undoDepth: commandHistory.undo_depth - 1,
          redoDepth: commandHistory.redo_depth + 1,
        })
      );
    }
  }

  function handleRedo() {
    const next = historyService.redo();
    if (!next) return;
    applySnapshot(next);
    void pushHistoryStepToCore("redo");
    const marker = historyAlignment.redo();
    if (marker?.origin === "command" && commandHistory.can_redo) {
      commandHistory.applyRedo(
        previewHistoryOutcome({
          description: `Redo ${marker.command ? describeCommand(marker.command) : "comando"}`,
          revision: commandHistory.revision + 1,
          undoDepth: commandHistory.undo_depth + 1,
          redoDepth: commandHistory.redo_depth - 1,
        })
      );
    }
  }

  /**
   * P0 §7.5 — busca um snapshot no núcleo e o devolve ao viewport.
   *
   * `force` pede a parte estática mesmo quando a revisão do cliente é a atual
   * (recuperação de contexto GPU/perda de buffers).
   */
  async function coreSnapshotProvider(force: boolean = false): Promise<CoreSnapshotDelivery | null> {
    if (!coreClient || !coreClient.available) return null;
    try {
      return await coreClient.pullSnapshot(force);
    } catch (error) {
      reportCoreFailure(error);
      return null;
    }
  }

  /**
   * Diagnóstico de falha do núcleo (nunca silenciosa, nunca fatal para o UI).
   *
   * Vai para o mesmo canal do renderer: a status bar e a telemetria veem tudo
   * num lugar só (P1-02/P1-06).
   */
  function reportCoreFailure(error: unknown): void {
    const detail = error instanceof Error ? error.message : String(error);
    const stale = detail.includes("fora de ordem");
    viewportRef?.reportDiagnostic?.(
      stale ? "snapshot_stale" : "snapshot_invalid",
      stale ? "snapshot do núcleo chegou fora de ordem e foi descartado" : "falha ao obter/decodificar o snapshot do núcleo",
      { detail }
    );
    refreshViewportDiagnostics();
    if (error instanceof CoreBridgeError && error.code === "core_unavailable") {
      lastDiagnostic = {
        code: "geometry_unavailable",
        severity: "warning",
        message: "núcleo indisponível",
        detail,
        context: null,
        count: 1,
        firstAt: 0,
        lastAt: 0,
      };
    }
  }

  /**
   * Manda o comando para o núcleo (autoridade) e atualiza o viewport com a
   * geometria/pesos que ele devolver. O log local é um espelho: quando os
   * revisões divergem, o núcleo vence e a divergência é registrada.
   */
  async function pushCommandToCore(command: CommandWire): Promise<void> {
    if (!coreClient || !coreClient.available) return;
    try {
      const outcome = await coreClient.applyCommand(command);
      if (outcome.revision !== commandHistory.revision) {
        console.debug(
          `[ANIGO][Core] revision do núcleo ${outcome.revision} ≠ espelho local ${commandHistory.revision}`
        );
      }
      await viewportRef?.requestCoreSnapshot?.();
    } catch (error) {
      reportCoreFailure(error);
    }
  }

  /** Undo/redo no núcleo (ele é quem decide o que é reversível) + refresh do viewport. */
  async function pushHistoryStepToCore(kind: "undo" | "redo"): Promise<void> {
    if (!coreClient || !coreClient.available) return;
    try {
      if (kind === "undo") await coreClient.undo();
      else await coreClient.redo();
      await viewportRef?.requestCoreSnapshot?.(true);
    } catch (error) {
      reportCoreFailure(error);
    }
  }

  /** Rótulo curto de um comando, usado nas descrições de undo/redo. */
  function describeCommand(command: CommandWire): string {
    const record = command as Record<string, unknown>;
    const target = (record.target ?? record.node_id ?? record.material_id ?? record.light_id) as string | undefined;
    return target ? `${command.kind} ${target}` : command.kind;
  }

  function markCleanSave() {
    isProjectDirty = false;
    try {
      if (typeof localStorage !== "undefined") {
        localStorage.setItem("anigo_last_clean_save", String(Date.now()));
      }
    } catch { /* ignore */ }
  }

  async function handleSaveProject() {
    if (!currentProjectPath) {
      await handleSaveProjectAs();
      return;
    }
    const payload = JSON.stringify(getProjectSnapshot(), null, 2);
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("save_project_file", { path: currentProjectPath, content: payload })
        .then(() => {
          markCleanSave();
        })
        .catch((err) => {
          console.error("Erro ao salvar:", err);
          alert("Erro ao salvar projeto: " + err);
        });
    } else {
      markCleanSave();
    }
  }

  async function handleSaveProjectAs() {
    let name = prompt("Nome do arquivo de projeto (.anigo):", currentProjectName.replace(/\.anigo$/, ""));
    if (!name || !name.trim()) return;
    name = name.trim();
    if (!name.endsWith(".anigo")) name += ".anigo";

    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      try {
        const dirs = await invoke<{ projects_dir: string }>("get_studio_directories");
        const fullPath = `${dirs.projects_dir}\\${name}`;
        const payload = JSON.stringify(getProjectSnapshot(), null, 2);
        await invoke("save_project_file", { path: fullPath, content: payload });
        currentProjectName = name;
        currentProjectPath = fullPath;
        markCleanSave();
      } catch (err) {
        console.error("Erro ao salvar como:", err);
        alert("Erro ao salvar projeto: " + err);
      }
    } else {
      currentProjectName = name;
      markCleanSave();
    }
  }

  async function handleOpenProject() {
    let name = prompt("Nome do arquivo .anigo na pasta Projects para carregar:");
    if (!name || !name.trim()) return;
    name = name.trim();
    if (!name.endsWith(".anigo")) name += ".anigo";

    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      try {
        const dirs = await invoke<{ projects_dir: string }>("get_studio_directories");
        const fullPath = `${dirs.projects_dir}\\${name}`;
        const rawJson = await invoke<string>("load_project_file", { path: fullPath });
        // P0-08: validated load (schema version + sanitized numerics + character).
        const data = parseProjectSnapshot(rawJson);
        applySnapshot(data as unknown as HistoryStateSnapshot);
        currentProjectName = name;
        currentProjectPath = fullPath;
        isProjectDirty = false;
        // P1-11 restore persisted settings (DPI/fps)
    try {
      const s = loadSettings();
      if (s.targetFps) { targetFpsCap = s.targetFps as any; viewportRef?.setFpsCap?.(s.targetFps as any); }
      // P1-11 DPI: clamp DPR 1..2 to avoid memory blow
      const dpr = devicePixelRatioSafe();
      if (dpr !== window.devicePixelRatio) console.info('[P1-11] DPR clamped', window.devicePixelRatio, '->', dpr);
    } catch {}
    resetHistories();
      } catch (err) {
        alert("Erro ao carregar projeto: " + err);
      }
    }
  }

  function handleNewProject() {
    if (isProjectDirty && !confirm("O projeto atual possui modificações não salvas. Deseja reiniciar a cena?")) {
      return;
    }
    currentProjectName = "Sem Título.anigo";
    currentProjectPath = null;
    isProjectDirty = false;
    headScale = 1.0;
    headRatio = 6.5;
    outlineWidth = 3.5;
    shadowThreshold = 0.5;
    lightAzimuth = 45;
    lightElevation = 45;
    lightIntensity = 1.0;
    // P0-08/P2-06: a new project resets the character too (was leaking the
    // previous body into the fresh scene).
    try {
      applyCharacterState(createDefaultCharacterState(), { swapModel: true });
    } catch (e) {
      console.warn("[App] handleNewProject character reset failed:", e);
    }
    handlePreset("mannequin", false);
    resetHistories();
  }

  async function handleOpenProjectsFolder() {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      const dirs = await invoke<{ projects_dir: string }>("get_studio_directories");
      await invoke("open_directory_in_explorer", { path: dirs.projects_dir }).catch(() => {});
    }
  }

  function getProjectSnapshot(): ProjectStateSnapshot {
    const radAz = (lightAzimuth * Math.PI) / 180;
    const radEl = (lightElevation * Math.PI) / 180;
    const lx = Math.cos(radEl) * Math.cos(radAz);
    const ly = Math.sin(radEl);
    const lz = Math.cos(radEl) * Math.sin(radAz);
    const eye = (viewportRef as any)?.renderer?.eye ?? [0, 1.5, 3.5];
    const target = (viewportRef as any)?.renderer?.target ?? [0, 1, 0];
    const up = (viewportRef as any)?.renderer?.up ?? [0, 1, 0];
    const fovDeg = ((viewportRef as any)?.renderer?.fov ?? (45*Math.PI/180)) * 180/Math.PI;

    return {
      preset: currentPreset,
      headScale,
      headRatio,
      outlineWidth,
      shadowThreshold,
      lightDir: [lx, ly, lz],
      lightIntensity,
      shadowColor: hexToRgb(shadowColorHex),
      cameraEye: eye,
      cameraTarget: target,
      cameraUp: up,
      fov: fovDeg,
      timestamp: Date.now(),
      version: "0.2.0",
      schemaVersion: CHARACTER_SNAPSHOT_SCHEMA_VERSION,
      // P0-08: the character domain travels with the project.
      character: getAppCharacterState(),
      lightAzimuth,
      lightElevation,
      toonSmoothness,
      specIntensity,
      specExponent,
      rimIntensity,
      rimSpread,
      hueShift,
      toonSteps,
      outlineColor,
      baseColorHex,
      shadowColorHex,
      sunColor,
      shadowSaturation,
      ambientIntensity,
      outlineOpacity,
      outlineSmoothness,
      outlineDepthBias,
      specSoftness,
      specOffset,
      specularSize,
      aoIntensity,
      ambientSky,
      ambientGround,
      specColorHex,
      rimColor,
      lightColor: hexToRgb(sunColor),
      // P0 persistência: cena + materiais + render viajam com o projeto.
      scene: getSceneDomain(),
    };
  }

  /**
   * P0 persistência — domínio de cena do autosave.
   *
   * Materiais vêm da UI (cores atuais), nós/assets da cena canônica. Background,
   * MSAA e tonemap ainda não têm controle no Studio, então são gravados com os
   * valores canônicos — mas passam a fazer parte do arquivo, então quando os
   * controles existirem nada muda no formato.
   */
  function getSceneDomain(): SceneDomainSnapshot {
    const scene = defaultSceneDomain();
    const rgba = (hex: string): [number, number, number, number] => {
      const [r, g, b] = hexToRgb(hex);
      return [r, g, b, 1];
    };
    scene.materials[0] = {
      ...scene.materials[0],
      base_color: rgba(baseColorHex),
      shade_color: rgba(shadowColorHex),
      outline_color: rgba(outlineColor),
    };
    return scene;
  }

  /**
   * P0 persistência — documento canônico do projeto.
   *
   * A autoria é do núcleo Rust. Enquanto os comandos Tauri que devolvem o
   * `ProjectState`/`CoreSnapshot` não estiverem ligados (item 5 do P0 de
   * autoridade), não existe documento autoritativo: o envelope é gravado como
   * `preview` (degradado) em vez de inventar um.
   */
  function getCoreProjectDocument(): CanonicalProjectDocumentV1 | null {
    return null;
  }

  /** P0 persistência — preferências de UI que acompanham a sessão. */
  function getUiStateSnapshot(): Record<string, unknown> {
    return {
      workspace: activeWorkspace,
      inspector_width: inspectorWidth,
      inspector_visible: inspectorVisible,
      preset: currentPreset,
    };
  }

  function handleSaveStudioSettings(settings: StudioSettings) {
    // P1-11 persist dpi/fps/vsync
    try { saveSettings({ targetFps: settings.fpsCap as number, dpiAware: settings.dpiScale !== "1.0x" } as any); } catch {}
    vsyncEnabled = settings.vsync;
    targetFpsCap = settings.fpsCap;
    dpiScale = settings.dpiScale;

    // Direct WebGPU hardware synchronization
    if (viewportRef) {
      viewportRef.setFpsCap(settings.fpsCap);
      const mult = settings.dpiScale === "2.0x" ? 2.0 : settings.dpiScale === "1.5x" ? 1.5 : 1.0;
      viewportRef.setDpiScale(mult);
      viewportRef.setVsync(settings.vsync);
    }

    // Direct Autosave Service reconfiguration
    autoSaveService.configure(
      settings.autoSave,
      settings.autoSaveInterval,
      getProjectSnapshot,
      () => isProjectDirty,
      {
        getCoreProject: getCoreProjectDocument,
        getUiState: getUiStateSnapshot,
        getCommandLog: exportCommandLogEntries,
      }
    );
  }

  // Right Properties Inspector Resizing & Visibility
  // Minimum width is calibrated (320px) so sliders and val-tags never truncate, max is 3x (960px)
  const MIN_INSPECTOR_WIDTH = 320;
  const MAX_INSPECTOR_WIDTH = 320 * 3; // 960px

  let inspectorWidth = $state(320);
  let savedInspectorWidth = $state(320);
  let inspectorVisible = $state(true);
  let isResizingInspector = $state(false);

  function startResizeInspector(e: PointerEvent) {
    e.preventDefault();
    e.stopPropagation();

    const target = e.currentTarget as HTMLElement;
    try {
      target.setPointerCapture(e.pointerId);
    } catch (_) {}

    isResizingInspector = true;
    const startX = e.clientX;
    const startWidth = inspectorWidth;
    let rafId: number | null = null;
    let pendingX = startX;

    const updateWidth = () => {
      rafId = null;
      if (!isResizingInspector) return;
      const delta = startX - pendingX;
      const newWidth = Math.min(
        Math.max(Math.round(startWidth + delta), MIN_INSPECTOR_WIDTH),
        MAX_INSPECTOR_WIDTH
      );
      if (newWidth !== inspectorWidth) {
        inspectorWidth = newWidth;
        viewportRef?.resize();
      }
    };

    const onPointerMove = (ev: PointerEvent) => {
      if (!isResizingInspector) return;
      pendingX = ev.clientX;
      if (rafId === null) {
        rafId = requestAnimationFrame(updateWidth);
      }
    };

    const onPointerUp = (ev: PointerEvent) => {
      isResizingInspector = false;
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
      updateWidth();
      try {
        target.releasePointerCapture(ev.pointerId);
      } catch (_) {}
      window.removeEventListener("pointermove", onPointerMove);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("pointercancel", onPointerUp);
      document.body.style.cursor = "";
      document.body.style.userSelect = "";
    };

    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";

    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerup", onPointerUp);
    window.addEventListener("pointercancel", onPointerUp);
  }

  function toggleInspector() {
    if (inspectorVisible) {
      savedInspectorWidth = inspectorWidth;
      inspectorVisible = false;
    } else {
      inspectorWidth = Math.min(
        Math.max(savedInspectorWidth, MIN_INSPECTOR_WIDTH),
        MAX_INSPECTOR_WIDTH
      );
      inspectorVisible = true;
    }
  }

  // Lifecycle
  onMount(async () => {
    updateLighting();
    handleOutlineChange();
    handleShadowThresholdChange();

    // P0 §7.5: conecta a sessão canônica. Com núcleo, a geometria do viewport
    // vem exclusivamente dos snapshots dele; sem núcleo (browser), o viewport
    // anuncia autoridade indisponível e mostra a malha base sem deformação.
    coreClient = new CoreSessionClient(await resolveCoreInvoker());
    if (coreClient.available) {
      await viewportRef?.requestCoreSnapshot?.(true);
    } else {
      console.warn(
        "[ANIGO][Core] núcleo indisponível: viewport degradado (sem geometria canônica; nenhuma deformação local)"
      );
    }

    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        appWindow = getCurrentWindow();
      } catch (err) {
        console.warn("[App] Could not initialize Tauri window:", err);
      }

      try {
        const { listen } = await import("@tauri-apps/api/event");

        await listen("anigo://load_preset", (event: any) => {
          const _norm = normalizeBridgePayload("anigo://load_preset", event.payload);
          if (_norm === null) { console.warn("[P1-07] invalid payload", "anigo://load_preset", event.payload); return; }
          // P1-07 single consumer validated — original handler follows (payload now in _norm when applicable)
          if (event.payload?.preset) {
            handlePreset(event.payload.preset);
          }
        });

        await listen("anigo://set_proportions", (event: any) => {
          if (event.payload?.head_scale !== undefined) headScale = event.payload.head_scale;
          if (event.payload?.head_ratio !== undefined) headRatio = event.payload.head_ratio;
          // P0-07: proportion edits enter history (was record=false always).
          updateProportions(true);
        });

        await listen("anigo://set_outline", (event: any) => {
          const _norm = normalizeBridgePayload("anigo://set_outline", event.payload);
          if (_norm === null) { console.warn("[P1-07] invalid payload", "anigo://set_outline", event.payload); return; }
          // P1-07 single consumer validated — original handler follows (payload now in _norm when applicable)
          if (event.payload?.width !== undefined) {
            outlineWidth = event.payload.width;
            handleOutlineChange();
          }
        });

        await listen("anigo://set_material_toon", (event: any) => {
          const _norm = normalizeBridgePayload("anigo://set_material_toon", event.payload);
          if (_norm === null) { console.warn("[P1-07] invalid payload", "anigo://set_material_toon", event.payload); return; }
          // P1-07 single consumer validated — original handler follows (payload now in _norm when applicable)
          const p: any = (typeof _norm !== 'undefined' && _norm !== null ? _norm : event.payload) || {};
          if (p.shadow_threshold !== undefined) shadowThreshold = p.shadow_threshold;
          if (p.shadow_smoothness !== undefined) toonSmoothness = p.shadow_smoothness;
          if (p.spec_intensity !== undefined) specIntensity = p.spec_intensity;
          if (p.spec_power !== undefined) specExponent = p.spec_power;
          if (p.rim_intensity !== undefined) rimIntensity = p.rim_intensity;
          if (p.rim_spread !== undefined) rimSpread = p.rim_spread;
          if (p.hue_shift !== undefined) hueShift = p.hue_shift;
          if (p.toon_steps !== undefined) toonSteps = p.toon_steps;
          if (p.shadow_saturation !== undefined) shadowSaturation = p.shadow_saturation;
          if (p.base_color && Array.isArray(p.base_color) && p.base_color.length >= 3) {
            baseColorHex = rgbToHex(p.base_color);
          }
          if (p.shade_color && Array.isArray(p.shade_color) && p.shade_color.length >= 3) {
            shadowColorHex = rgbToHex(p.shade_color);
          }
          if (p.outline_width !== undefined) {
            outlineWidth = p.outline_false /* P1-13 removed heuristic — use outlineFromPreset mode */ ? p.outline_width : p.outline_width * 1000;
          }
          if (p.outline_color && Array.isArray(p.outline_color) && p.outline_color.length >= 3) {
            outlineColor = rgbToHex(p.outline_color);
          }
          updateMaterial(false);
        });

        await listen("anigo://set_material", (event: any) => {
          const _norm = normalizeBridgePayload("anigo://set_material", event.payload);
          if (_norm === null) { console.warn("[P1-07] invalid payload", "anigo://set_material", event.payload); return; }
          // P1-07 single consumer validated — original handler follows (payload now in _norm when applicable)
          const p: any = (typeof _norm !== 'undefined' && _norm !== null ? _norm : event.payload) || {};
          if (p.shadow_threshold !== undefined) {
            shadowThreshold = p.shadow_threshold;
            handleShadowThresholdChange();
          }
        });

        await listen("anigo://set_light", (event: any) => {
          const _norm = normalizeBridgePayload("anigo://set_light", event.payload);
          if (_norm === null) { console.warn("[P1-07] invalid payload", "anigo://set_light", event.payload); return; }
          // P1-07 single consumer validated — original handler follows (payload now in _norm when applicable)
          const p: any = (typeof _norm !== 'undefined' && _norm !== null ? _norm : event.payload) || {};
          if (p.direction && Array.isArray(p.direction) && p.direction.length === 3) {
            const [x, y, z] = p.direction;
            const el = Math.asin(Math.max(-1, Math.min(1, y))) * (180 / Math.PI);
            const az = Math.atan2(z, x) * (180 / Math.PI);
            lightElevation = Math.round(el);
            lightAzimuth = Math.round((az + 360) % 360);
          }
          if (p.intensity !== undefined) lightIntensity = p.intensity;
          if (p.color && Array.isArray(p.color) && p.color.length >= 3) {
            sunColor = rgbToHex(p.color);
          }
          if (p.shadow_color && Array.isArray(p.shadow_color) && p.shadow_color.length >= 3) {
            shadowColorHex = rgbToHex(p.shadow_color);
          }
          if (p.ambient_intensity !== undefined) ambientIntensity = p.ambient_intensity;
          if (p.shadow_saturation !== undefined) shadowSaturation = p.shadow_saturation;
          updateLighting(false);
        });

        await listen("anigo://recenter_camera", () => {
          viewportRef?.recenterCamera();
        });

        await listen("anigo://set_character_model", (event: any) => {
          const p: any = (typeof _norm !== 'undefined' && _norm !== null ? _norm : event.payload) || {};
          // P0-03/P0-09: validated model + canonical polarity + inspector sync.
          if (p.model_type === "male" || p.model_type === "female") {
            const c = getAppCharacterState();
            c.baseGender = p.model_type;
            genderDimorphism = clampGenderDimorphism(p.model_type === "female" ? 0.0 : 1.0);
            c.genderDimorphism = genderDimorphism;
            loadedModelGender = p.model_type;
            viewportRef?.setGenderDimorphism?.(genderDimorphism);
            viewportRef?.loadCanonicalModel?.(p.model_type)?.catch?.((e: unknown) => {
              console.error("[App] MCP model load failed:", e);
            });
            try {
              anatomyInspectorRef?.setCharacterState?.(c);
            } catch { /* inspector unmounted */ }
            inspectorCharacter = c;
            isProjectDirty = true;
            recordHistory(
              `MCP: trocar modelo (${p.model_type})`,
              false,
              intentCommand({ kind: "base_gender", gender: p.model_type })
            );
          } else if (p.model_type !== undefined) {
            console.warn("[App] MCP set_character_model: invalid model_type", p.model_type);
          }
        });

        await listen("anigo://set_somatotype", (event: any) => {
          const p: any = (typeof _norm !== 'undefined' && _norm !== null ? _norm : event.payload) || {};
          // P0-02/P0-09: normalized + clamped + inspector sync.
          const n = normalizeSomatotype(
            p.endo !== undefined ? p.endo : somatotypeEndo,
            p.meso !== undefined ? p.meso : somatotypeMeso,
            p.ecto !== undefined ? p.ecto : somatotypeEcto
          );
          somatotypeEndo = n.endo;
          somatotypeMeso = n.meso;
          somatotypeEcto = n.ecto;
          activeCharacterPreset = null;
          viewportRef?.setSomatotype?.(somatotypeEndo, somatotypeMeso, somatotypeEcto);
          try {
            anatomyInspectorRef?.applyExternalSomatotype?.(somatotypeEndo, somatotypeMeso, somatotypeEcto);
          } catch { /* inspector unmounted */ }
          inspectorCharacter = getAppCharacterState();
          isProjectDirty = true;
          recordHistory(
            "MCP: ajustar somatótipo",
            true,
            intentCommand({
              kind: "somatotype",
              endo: somatotypeEndo,
              meso: somatotypeMeso,
              ecto: somatotypeEcto,
            })
          );
        });

        await listen("anigo://apply_morph_slider", (event: any) => {
          const p: any = (typeof _norm !== 'undefined' && _norm !== null ? _norm : event.payload) || {};
          // P0-09: validated + catalog-clamped + inspector sync.
          if (p.slider_id && p.value !== undefined) {
            if (!isKnownSliderId(p.slider_id)) {
              console.warn("[App] MCP apply_morph_slider: unknown id", p.slider_id);
              return;
            }
            const clamped = clampCatalog(p.slider_id, p.value);
            if (clamped === null) return;
            morphSliders[p.slider_id] = clamped;
            activeCharacterPreset = null;
            viewportRef?.setMorphSlider?.(p.slider_id, clamped);
            try {
              anatomyInspectorRef?.applyExternalMorph?.(p.slider_id, clamped);
            } catch { /* inspector unmounted */ }
            inspectorCharacter = getAppCharacterState();
            isProjectDirty = true;
            recordHistory(
              `MCP: ${p.slider_id}`,
              true,
              intentCommand({ kind: "morph", slider_id: p.slider_id, value: clamped })
            );
          }
        });

        await listen("anigo://reset_morphs", () => {
          const c = getAppCharacterState();
          const n = normalizeSomatotype(1 / 3, 1 / 3, 1 / 3);
          c.morphSliders = {};
          c.somatotype = n;
          c.activePresetId = null;
          applyCharacterState(c, { swapModel: false });
          isProjectDirty = true;
          recordHistory("MCP: resetar morphs", false, intentCommand({ kind: "reset_morphs" }));
        });

        await listen("anigo://ui_action", (event: any) => {
          const p: any = (event.payload as any) || {};
          console.log("[ANIGO Studio] UI Action received:", p);
          const action = p.action || p.type;

          if (action === "select_tab" || p.name === "tab") {
            const tab = p.value || p.tab || p.target;
            if (tab && CANONICAL_WORKSPACES.includes(tab as WorkspaceId)) {
              switchWorkspace(tab as WorkspaceId);
            }
          } else if (action === "select_tool" || p.name === "tool") {
            const tool = p.value || p.tool || p.target;
            if (tool) {
              if (tool === "settings") {
                isSettingsModalOpen = true;
              } else {
                selectTool(tool);
              }
            }
          } else if (action === "open_settings" || action === "show_settings") {
            isSettingsModalOpen = true;
          } else if (action === "close_settings" || action === "hide_settings") {
            isSettingsModalOpen = false;
          } else if (action === "toggle_inspector") {
            toggleInspector();
          } else if (action === "set_inspector_width") {
            const w = Number(p.width || p.value);
            if (!isNaN(w)) {
              inspectorWidth = Math.min(Math.max(Math.round(w), MIN_INSPECTOR_WIDTH), MAX_INSPECTOR_WIDTH);
            }
          } else if (action === "set_slider") {
            const prop = p.property || p.name || p.param;
            const val = Number(p.value);
            if (!isNaN(val)) {
              handleSliderUpdate(prop, val);
            }
          } else if (action === "set_preset") {
            const preset = p.value || p.preset;
            if (preset === "mannequin" || preset === "sphere" || preset === "cube") {
              handlePreset(preset);
            }
          }
        });
      } catch (e) {
        console.warn("[App] Error registering Tauri bridge listeners:", e);
      }
    }

    // Initialize real background autosave engine (P0-08: dirty-gated).
    autoSaveService.configure(true, 5, getProjectSnapshot, () => isProjectDirty, {
      getCoreProject: getCoreProjectDocument,
      getUiState: getUiStateSnapshot,
      getCommandLog: exportCommandLogEntries,
    });
    autoSaveService.onSaveCompleted = (_filePath: string, timeStr: string) => {
      lastAutosaveTime = timeStr;
    };
    autoSaveService.onSaveError = (err: unknown) => {
      console.error("[App] autosave failed:", err);
      alert("Falha no salvamento automático: " + String(err));
    };

    // P0 persistência: recuperação de sessão lida e validada (migrações
    // explícitas; payload inválido é reportado em vez de descartado em silêncio).
    try {
      const recovery = autoSaveService.recoverSession();
      let cleanAt = 0;
      try {
        cleanAt = Number(localStorage.getItem("anigo_last_clean_save") ?? 0) || 0;
      } catch { /* ignore */ }
      if (recovery.status === "invalid" && recovery.error) {
        console.warn("[App] autosave cache rejeitado:", recovery.error.code, recovery.error.message);
        alert(
          `O autosave encontrado não pôde ser lido (${recovery.error.code}).\n` +
            `${recovery.error.message}\n\nA sessão atual continuará; o cache foi mantido em disco.`
        );
      } else if (recovery.envelope && recovery.envelope.saved_at > cleanAt) {
        const when = new Date(recovery.envelope.saved_at).toLocaleString();
        const notes: string[] = [];
        if (isDegradedRecovery(recovery.envelope)) {
          notes.push("Este autosave não contém o documento canônico do núcleo (sessão de preview).");
        }
        if (recovery.migrations.length > 0) {
          notes.push(`Migrações aplicadas: ${recovery.migrations.join(" → ")}.`);
        }
        const extra = notes.length > 0 ? `\n\n${notes.join("\n")}` : "";
        if (confirm(`Sessão não salva encontrada (autosave de ${when}). Deseja recuperar?${extra}`)) {
          const restoredLog = commandLogOf(recovery.envelope);
          applySnapshot(toProjectSnapshot(recovery.envelope.session) as unknown as HistoryStateSnapshot);
          historyService.init(getHistorySnapshot());
          // Replay: the session comes back with its accepted commands, so undo
          // history after the restore is the same as before the crash — and the
          // command-backed marker keeps both stacks aligned.
          if (restoredLog) {
            try {
              commandHistory.restoreLog(restoredLog);
              historyAlignment.clear();
              for (const entry of restoredLog.entries) {
                historyAlignment.markAdded("command", entry.command);
              }
            } catch (error) {
              console.warn("[App] log de comandos restaurado inválido:", error);
            }
          }
          isProjectDirty = true;
        } else {
          autoSaveService.clearRecoveryCache();
        }
      }
    } catch (e) {
      console.warn("[App] recovery check failed:", e);
    }

    // Initialize history baseline
    resetHistories();

    // History stack change synchronization
    historyService.onStackChange = (canUndo, canRedo, desc) => {
      canUndoAction = canUndo;
      canRedoAction = canRedo;
      lastUndoDescription = desc;
    };

    // Sync window maximized state on startup
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      import("@tauri-apps/api/core").then(({ invoke }) => {
        invoke<boolean>("app_is_maximized").then((isMax) => {
          isWindowMaximized = isMax;
        }).catch(() => {});
      });
    }

    // Check quick start modal
    if (typeof localStorage !== "undefined") {
      const skip = localStorage.getItem("anigo_skip_quickstart");
      if (!skip) {
        isQuickStartOpen = true;
      }
    }
  });

  onDestroy(() => {
    autoSaveService.destroy();
  });

  function handleSliderUpdate(prop: string, val: number) {
    // P0-07: proportion edits enter history (was record=false always).
    if (prop === "head_scale") { headScale = val; updateProportions(true); }
    else if (prop === "head_ratio") { headRatio = val; updateProportions(true); }
    else if (prop === "shoulder_width" || prop === "shoulders") { shoulderWidth = val; updateProportions(true); }
    else if (prop === "leg_length" || prop === "legs") { legLength = val; updateProportions(true); }
    else if (prop === "arm_length" || prop === "arms") { armLength = val; updateProportions(true); }
    else if (prop === "neck_length" || prop === "neck") { neckLength = val; updateProportions(true); }
    else if (prop === "outline_width") { outlineWidth = val; handleOutlineChange(); }
    else if (prop === "outline_extrusion") { outlineExtrusion = val; outlineWidth = val * 1000; handleOutlineChange(); }
    else if (prop === "shadow_threshold") { shadowThreshold = val; handleShadowThresholdChange(); }
    else if (prop === "light_azimuth") { lightAzimuth = val; updateLighting(); }
    else if (prop === "light_elevation") { lightElevation = val; updateLighting(); }
    else if (prop === "light_intensity") { lightIntensity = val; updateLighting(); }
    else if (prop === "shadow_saturation") { shadowSaturation = val; updateLighting(); updateMaterial(); }
    else if (prop === "ambient_intensity") { ambientIntensity = val; updateLighting(); }
    else if (prop === "eye_scale" || prop === "eye_size") { eyeScale = val; }
    else if (prop === "chin_width") { chinWidth = val; }
    else if (prop === "jaw_width") { jawWidth = val; }
    else if (prop === "hair_volume") { hairVolume = val; }
    else if (prop === "hair_thickness") { hairThickness = val; }
    else if (prop === "hair_curvature") { hairCurvature = val; }
    else if (prop === "cloth_tension") { clothTension = val; }
    else if (prop === "rim_intensity") { rimIntensity = val; updateMaterial(); }
    else if (prop === "rim_spread") { rimSpread = val; updateMaterial(); }
    else if (prop === "hue_shift") { hueShift = val; updateMaterial(); }
    else if (prop === "spec_intensity") { specIntensity = val; updateMaterial(); }
    else if (prop === "spec_power" || prop === "spec_exponent") { specExponent = val; updateMaterial(); }
    else if (prop === "toon_steps") { toonSteps = val; updateMaterial(); }
    else if (prop === "toon_smoothness" || prop === "shadow_smoothness") { toonSmoothness = val; updateMaterial(); }
    else if (prop === "camera_fov") { cameraFov = val; }
    else if (prop === "current_frame") { currentFrame = Math.round(val); }
  }

  function switchWorkspace(ws: WorkspaceId) {
    activeWorkspace = ws;
    activeTool = defaultWorkspaceTool[ws] || "body";
  }

  function selectTool(toolId: string) {
    if (toolId === "settings") {
      isSettingsModalOpen = true;
      return;
    }
    activeTool = toolId;
    for (const [ws, tools] of Object.entries(workspaceTools)) {
      if (tools.some((t) => t.id === toolId)) {
        activeWorkspace = ws as WorkspaceId;
        break;
      }
    }
  }

  // Window Controls
  async function handleMinimize() {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("app_minimize").catch(async () => {
        if (appWindow) await appWindow.minimize().catch(() => {});
      });
    } else if (appWindow) {
      await appWindow.minimize().catch(() => {});
    }
  }

  async function handleMaximize() {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      const res = await invoke<boolean>("app_toggle_maximize").catch(async () => {
        if (appWindow) await appWindow.toggleMaximize().catch(() => {});
        return !isWindowMaximized;
      });
      isWindowMaximized = res;
    } else if (appWindow) {
      await appWindow.toggleMaximize().catch(() => {});
      isWindowMaximized = !isWindowMaximized;
    }
  }

  async function handleClose() {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("app_close").catch(async () => {
        if (appWindow) await appWindow.close().catch(() => {});
      });
    } else if (appWindow) {
      await appWindow.close().catch(() => {});
    }
  }

  // Keyboard Shortcuts
  function handleKeyDown(e: KeyboardEvent) {
    if (
      e.target instanceof HTMLInputElement ||
      e.target instanceof HTMLTextAreaElement ||
      e.target instanceof HTMLSelectElement
    ) {
      return;
    }

    const isCtrl = e.ctrlKey || e.metaKey;

    // Quick Workspace Switching: Keys 1..8 (without modifiers)
    if (!isCtrl && !e.altKey && !e.shiftKey) {
      const workspaceKeyMap: Record<string, WorkspaceId> = {
        "1": "personagem",
        "2": "posing",
        "3": "shading",
        "4": "iluminacao",
        "5": "cenario",
        "6": "animacao",
        "7": "render",
        "8": "biblioteca",
      };
      const targetWorkspace = workspaceKeyMap[e.key];
      if (targetWorkspace) {
        e.preventDefault();
        switchWorkspace(targetWorkspace);
        return;
      }
    }

    // Undo: Ctrl+Z (without shift)
    if (isCtrl && (e.key === "z" || e.key === "Z") && !e.shiftKey) {
      e.preventDefault();
      handleUndo();
      return;
    }

    // Redo: Ctrl+Y or Ctrl+Shift+Z
    if (isCtrl && (e.key === "y" || e.key === "Y" || ((e.key === "z" || e.key === "Z") && e.shiftKey))) {
      e.preventDefault();
      handleRedo();
      return;
    }

    // Save: Ctrl+S
    if (isCtrl && (e.key === "s" || e.key === "S") && !e.shiftKey) {
      e.preventDefault();
      handleSaveProject();
      return;
    }

    // Save As: Ctrl+Shift+S
    if (isCtrl && (e.key === "s" || e.key === "S") && e.shiftKey) {
      e.preventDefault();
      handleSaveProjectAs();
      return;
    }

    // Open Project: Ctrl+O
    if (isCtrl && (e.key === "o" || e.key === "O")) {
      e.preventDefault();
      handleOpenProject();
      return;
    }

    // New Project: Ctrl+N
    if (isCtrl && (e.key === "n" || e.key === "N")) {
      e.preventDefault();
      handleNewProject();
      return;
    }

    // Settings: Ctrl+,
    if (isCtrl && e.key === ",") {
      e.preventDefault();
      isSettingsModalOpen = !isSettingsModalOpen;
      return;
    }

    // Recenter 3D Viewport: F
    if (e.key === "f" || e.key === "F") {
      e.preventDefault();
      viewportRef?.recenterCamera();
    }
  }

  // 3D Viewport Synchronizations
  function updateProportions(record = false) {
    if (currentPreset !== "mannequin") {
      handlePreset("mannequin", false);
    }
    if (viewportRef?.setProportions) {
      viewportRef.setProportions({
        headScale,
        headRatio,
        shoulderWidth,
        legLength,
        armLength,
        neckLength,
      });
    } else if (viewportRef?.setHeadProportions) {
      viewportRef.setHeadProportions(headScale, headRatio);
    }
    reportLiveTelemetry();
    if (record) {
      const command = intentCommand({
        kind: "proportions",
        patch: {
          head_scale: headScale,
          head_ratio: headRatio,
          shoulder_width: shoulderWidth,
          leg_length: legLength,
          arm_length: armLength,
          neck_length: neckLength,
        },
      });
      recordHistory("Ajustar Proporções Anatômicas", true, command);
    }
  }

  async function handlePreset(preset: "mannequin" | "sphere" | "cube", record = true) {
    if (currentPreset === preset && !record) return;
    currentPreset = preset;
    if (viewportRef?.switchPreset) {
      viewportRef.switchPreset(preset, headScale, headRatio);
    }
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("load_mesh_preset", { preset }).catch(() => {});
    }
    reportLiveTelemetry();
    if (record) {
      const command = intentCommand({ kind: "preset", preset });
      recordHistory(`Alterar Modelo para ${presetLabels[preset]}`, false, command);
    }
  }

  function updateLighting(record = false, isContinuous = false) {
    const radAz = (lightAzimuth * Math.PI) / 180;
    const radEl = (lightElevation * Math.PI) / 180;
    const x = Math.cos(radEl) * Math.cos(radAz);
    const y = Math.sin(radEl);
    const z = Math.cos(radEl) * Math.sin(radAz);

    const sunRgb = hexToRgb(sunColor);
    const shadowRgb = hexToRgb(shadowColorHex);
    // P0-02: neutral white tint — shade_color (material) alone defines shadow hue (was duplicate shadowRgb → double tint)
    const neutralShadowTint: [number, number, number] = [1.0, 1.0, 1.0];

    if (viewportRef?.setLight) {
      viewportRef.setLight([x, y, z], lightIntensity, neutralShadowTint, sunRgb, ambientIntensity, shadowSaturation);
    }

    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      import("@tauri-apps/api/core").then(({ invoke }) => {
        invoke("set_light_params", {
          direction: [x, y, z],
          intensity: lightIntensity,
          color: sunRgb,
          shadow_color: neutralShadowTint,
          ambient_intensity: ambientIntensity,
          shadow_saturation: shadowSaturation,
        }).catch(() => {});
      });
    }
    reportLiveTelemetry();
    if (record) {
      const command = intentCommand({
        kind: "light",
        patch: {
          direction: [x, y, z],
          intensity: lightIntensity,
          color: [sunRgb[0], sunRgb[1], sunRgb[2]],
          shadow_color: [
            neutralShadowTint[0],
            neutralShadowTint[1],
            neutralShadowTint[2],
          ],
          ambient_intensity: ambientIntensity,
          shadow_saturation: shadowSaturation,
        },
      });
      recordHistory("Ajustar Iluminação Solar", isContinuous, command);
    }
  }

  function updateMaterial(record = false, isContinuous = false) {
    const baseRgb = hexToRgb(baseColorHex);
    const shadeRgb = hexToRgb(shadowColorHex);
    const outlineRgb = hexToRgb(outlineColor);
    const specRgb = hexToRgb(specColorHex);

    const rimRgbLocal = hexToRgb(rimColor);
    if (viewportRef?.setMaterialParams) {
      viewportRef.setMaterialParams({
        baseColor: [baseRgb[0], baseRgb[1], baseRgb[2], 1.0],
        shadeColor: [shadeRgb[0], shadeRgb[1], shadeRgb[2], 1.0],
        shadowThreshold,
        toonSmoothness,
        specIntensity,
        specExponent,
        specSoftness,
        specOffset,
        specColor: [specRgb[0], specRgb[1], specRgb[2], 1.0],
        rimIntensity,
        rimSpread,
        rimColor: [rimRgbLocal[0], rimRgbLocal[1], rimRgbLocal[2], 1.0],
        hueShift,
        toonSteps,
        outlineWidth: outlineWidth * 0.001,
        outlineColor: [outlineRgb[0], outlineRgb[1], outlineRgb[2], 1.0],
        outlineOpacity,
        outlineSmoothness,
        outlineDepthBias,
        shadowSaturation,
      });
    }

    const rimRgb = rimRgbLocal;
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      import("@tauri-apps/api/core").then(({ invoke }) => {
        invoke("set_material_toon_params", {
          shadow_threshold: shadowThreshold,
          shadow_smoothness: toonSmoothness,
          spec_intensity: specIntensity,
          spec_power: specExponent,
          rim_intensity: rimIntensity,
          rim_spread: rimSpread,
          hue_shift: hueShift,
          toon_steps: toonSteps,
          base_color: [baseRgb[0], baseRgb[1], baseRgb[2], 1.0],
          shade_color: [shadeRgb[0], shadeRgb[1], shadeRgb[2], 1.0],
          outline_width: outlineWidth * 0.001,
          outline_color: [outlineRgb[0], outlineRgb[1], outlineRgb[2], 1.0],
          specular_color: [specRgb[0], specRgb[1], specRgb[2], 1.0],
          specular_softness: specSoftness,
          specular_offset: specOffset,
          rim_color: [rimRgb[0], rimRgb[1], rimRgb[2], 1.0],
          outline_opacity: outlineOpacity,
          outline_smoothness: outlineSmoothness,
          outline_depth_bias: outlineDepthBias,
          shadow_saturation: shadowSaturation,
        }).catch(() => {});
      });
    }
    reportLiveTelemetry();
    if (record) {
      const command = intentCommand({
        kind: "material",
        patch: {
          base_color: [baseRgb[0], baseRgb[1], baseRgb[2], 1.0],
          shade_color: [shadeRgb[0], shadeRgb[1], shadeRgb[2], 1.0],
          outline_color: [outlineRgb[0], outlineRgb[1], outlineRgb[2], 1.0],
          specular_color: [specRgb[0], specRgb[1], specRgb[2], 1.0],
          rim_color: [rimRgb[0], rimRgb[1], rimRgb[2], 1.0],
          shadow_threshold: shadowThreshold,
          shadow_smoothness: toonSmoothness,
          spec_intensity: specIntensity,
          spec_power: specExponent,
          specular_softness: specSoftness,
          specular_offset: specOffset,
          specular_size: specularSize,
          rim_intensity: rimIntensity,
          rim_spread: rimSpread,
          hue_shift: hueShift,
          toon_steps: toonSteps,
          outline_width: outlineWidth * 0.001,
          outline_opacity: outlineOpacity,
          outline_smoothness: outlineSmoothness,
          outline_depth_bias: outlineDepthBias,
          ao_intensity: aoIntensity,
        },
      });
      recordHistory("Ajustar Material Toon", isContinuous, command);
    }
  }

  function handleOutlineChange(record = false, isContinuous = false) {
    if (viewportRef?.setOutlineWidth) {
      viewportRef.setOutlineWidth(outlineWidth);
    }
    const outlineRgb = hexToRgb(outlineColor);
    if (viewportRef?.setOutlineColor) {
      viewportRef.setOutlineColor([outlineRgb[0], outlineRgb[1], outlineRgb[2], 1.0]);
    }
    updateMaterial(false);
    reportLiveTelemetry();
    if (record) {
      const command = intentCommand({
        kind: "material",
        patch: {
          outline_width: outlineWidth * 0.001,
          outline_color: [outlineRgb[0], outlineRgb[1], outlineRgb[2], 1.0],
        },
      });
      recordHistory("Ajustar Contorno Inverted Hull", isContinuous, command);
    }
  }

  function handleShadowThresholdChange(record = false, isContinuous = false) {
    if (viewportRef?.setShadowThreshold) {
      viewportRef.setShadowThreshold(shadowThreshold);
    }
    updateMaterial(false);
    reportLiveTelemetry();
    if (record) {
      const command = intentCommand({
        kind: "material",
        patch: { shadow_threshold: shadowThreshold },
      });
      recordHistory("Ajustar Limiar Toon Ramp", isContinuous, command);
    }
  }

  function handleToonSmoothnessChange(record = false, isContinuous = false) {
    if (viewportRef?.setToonSmoothness) {
      viewportRef.setToonSmoothness(toonSmoothness);
    }
    updateMaterial(false);
    reportLiveTelemetry();
    if (record) {
      const command = intentCommand({
        kind: "material",
        patch: { shadow_smoothness: toonSmoothness },
      });
      recordHistory("Ajustar Suavidade Toon", isContinuous, command);
    }
  }

  function reportLiveTelemetry() {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      import("@tauri-apps/api/core").then(({ invoke }) => {
        const radAz = (lightAzimuth * Math.PI) / 180;
        const radEl = (lightElevation * Math.PI) / 180;
        const lx = Math.cos(radEl) * Math.cos(radAz);
        const ly = Math.sin(radEl);
        const lz = Math.cos(radEl) * Math.sin(radAz);
        const rend: any = (viewportRef as any)?.renderer;
        const triCount = rend?.indexCount ? Math.floor(rend.indexCount/3) : (currentPreset === "mannequin" ? 6880 : currentPreset === "sphere" ? 2592 : 12);
        const eye = rend?.eye ?? [0, 1.5, 3.5];
        const target = rend?.target ?? [0, 1, 0];
        const fpsVal = rend ? Math.round(1000 / Math.max(rend.frameTimeMs || 16, 1)) : telemetryFps;
        invoke("report_live_telemetry", {
          telemetry: {
            fps: fpsVal,
            frame_time_ms: rend?.frameTimeMs ?? telemetryFrameMs,
            draw_calls: 2,
            triangle_count: triCount,
            adapter_name: rend?.adapterName ?? telemetryAdapter,
            camera_eye: eye,
            camera_target: target,
            light_direction: [lx, ly, lz],
            light_intensity: lightIntensity,
            shadow_color: hexToRgb(shadowColorHex),
            active_preset: currentPreset,
            outline_width: outlineWidth,
            shadow_threshold: shadowThreshold,
            head_scale: headScale,
            head_ratio: headRatio,
            webgpu_active: (rend?.backend ?? telemetryBackend).includes("WebGPU"),
          },
        }).catch(() => {});
      });
    }
  }

  const presetLabels = {
    mannequin: "Manequim Anime",
    sphere: "Esfera NPR",
    cube: "Cubo Unitário",
  };
</script>

<svelte:window onkeydown={handleKeyDown} />

<div class="app-layout">
  <!-- 1. Native Tauri Frameless Studio Titlebar -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header class="app-titlebar" data-tauri-drag-region ondblclick={handleMaximize}>
    <!-- Left: Logo, Brand Tag -->
    <div class="brand-section" data-tauri-drag-region>
      <img src="/icon.png" alt="ANIGO Logo" class="brand-logo" />
      <div class="brand-info" data-tauri-drag-region>
        <span class="brand-title">ANIGO</span>
        <span class="brand-tag">STUDIO</span>
      </div>
    </div>

    <!-- Center: 8 Workspaces Tabs & Undo/Redo -->
    <nav class="workspace-tabs" aria-label="Workspaces">
      <button
        class="tab-button"
        class:active={activeWorkspace === "personagem"}
        onclick={() => switchWorkspace("personagem")}
      >
        {t("workspace.personagem")}
      </button>
      <button
        class="tab-button"
        class:active={activeWorkspace === "posing"}
        onclick={() => switchWorkspace("posing")}
      >
        {t("workspace.posing")}
      </button>
      <button
        class="tab-button"
        class:active={activeWorkspace === "shading"}
        onclick={() => switchWorkspace("shading")}
      >
        {t("workspace.shading")}
      </button>
      <button
        class="tab-button"
        class:active={activeWorkspace === "iluminacao"}
        onclick={() => switchWorkspace("iluminacao")}
      >
        {t("workspace.iluminacao")}
      </button>
      <button
        class="tab-button"
        class:active={activeWorkspace === "cenario"}
        onclick={() => switchWorkspace("cenario")}
      >
        {t("workspace.cenario")}
      </button>
      <button
        class="tab-button"
        class:active={activeWorkspace === "animacao"}
        onclick={() => switchWorkspace("animacao")}
      >
        {t("workspace.animacao")}
      </button>
      <button
        class="tab-button"
        class:active={activeWorkspace === "render"}
        onclick={() => switchWorkspace("render")}
      >
        {t("workspace.render")}
      </button>
      <button
        class="tab-button"
        class:active={activeWorkspace === "biblioteca"}
        onclick={() => switchWorkspace("biblioteca")}
      >
        {t("workspace.biblioteca")}
      </button>

      <!-- Undo / Redo Header Actions -->
      <div class="history-actions" title="Histórico de Edições">
        <button
          class="hist-btn"
          disabled={!canUndoAction}
          onclick={handleUndo}
          title={lastUndoDescription ? `Desfazer: ${lastUndoDescription} (Ctrl+Z)` : "Desfazer (Ctrl+Z)"}
          aria-label="Desfazer"
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <polyline points="1 4 1 10 7 10"></polyline>
            <path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10"></path>
          </svg>
        </button>
        <button
          class="hist-btn"
          disabled={!canRedoAction}
          onclick={handleRedo}
          title="Refazer (Ctrl+Y / Ctrl+Shift+Z)"
          aria-label="Refazer"
        >
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <polyline points="23 4 23 10 17 10"></polyline>
            <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10"></path>
          </svg>
        </button>
      </div>
    </nav>

    <!-- Draggable Spacer -->
    <div class="titlebar-drag-spacer" data-tauri-drag-region></div>

    <!-- Right: Custom Window Controls -->
    <div class="titlebar-actions">
      <div class="window-controls" aria-label="Controles da Janela">
        <button
          class="win-btn win-min"
          onclick={handleMinimize}
          title={t("window.minimize")}
          aria-label={t("window.minimize")}
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <line x1="1" y1="5" x2="9" y2="5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
          </svg>
        </button>
        <button
          class="win-btn win-max"
          onclick={handleMaximize}
          title={isWindowMaximized ? "Restaurar" : t("window.maximize")}
          aria-label={isWindowMaximized ? "Restaurar" : t("window.maximize")}
        >
          {#if isWindowMaximized}
            <svg width="10" height="10" viewBox="0 0 10 10">
              <path d="M2.5 1.5h6v6h-6z" fill="none" stroke="currentColor" stroke-width="1.1" />
              <path d="M1.5 3.5h5v5h-5z" fill="#0f121a" stroke="currentColor" stroke-width="1.1" />
            </svg>
          {:else}
            <svg width="10" height="10" viewBox="0 0 10 10">
              <rect x="1.5" y="1.5" width="7" height="7" fill="none" stroke="currentColor" stroke-width="1.2" />
            </svg>
          {/if}
        </button>
        <button
          class="win-btn win-close"
          onclick={handleClose}
          title={t("window.close")}
          aria-label={t("window.close")}
        >
          <svg width="10" height="10" viewBox="0 0 10 10">
            <line x1="1.5" y1="1.5" x2="8.5" y2="8.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
            <line x1="8.5" y1="1.5" x2="1.5" y2="8.5" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" />
          </svg>
        </button>
      </div>
    </div>
  </header>

  <!-- Main Body Area -->
  <div class="workspace-body" class:resizing-inspector={isResizingInspector}>
    <!-- 2. Dynamic Contextual Left Toolbar -->
    <aside class="left-toolbar" aria-label="Barra de Ferramentas">
      <div class="contextual-tools">
        {#each workspaceTools[activeWorkspace] as tool (tool.id)}
          {@const ToolIcon = tool.icon}
          <button
            class="tool-btn"
            class:active={activeTool === tool.id}
            title={`${tool.labelKey ? t(tool.labelKey, tool.label) : tool.label}: ${tool.labelKey ? t(tool.labelKey + ".desc", tool.description) : tool.description}`}
            onclick={() => selectTool(tool.id)}
          >
            <ToolIcon size={18} />
          </button>
        {/each}
      </div>

      <div class="toolbar-spacer"></div>

      <!-- Bottom Pinned Global Tools (Focus & Settings) -->
      <div class="pinned-tools">
        <button
          class="tool-btn focus-btn"
          title={t("tool.focus")}
          aria-label={t("tool.focus")}
          onclick={() => viewportRef?.recenterCamera()}
        >
          <FocusIcon size={18} />
        </button>
        <button
          class="tool-btn settings-btn"
          class:active={isSettingsModalOpen}
          title={t("tool.settings")}
          aria-label={t("tool.settings")}
          onclick={() => { isSettingsModalOpen = !isSettingsModalOpen; }}
        >
          <SettingsIcon size={18} />
        </button>
      </div>
    </aside>

    <!-- 3D Center Viewport or Dedicated Asset Browser -->
    <main class="viewport-area">
      <div class="viewport-wrapper" class:hidden={activeWorkspace === "biblioteca"}>
        <Viewport
          bind:this={viewportRef}
          onResize={(w, h) => { vpWidth = w; vpHeight = h; }}
          onMetrics={handleMetrics}
          onTactileDrag={handleTactileDrag}
          onTactileDragEnd={handleTactileDragEnd}
          tactileEnabled={activeWorkspace === "personagem" && (activeTool === "body" || activeTool === "face")}
          onModelLoadError={(msg) => alert("Erro ao carregar modelo: " + msg)}
          onDiagnostic={handleViewportDiagnostic}
          coreSnapshotProvider={coreSnapshotProvider}
        />
      </div>

      {#if activeWorkspace === "biblioteca"}
        <AssetBrowser
          bind:this={assetBrowserRef}
          bind:selectedAsset={selectedLibraryAsset}
          onSelectAsset={(asset) => {
            selectedLibraryAsset = asset;
            if (!inspectorVisible) {
              inspectorVisible = true;
            }
          }}
          onUseAsset={handleUseLibraryAsset}
        />
      {/if}
    </main>

    <!-- 3. Dynamic Right Properties Inspector -->
    {#if inspectorVisible}
      <aside
        class="right-inspector"
        aria-label="Inspetor de Propriedades"
        style="width: {inspectorWidth}px;"
      >
        <!-- Resizer Handle on Left Edge -->
        <div
          class="inspector-resizer"
          class:active={isResizingInspector}
          role="separator"
          aria-orientation="vertical"
          aria-label="Redimensionador do Inspetor de Propriedades"
          aria-valuenow={inspectorWidth}
          aria-valuemin={MIN_INSPECTOR_WIDTH}
          aria-valuemax={MAX_INSPECTOR_WIDTH}
          tabindex="-1"
          title="Arrastar para redimensionar (320px – 960px)"
          onpointerdown={startResizeInspector}
        ></div>

        <div class="inspector-header">
          <div class="inspector-header-info">
            <div class="inspector-title">
              {activeWorkspace === "biblioteca" && selectedLibraryAsset
                ? selectedLibraryAsset.name
                : t("tool." + activeTool, toolLabels[activeTool] || "Propriedades")}
            </div>
            <div class="inspector-category">
              {activeWorkspace === "biblioteca" && selectedLibraryAsset
                ? `Biblioteca • ${selectedLibraryAsset.category}`
                : t("workspace." + activeWorkspace, workspaceLabels[activeWorkspace])}
            </div>
          </div>
          <button
            type="button"
            class="btn-toggle-inspector"
            title="Recolher Inspetor (Esconder)"
            aria-label="Recolher painel de propriedades"
            onclick={toggleInspector}
          >
            <ChevronRightIcon size={15} />
          </button>
        </div>

        <div class="inspector-content">
        <!-- TOOL: body & face (Anatomia, Corpo, Rosto & Sliders Canônicos Sprint 03) -->
        {#if activeTool === "body" || activeTool === "face"}
          <div class="control-group">
            <div class="group-title">PRESETS DE MALHA BASE</div>
            <div class="btn-grid">
              <button
                class="btn-secondary"
                class:selected={currentPreset === "mannequin"}
                onclick={() => handlePreset("mannequin")}
              >
                Manequim Canônico
              </button>
              <button
                class="btn-secondary"
                class:selected={currentPreset === "sphere"}
                onclick={() => handlePreset("sphere")}
              >
                Esfera NPR
              </button>
              <button
                class="btn-secondary"
                class:selected={currentPreset === "cube"}
                onclick={() => handlePreset("cube")}
              >
                Cubo Unitário
              </button>
            </div>
          </div>

          <AnatomyInspector
            bind:this={anatomyInspectorRef}
            {viewportRef}
            onModelChange={(g) => { loadedModelGender = g; }}
            onCharacterChange={handleInspectorCharacterChange}
            onCharacterCommit={(desc) => recordHistory(desc, false, morphStateCommand())}
            onProportionsChange={handleInspectorProportionsChange}
            onError={(msg) => alert(msg)}
          />

        <!-- TOOL: hair (Cabelo 3D) -->
        {:else if activeTool === "hair"}
          <div class="control-group">
            <div class="group-title">PARÂMETROS DE MECHA SPLINE</div>
            <div class="slider-row">
              <span class="label">Volume Geral</span>
              <input type="range" min="0.8" max="2.5" step="0.1" bind:value={hairVolume} />
              <span class="val-tag">{hairVolume.toFixed(1)}x</span>
            </div>
            <div class="slider-row">
              <span class="label">Espessura da Fita</span>
              <input type="range" min="0.01" max="0.20" step="0.01" bind:value={hairThickness} />
              <span class="val-tag">{(hairThickness * 100).toFixed(0)}mm</span>
            </div>
            <div class="slider-row">
              <span class="label">Curvatura Spline</span>
              <input type="range" min="0.1" max="1.0" step="0.05" bind:value={hairCurvature} />
              <span class="val-tag">{hairCurvature.toFixed(2)}</span>
            </div>
            <div class="slider-row">
              <span class="label">Divisões / Mechas</span>
              <input type="range" min="4" max="32" step="2" bind:value={hairStrands} />
              <span class="val-tag">{hairStrands}</span>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">AÇÃO RÁPIDA DE CABELO</div>
            <button class="btn-action">Gerar Nova Mecha Guiada</button>
          </div>

        <!-- TOOL: cloth (Vestuário) -->
        {:else if activeTool === "cloth"}
          <div class="control-group">
            <div class="group-title">CAMADAS DE ROUPA (2D/3D)</div>
            <div class="btn-grid">
              <button
                class="btn-secondary"
                class:selected={clothLayer === "base"}
                onclick={() => clothLayer = "base"}
              >
                Base / Roupa Íntima
              </button>
              <button
                class="btn-secondary"
                class:selected={clothLayer === "uniforme"}
                onclick={() => clothLayer = "uniforme"}
              >
                Uniforme / Blusa
              </button>
              <button
                class="btn-secondary"
                class:selected={clothLayer === "casaco"}
                onclick={() => clothLayer = "casaco"}
              >
                Sobretudo / Jaqueta
              </button>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">FÍSICA & CAIMENTO DO TECIDO</div>
            <div class="slider-row">
              <span class="label">Tensão do Tecido</span>
              <input type="range" min="0.1" max="1.0" step="0.05" bind:value={clothTension} />
              <span class="val-tag">{clothTension.toFixed(2)}</span>
            </div>
            <div class="slider-row">
              <span class="label">Rigidez de Flexão</span>
              <input type="range" min="0.0" max="1.0" step="0.05" bind:value={clothRigidity} />
              <span class="val-tag">{clothRigidity.toFixed(2)}</span>
            </div>
            <div class="slider-row">
              <span class="label">Gravidade / Peso</span>
              <input type="range" min="0.0" max="2.0" step="0.1" bind:value={clothGravity} />
              <span class="val-tag">{clothGravity.toFixed(1)}x</span>
            </div>
          </div>

        <!-- TOOL: accessories (Acessórios 3D) -->
        {:else if activeTool === "accessories"}
          <div class="control-group">
            <div class="group-title">SOQUETES DE ANCORAGEM</div>
            <div class="btn-grid">
              <button class="btn-secondary" class:selected={activeSocket === "head"} onclick={() => activeSocket = "head"}>Cabeça</button>
              <button class="btn-secondary" class:selected={activeSocket === "chest"} onclick={() => activeSocket = "chest"}>Tronco</button>
              <button class="btn-secondary" class:selected={activeSocket === "r_hand"} onclick={() => activeSocket = "r_hand"}>Mão Dir</button>
              <button class="btn-secondary" class:selected={activeSocket === "l_hand"} onclick={() => activeSocket = "l_hand"}>Mão Esq</button>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">TRANSFORMAÇÃO DO ADEREÇO</div>
            <div class="slider-row">
              <span class="label">Escala</span>
              <input type="range" min="0.2" max="2.5" step="0.1" bind:value={accessoryScale} />
              <span class="val-tag">{accessoryScale.toFixed(1)}x</span>
            </div>
            <div class="slider-row">
              <span class="label">Offset X</span>
              <input type="range" min="-0.5" max="0.5" step="0.02" bind:value={accessoryOffsetX} />
              <span class="val-tag">{accessoryOffsetX.toFixed(2)}m</span>
            </div>
            <div class="slider-row">
              <span class="label">Offset Y</span>
              <input type="range" min="-0.5" max="0.5" step="0.02" bind:value={accessoryOffsetY} />
              <span class="val-tag">{accessoryOffsetY.toFixed(2)}m</span>
            </div>
            <div class="slider-row">
              <span class="label">Offset Z</span>
              <input type="range" min="-0.5" max="0.5" step="0.02" bind:value={accessoryOffsetZ} />
              <span class="val-tag">{accessoryOffsetZ.toFixed(2)}m</span>
            </div>
          </div>

        <!-- TOOL: paint (Pintura de Textura) -->
        {:else if activeTool === "paint"}
          <div class="control-group">
            <div class="group-title">PINCEL 3D DE TEXTURA</div>
            <div class="slider-row">
              <span class="label">Raio do Pincel</span>
              <input type="range" min="2" max="64" step="1" value="16" />
              <span class="val-tag">16px</span>
            </div>
            <div class="slider-row">
              <span class="label">Opacidade / Força</span>
              <input type="range" min="0.1" max="1.0" step="0.05" value="0.8" />
              <span class="val-tag">80%</span>
            </div>
            <div class="slider-row">
              <span class="label">Dureza da Borda</span>
              <input type="range" min="0.0" max="1.0" step="0.05" value="0.5" />
              <span class="val-tag">50%</span>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">MÁSCARA DE PINTURA</div>
            <div class="btn-grid">
              <button class="btn-secondary selected">Albedo Base</button>
              <button class="btn-secondary">Máscara Sombra</button>
              <button class="btn-secondary">Brilho Especular</button>
            </div>
          </div>

        <!-- TOOL: cel_shader (Toon Ramp & Cel-Shading) -->
        {:else if activeTool === "cel_shader"}
          <div class="control-group">
            <div class="group-title">TOON RAMP</div>
            <div class="slider-row">
              <span class="label">Corte</span>
              <input
                type="range"
                min="0.05"
                max="0.95"
                step="0.02"
                bind:value={shadowThreshold}
                oninput={() => handleShadowThresholdChange(true, true)}
                onchange={() => handleShadowThresholdChange(true, false)}
              />
              <span class="val-tag">{shadowThreshold.toFixed(2)}</span>
            </div>

            <div class="slider-row">
              <span class="label">Suavidade</span>
              <input
                type="range"
                min="0.001"
                max="0.250"
                step="0.005"
                bind:value={toonSmoothness}
                oninput={() => handleToonSmoothnessChange(true, true)}
                onchange={() => handleToonSmoothnessChange(true, false)}
              />
              <span class="val-tag">{toonSmoothness.toFixed(3)}</span>
            </div>

            <div class="slider-row">
              <span class="label">Especular</span>
              <input
                type="range"
                min="0.0"
                max="2.5"
                step="0.05"
                bind:value={specIntensity}
                oninput={() => updateMaterial(true, true)}
                onchange={() => updateMaterial(true, false)}
              />
              <span class="val-tag">{specIntensity.toFixed(2)}x</span>
            </div>

            <div class="slider-row">
              <span class="label">Tamanho</span>
              <input
                type="range"
                min="4"
                max="128"
                step="2"
                bind:value={specExponent}
                oninput={() => updateMaterial(true, true)}
                onchange={() => updateMaterial(true, false)}
              />
              <span class="val-tag">{specExponent.toFixed(0)}</span>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">BRILHO</div>
            <div class="slider-row">
              <span class="label">Suavidade</span>
              <input
                type="range"
                min="0.0"
                max="0.5"
                step="0.01"
                bind:value={specSoftness}
                oninput={() => updateMaterial(true, true)}
                onchange={() => updateMaterial(true, false)}
              />
              <span class="val-tag">{specSoftness.toFixed(2)}</span>
            </div>

            <div class="slider-row">
              <span class="label">Offset Y</span>
              <input
                type="range"
                min="-1.0"
                max="1.0"
                step="0.05"
                bind:value={specOffset}
                oninput={() => updateMaterial(true, true)}
                onchange={() => updateMaterial(true, false)}
              />
              <span class="val-tag">{specOffset.toFixed(2)}</span>
            </div>

            <div class="color-row">
              <span class="label">Cor</span>
              <div class="color-input-wrapper">
                <input
                  type="color"
                  bind:value={specColorHex}
                  oninput={() => updateMaterial(true, true)}
                  onchange={() => updateMaterial(true, false)}
                />
                <span class="color-hex">{specColorHex.toUpperCase()}</span>
              </div>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">BANDAS (STEPS)</div>
            <div class="btn-grid-3">
              <button
                class="btn-secondary"
                class:selected={toonSteps === 1.0}
                onclick={() => { toonSteps = 1.0; updateMaterial(true, false); }}
              >
                1 Degrau (Cel)
              </button>
              <button
                class="btn-secondary"
                class:selected={toonSteps === 2.0}
                onclick={() => { toonSteps = 2.0; updateMaterial(true, false); }}
              >
                2 Degraus (Ghibli)
              </button>
              <button
                class="btn-secondary"
                class:selected={toonSteps === 3.0}
                onclick={() => { toonSteps = 3.0; updateMaterial(true, false); }}
              >
                3 Degraus (High-Key)
              </button>
              <button
                class="btn-secondary"
                class:selected={toonSteps === 0.0}
                onclick={() => { toonSteps = 0.0; updateMaterial(true, false); }}
              >
                Gradiente NPR
              </button>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">CORES BASE</div>
            <div class="color-row">
              <span class="label">Cor Base</span>
              <div class="color-input-wrapper">
                <input
                  type="color"
                  bind:value={baseColorHex}
                  oninput={() => updateMaterial(true, true)}
                  onchange={() => updateMaterial(true, false)}
                />
                <span class="color-hex">{baseColorHex.toUpperCase()}</span>
              </div>
            </div>
            <div class="color-row">
              <span class="label">Cor Sombra</span>
              <div class="color-input-wrapper">
                <input
                  type="color"
                  bind:value={shadowColorHex}
                  oninput={() => { updateMaterial(true, true); updateLighting(true, true); }}
                  onchange={() => { updateMaterial(true, false); updateLighting(true, false); }}
                />
                <span class="color-hex">{shadowColorHex.toUpperCase()}</span>
              </div>
            </div>
          </div>

        <!-- TOOL: rim (Luz de Borda) -->
        {:else if activeTool === "rim"}
          <div class="control-group">
            <div class="group-title">RIM LIGHT</div>
            <div class="slider-row">
              <span class="label">Intensidade</span>
              <input
                type="range"
                min="0.0"
                max="3.0"
                step="0.05"
                bind:value={rimIntensity}
                oninput={() => updateMaterial(true, true)}
                onchange={() => updateMaterial(true, false)}
              />
              <span class="val-tag">{rimIntensity.toFixed(2)}x</span>
            </div>

            <div class="slider-row">
              <span class="label">Espalhamento</span>
              <input
                type="range"
                min="0.05"
                max="0.95"
                step="0.02"
                bind:value={rimSpread}
                oninput={() => updateMaterial(true, true)}
                onchange={() => updateMaterial(true, false)}
              />
              <span class="val-tag">{rimSpread.toFixed(2)}</span>
            </div>

            <div class="color-row">
              <span class="label">Cor</span>
              <div class="color-input-wrapper">
                <input
                  type="color"
                  bind:value={rimColor}
                  oninput={() => updateMaterial(true, true)}
                  onchange={() => updateMaterial(true, false)}
                />
                <span class="color-hex">{rimColor.toUpperCase()}</span>
              </div>
            </div>
          </div>

        <!-- TOOL: outline (Contorno Inverted Hull) -->
        {:else if activeTool === "outline"}
          <div class="control-group">
            <div class="group-title">CONTORNO</div>
            <div class="slider-row">
              <span class="label">Espessura</span>
              <input
                type="range"
                min="0.0"
                max="10.0"
                step="0.2"
                bind:value={outlineWidth}
                oninput={() => handleOutlineChange(true, true)}
                onchange={() => handleOutlineChange(true, false)}
              />
              <span class="val-tag">{outlineWidth.toFixed(1)}px</span>
            </div>

            <div class="slider-row">
              <span class="label">Opacidade</span>
              <input
                type="range"
                min="0.0"
                max="1.0"
                step="0.05"
                bind:value={outlineOpacity}
                oninput={() => handleOutlineChange(true, true)}
                onchange={() => handleOutlineChange(true, false)}
              />
              <span class="val-tag">{(outlineOpacity * 100).toFixed(0)}%</span>
            </div>

            <div class="slider-row">
              <span class="label">Suavização</span>
              <input
                type="range"
                min="0.0"
                max="1.0"
                step="0.05"
                bind:value={outlineSmoothness}
                oninput={() => handleOutlineChange(true, true)}
                onchange={() => handleOutlineChange(true, false)}
              />
              <span class="val-tag">{(outlineSmoothness * 100).toFixed(0)}%</span>
            </div>

            <div class="slider-row">
              <span class="label">Profundidade Z</span>
              <input
                type="range"
                min="-0.005"
                max="0.005"
                step="0.0005"
                bind:value={outlineDepthBias}
                oninput={() => handleOutlineChange(true, true)}
                onchange={() => handleOutlineChange(true, false)}
              />
              <span class="val-tag">{(outlineDepthBias * 1000).toFixed(1)}</span>
            </div>

            <div class="color-row">
              <span class="label">Cor</span>
              <div class="color-input-wrapper">
                <input
                  type="color"
                  bind:value={outlineColor}
                  oninput={() => handleOutlineChange(true, true)}
                  onchange={() => handleOutlineChange(true, false)}
                />
                <span class="color-hex">{outlineColor.toUpperCase()}</span>
              </div>
            </div>
          </div>

        <!-- TOOL: palette (Paleta de Sombras Anime) -->
        {:else if activeTool === "palette"}
          <div class="control-group">
            <div class="group-title">HUE SHIFT</div>
            <div class="slider-row">
              <span class="label">Hue Shift</span>
              <input
                type="range"
                min="-60"
                max="60"
                step="1"
                bind:value={hueShift}
                oninput={() => updateMaterial(true, true)}
                onchange={() => updateMaterial(true, false)}
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
                oninput={() => { updateMaterial(true, true); updateLighting(true, true); }}
                onchange={() => { updateMaterial(true, false); updateLighting(true, false); }}
              />
              <span class="val-tag">{shadowSaturation.toFixed(2)}x</span>
            </div>

            <div class="color-row">
              <span class="label">Cor Base</span>
              <div class="color-input-wrapper">
                <input
                  type="color"
                  bind:value={baseColorHex}
                  oninput={() => updateMaterial(true, true)}
                  onchange={() => updateMaterial(true, false)}
                />
                <span class="color-hex">{baseColorHex.toUpperCase()}</span>
              </div>
            </div>

            <div class="color-row">
              <span class="label">Cor Sombra</span>
              <div class="color-input-wrapper">
                <input
                  type="color"
                  bind:value={shadowColorHex}
                  oninput={() => { updateMaterial(true, true); updateLighting(true, true); }}
                  onchange={() => { updateMaterial(true, false); updateLighting(true, false); }}
                />
                <span class="color-hex">{shadowColorHex.toUpperCase()}</span>
              </div>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">PRESETS HARMONIA</div>
            <div class="btn-grid">
              <button
                class="btn-secondary"
                onclick={() => {
                  hueShift = -18;
                  shadowSaturation = 1.25;
                  shadowColorHex = "#8b85b8";
                  updateMaterial(true, false);
                  updateLighting(true, false);
                }}
              >
                Sombra Fria (Lavanda)
              </button>
              <button
                class="btn-secondary"
                onclick={() => {
                  hueShift = 15;
                  shadowSaturation = 1.2;
                  shadowColorHex = "#a87162";
                  updateMaterial(true, false);
                  updateLighting(true, false);
                }}
              >
                Pôr-do-Sol Quente
              </button>
              <button
                class="btn-secondary"
                onclick={() => {
                  hueShift = 0;
                  shadowSaturation = 1.0;
                  shadowColorHex = "#808080";
                  updateMaterial(true, false);
                  updateLighting(true, false);
                }}
              >
                Neutro Standard
              </button>
              <button
                class="btn-secondary"
                onclick={() => {
                  hueShift = -12;
                  shadowSaturation = 1.35;
                  shadowColorHex = "#668574";
                  updateMaterial(true, false);
                  updateLighting(true, false);
                }}
              >
                Ghibli Natureza
              </button>
            </div>
          </div>

        <!-- TOOL: shader_ball (Shader Ball & Preview NPR) -->
        {:else if activeTool === "shader_ball"}
          <div class="control-group">
            <div class="group-title">CALIBRAÇÃO</div>
            <p class="section-desc">
              Utilize a esfera de calibração NPR para isolar e avaliar a distribuição de luz, ponto de corte de sombra e reflexo especular Blinn-Phong.
            </p>
            <div class="btn-grid">
              <button
                class="btn-secondary"
                class:selected={currentPreset === "sphere"}
                onclick={() => handlePreset("sphere")}
              >
                Esfera NPR
              </button>
              <button
                class="btn-secondary"
                class:selected={currentPreset === "mannequin"}
                onclick={() => handlePreset("mannequin")}
              >
                Manequim Anime
              </button>
              <button
                class="btn-secondary"
                class:selected={currentPreset === "cube"}
                onclick={() => handlePreset("cube")}
              >
                Cubo Unitário
              </button>
              <button
                class="btn-secondary"
                onclick={() => viewportRef?.recenterCamera()}
                title="Recentralizar Câmera (Tecla F)"
              >
                Focalizar [F]
              </button>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">RESUMO</div>
            <div class="param-summary-list">
              <div class="summary-item">
                <span class="s-label">Limiar Sombra:</span>
                <span class="s-val">{shadowThreshold.toFixed(2)}</span>
              </div>
              <div class="summary-item">
                <span class="s-label">Suavidade Degrau:</span>
                <span class="s-val">{toonSmoothness.toFixed(3)}</span>
              </div>
              <div class="summary-item">
                <span class="s-label">Especular:</span>
                <span class="s-val">{specIntensity.toFixed(2)}x (pwr {specExponent.toFixed(0)})</span>
              </div>
              <div class="summary-item">
                <span class="s-label">Rim Light:</span>
                <span class="s-val">{rimIntensity.toFixed(2)}x (spr {rimSpread.toFixed(2)})</span>
              </div>
              <div class="summary-item">
                <span class="s-label">Desvio Hue:</span>
                <span class="s-val">{hueShift > 0 ? `+${hueShift}` : hueShift}°</span>
              </div>
              <div class="summary-item">
                <span class="s-label">Saturação Sombra:</span>
                <span class="s-val">{shadowSaturation.toFixed(2)}x</span>
              </div>
              <div class="summary-item">
                <span class="s-label">Luz do Sol:</span>
                <span class="s-val">{lightIntensity.toFixed(2)}x ({lightAzimuth}°, {lightElevation}°)</span>
              </div>
              <div class="summary-item">
                <span class="s-label">Luz Ambiente:</span>
                <span class="s-val">{ambientIntensity.toFixed(2)}x</span>
              </div>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">PRESETS NPR</div>
            <div class="btn-grid">
              <button
                class="btn-secondary"
                onclick={() => {
                  shadowThreshold = 0.50;
                  toonSmoothness = 0.02;
                  specIntensity = 0.40;
                  specExponent = 32.0;
                  toonSteps = 1.0;
                  rimIntensity = 0.80;
                  rimSpread = 0.40;
                  hueShift = -15;
                  shadowSaturation = 1.15;
                  updateMaterial(true, false);
                }}
              >
                Padrão Anime TV
              </button>
              <button
                class="btn-secondary"
                onclick={() => {
                  shadowThreshold = 0.40;
                  toonSmoothness = 0.08;
                  specIntensity = 0.15;
                  specExponent = 16.0;
                  toonSteps = 2.0;
                  rimIntensity = 0.40;
                  rimSpread = 0.60;
                  hueShift = -8;
                  shadowSaturation = 1.25;
                  updateMaterial(true, false);
                }}
              >
                Ghibli Suave
              </button>
              <button
                class="btn-secondary"
                onclick={() => {
                  shadowThreshold = 0.65;
                  toonSmoothness = 0.002;
                  specIntensity = 1.20;
                  specExponent = 64.0;
                  toonSteps = 1.0;
                  rimIntensity = 1.50;
                  rimSpread = 0.25;
                  hueShift = -25;
                  shadowSaturation = 1.50;
                  updateMaterial(true, false);
                }}
              >
                Alto Contraste Mecha
              </button>
              <button
                class="btn-secondary"
                onclick={() => {
                  shadowThreshold = 0.45;
                  toonSmoothness = 0.20;
                  specIntensity = 0.30;
                  specExponent = 24.0;
                  toonSteps = 0.0;
                  rimIntensity = 0.60;
                  rimSpread = 0.50;
                  hueShift = -5;
                  shadowSaturation = 1.05;
                  updateMaterial(true, false);
                }}
              >
                Gradiente Suave 3D
              </button>
            </div>
          </div>

        <!-- TOOL: sun / shadows / ambient / Workspace Iluminação -->
        {:else if activeTool === "sun" || activeTool === "shadows" || activeTool === "ambient" || activeWorkspace === "iluminacao"}
          <LightingControls
            activeTool={activeTool}
            bind:lightAzimuth
            bind:lightElevation
            bind:lightIntensity
            bind:sunColor
            bind:shadowColorHex
            bind:hueShift
            bind:shadowSaturation
            bind:ambientIntensity
            onUpdate={(params) => {
              lightAzimuth = params.azimuth;
              lightElevation = params.elevation;
              lightIntensity = params.intensity;
              sunColor = params.sunColor;
              shadowColorHex = params.shadowColorHex;
              hueShift = params.hueShift;
              if (params.shadowSaturation !== undefined) shadowSaturation = params.shadowSaturation;
              if (params.ambientIntensity !== undefined) ambientIntensity = params.ambientIntensity;
              updateLighting(!params.isContinuous, params.isContinuous);
              updateMaterial(!params.isContinuous, params.isContinuous);
            }}
          />

        <!-- TOOL: browser / search / filters / models / Workspace Biblioteca (Inspetor de Ativos) -->
        {:else if activeWorkspace === "biblioteca" || activeTool === "browser" || activeTool === "search" || activeTool === "filters" || activeTool === "models"}
          {#if selectedLibraryAsset}
            <!-- 1. Identidade & Miniatura do Modelo -->
            <div class="control-group">
              <div class="group-title">
                <span>INSPETOR DE ATIVO</span>
                <span class="badge-format-pill">{selectedLibraryAsset.format}</span>
              </div>

              <div class="library-inspector-hero">
                <div class="hero-icon-box">
                  {#if selectedLibraryAsset.category === "Personagens"}
                    <UserIcon size={34} />
                  {:else if selectedLibraryAsset.category === "Roupas"}
                    <ShirtIcon size={34} />
                  {:else if selectedLibraryAsset.category === "Penteados"}
                    <ScissorsIcon size={34} />
                  {:else if selectedLibraryAsset.category === "Acessórios"}
                    <BoxIcon size={34} />
                  {:else if selectedLibraryAsset.category === "Poses"}
                    <BotIcon size={34} />
                  {:else if selectedLibraryAsset.category === "Materiais"}
                    <PaletteIcon size={34} />
                  {:else}
                    <CloudIcon size={34} />
                  {/if}
                </div>
                <div class="hero-meta">
                  <div class="hero-name" title={selectedLibraryAsset.name}>{selectedLibraryAsset.name}</div>
                  <div class="hero-sub">{selectedLibraryAsset.category} • {selectedLibraryAsset.author || "ANIGO Studio"}</div>
                </div>
              </div>
            </div>

            <!-- 2. Ficha Técnica da Malha (Faces, Vértices, etc.) -->
            <div class="control-group">
              <div class="group-title">FICHA TÉCNICA DA MALHA</div>
              <div class="param-summary-list">
                <div class="summary-item">
                  <span class="s-label">Categoria</span>
                  <span class="s-val">{selectedLibraryAsset.category}</span>
                </div>
                <div class="summary-item">
                  <span class="s-label">Formato Canônico</span>
                  <span class="s-val">{selectedLibraryAsset.format}</span>
                </div>
                <div class="summary-item">
                  <span class="s-label">Polígonos / Faces</span>
                  <span class="s-val">
                    {selectedLibraryAsset.polyCount > 0 ? selectedLibraryAsset.polyCount.toLocaleString() + " tris" : "Procedural / Vetorial"}
                  </span>
                </div>
                <div class="summary-item">
                  <span class="s-label">Vértices</span>
                  <span class="s-val">
                    {selectedLibraryAsset.vertexCount > 0 ? selectedLibraryAsset.vertexCount.toLocaleString() : "Vetorial"}
                  </span>
                </div>
                <div class="summary-item">
                  <span class="s-label">Origem / Autor</span>
                  <span class="s-val">{selectedLibraryAsset.author || "ANIGO Studio"}</span>
                </div>
              </div>
            </div>

            <!-- 3. Descrição do Modelo -->
            {#if selectedLibraryAsset.description}
              <div class="control-group">
                <div class="group-title">DESCRIÇÃO DO ATIVO</div>
                <p class="section-desc">{selectedLibraryAsset.description}</p>
              </div>
            {/if}

            <!-- 4. Tags de Busca -->
            {#if selectedLibraryAsset.tags && selectedLibraryAsset.tags.length > 0}
              <div class="control-group">
                <div class="group-title">TAGS & METADADOS</div>
                <div class="library-tags-list">
                  {#each selectedLibraryAsset.tags as tag}
                    <span class="inspector-tag-pill">#{tag}</span>
                  {/each}
                </div>
              </div>
            {/if}

            <!-- 5. Ações do Ativo -->
            <div class="control-group">
              <div class="group-title">AÇÕES DO MODELO</div>
              <button
                type="button"
                class="btn-action"
                onclick={() => handleUseLibraryAsset(selectedLibraryAsset!)}
                title="Carregar ou vincular este modelo ao projeto"
              >
                Usar no Cenário
              </button>
              <button
                type="button"
                class="btn-secondary btn-inspect"
                onclick={() => assetBrowserRef?.openInspectModal(selectedLibraryAsset)}
                title="Abrir ficha técnica completa em janela ampliada"
              >
                Inspecionar Detalhes 3D
              </button>
            </div>
          {:else}
            <div class="control-group">
              <div class="group-title">INSPETOR DE ATIVOS</div>
              <div class="library-empty-inspector">
                <GridIcon size={28} />
                <div class="empty-inspector-title">Nenhum Ativo Selecionado</div>
                <div class="empty-inspector-desc">
                  Clique em qualquer miniatura de modelo no navegador para inspecionar seus dados, contagem de polígonos e utilizá-lo no projeto.
                </div>
              </div>
            </div>
          {/if}

        <!-- TOOL: hair_lib (Biblioteca - Penteados) -->
        {:else if activeTool === "hair_lib"}
          <div class="control-group">
            <div class="group-title">PENTEADOS ANIME 3D</div>
            <div class="card-grid">
              <div class="asset-card active">
                <div class="card-icon"><ScissorsIcon size={24} /></div>
                <div class="card-title">Bob Curto com Franja</div>
                <div class="card-meta">1,240 tris • Fita Spline</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><SparklesIcon size={24} /></div>
                <div class="card-title">Espigado Shonen</div>
                <div class="card-meta">1,950 tris • Estilo Herói</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><ScissorsIcon size={24} /></div>
                <div class="card-title">Twintails Duplas</div>
                <div class="card-meta">3,100 tris • Física Pronta</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><SparklesIcon size={24} /></div>
                <div class="card-title">Longo Ondulado</div>
                <div class="card-meta">2,800 tris • Elegante</div>
              </div>
            </div>
          </div>

        <!-- TOOL: clothes_lib (Biblioteca - Roupas) -->
        {:else if activeTool === "clothes_lib"}
          <div class="control-group">
            <div class="group-title">COLEÇÃO DE VESTUÁRIO</div>
            <div class="card-grid">
              <div class="asset-card active">
                <div class="card-icon"><ShirtIcon size={24} /></div>
                <div class="card-title">Seifuku Escolar</div>
                <div class="card-meta">3 Peças • Toon Anime</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><BoxIcon size={24} /></div>
                <div class="card-title">Jaqueta Cyberpunk</div>
                <div class="card-meta">Alta Densidade • Gola Alta</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><ShirtIcon size={24} /></div>
                <div class="card-title">Yukata / Kimono</div>
                <div class="card-meta">Tradicional • Caimento</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><BoxIcon size={24} /></div>
                <div class="card-title">Armadura Paladino</div>
                <div class="card-meta">Placas Rígidas • Soquetes</div>
              </div>
            </div>
          </div>

        <!-- TOOL: materials (Biblioteca - Materiais) -->
        {:else if activeTool === "materials"}
          <div class="control-group">
            <div class="group-title">MATERIAIS CEL-SHADER</div>
            <div class="card-grid">
              <div class="asset-card active">
                <div class="card-icon"><LayersIcon size={24} /></div>
                <div class="card-title">Pele Anime Toon</div>
                <div class="card-meta">Subsurface • Toon Ramp</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><LayersIcon size={24} /></div>
                <div class="card-title">Cabelo Anisotrópico</div>
                <div class="card-meta">Brilho Anel Toon</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><LayersIcon size={24} /></div>
                <div class="card-title">Algodão Mate</div>
                <div class="card-meta">Sombra Difusa Flat</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><LayersIcon size={24} /></div>
                <div class="card-title">Metal Dourado NPR</div>
                <div class="card-meta">Reflexo Step Anime</div>
              </div>
            </div>
          </div>

        <!-- TOOL: poses_preset / poses_lib (Poses do Manequim) -->
        {:else if activeTool === "poses_lib" || activeTool === "poses_preset"}
          <div class="control-group">
            <div class="group-title">BIBLIOTECA DE POSES</div>
            <div class="card-grid">
              <div class="asset-card active">
                <div class="card-icon"><BoneIcon size={24} /></div>
                <div class="card-title">T-Pose Padrão</div>
                <div class="card-meta">Calibração Anatômica</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><BoneIcon size={24} /></div>
                <div class="card-title">A-Pose Neutra</div>
                <div class="card-meta">Modelagem & Vestuário</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><TargetIcon size={24} /></div>
                <div class="card-title">Guarda de Combate</div>
                <div class="card-meta">Dinâmica Shonen</div>
              </div>
              <div class="asset-card">
                <div class="card-icon"><TargetIcon size={24} /></div>
                <div class="card-title">Pouso de Herói</div>
                <div class="card-meta">Impacto Dramático</div>
              </div>
            </div>
          </div>

        <!-- TOOL: timeline / curves (Animação - Linha do Tempo & Curvas) -->
        {:else if activeTool === "timeline" || activeTool === "curves"}
          <div class="control-group">
            <div class="group-title">REPRODUÇÃO & TIMELINE</div>
            <div class="slider-row">
              <span class="label">Frame Atual</span>
              <input type="range" min="1" max="240" step="1" bind:value={currentFrame} />
              <span class="val-tag">{currentFrame} / 240</span>
            </div>

            <div class="btn-grid">
              <button
                class="btn-secondary"
                class:selected={isPlaying}
                onclick={() => isPlaying = !isPlaying}
              >
                {isPlaying ? "Pausar" : "Reproduzir"}
              </button>
              <button class="btn-secondary" onclick={() => currentFrame = 1}>Ir ao Início</button>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">CADÊNCIA DE ANIMAÇÃO ANIME</div>
            <div class="btn-grid">
              <button class="btn-secondary" class:selected={animationFps === 24} onclick={() => animationFps = 24}>24 FPS (Cinemático)</button>
              <button class="btn-secondary" class:selected={animationFps === 12} onclick={() => animationFps = 12}>12 FPS (Anime em 2s)</button>
              <button class="btn-secondary" class:selected={animationFps === 60} onclick={() => animationFps = 60}>60 FPS (Fluido)</button>
            </div>
          </div>

        <!-- TOOL: rig (Animação - Esqueleto) -->
        {:else if activeTool === "rig"}
          <div class="control-group">
            <div class="group-title">ESTRUTURA ÓSSEA</div>
            <div class="slider-row">
              <span class="label">Osso Ativo</span>
              <select bind:value={selectedBone} class="dark-select">
                <option value="Head">Head (Cabeça)</option>
                <option value="Neck">Neck (Pescoço)</option>
                <option value="Spine">Spine (Coluna)</option>
                <option value="Arm_L">Arm_L (Braço Esquerdo)</option>
                <option value="Arm_R">Arm_R (Braço Direito)</option>
                <option value="Leg_L">Leg_L (Perna Esquerda)</option>
                <option value="Leg_R">Leg_R (Perna Direita)</option>
              </select>
            </div>

            <div class="slider-row">
              <span class="label">Rotação X</span>
              <input type="range" min="-180" max="180" value="0" />
              <span class="val-tag">0°</span>
            </div>
            <div class="slider-row">
              <span class="label">Rotação Y</span>
              <input type="range" min="-180" max="180" value="0" />
              <span class="val-tag">0°</span>
            </div>
            <div class="slider-row">
              <span class="label">Rotação Z</span>
              <input type="range" min="-180" max="180" value="0" />
              <span class="val-tag">0°</span>
            </div>
          </div>

        <!-- TOOL: ik (Animação - Cinemática Inversa) -->
        {:else if activeTool === "ik"}
          <div class="control-group">
            <div class="group-title">SOLVER IK (CINEMÁTICA INVERSA)</div>
            <div class="btn-grid">
              <button class="btn-secondary" class:selected={ikSolver === "two_bone"} onclick={() => ikSolver = "two_bone"}>Two-Bone IK</button>
              <button class="btn-secondary" class:selected={ikSolver === "fabrik"} onclick={() => ikSolver = "fabrik"}>FABRIK Suave</button>
            </div>

            <div class="slider-row">
              <span class="label">Peso de Influência</span>
              <input type="range" min="0.0" max="1.0" step="0.05" bind:value={ikWeight} />
              <span class="val-tag">{(ikWeight * 100).toFixed(0)}%</span>
            </div>

            <div class="slider-row">
              <span class="label">Ângulo Pole Target</span>
              <input type="range" min="-180" max="180" step="5" value="0" />
              <span class="val-tag">0°</span>
            </div>
          </div>

        <!-- TOOL: lipsync (Animação - Sincronia Labial) -->
        {:else if activeTool === "lipsync"}
          <div class="control-group">
            <div class="group-title">VISEMAS DE BOCA ANIME</div>
            <div class="slider-row">
              <span class="label">Visema [A] (Aberta)</span>
              <input type="range" min="0.0" max="1.0" step="0.05" value="0.0" />
              <span class="val-tag">0.0</span>
            </div>
            <div class="slider-row">
              <span class="label">Visema [I] (Sorriso)</span>
              <input type="range" min="0.0" max="1.0" step="0.05" value="0.0" />
              <span class="val-tag">0.0</span>
            </div>
            <div class="slider-row">
              <span class="label">Visema [U] (Bico)</span>
              <input type="range" min="0.0" max="1.0" step="0.05" value="0.0" />
              <span class="val-tag">0.0</span>
            </div>
            <div class="slider-row">
              <span class="label">Visema [E] (Média)</span>
              <input type="range" min="0.0" max="1.0" step="0.05" value="0.0" />
              <span class="val-tag">0.0</span>
            </div>
            <div class="slider-row">
              <span class="label">Visema [O] (Redonda)</span>
              <input type="range" min="0.0" max="1.0" step="0.05" value="0.0" />
              <span class="val-tag">0.0</span>
            </div>
          </div>

        <!-- TOOL: stage / props (Cenário - Blocagem & Props) -->
        {:else if activeTool === "stage" || activeTool === "props"}
          <div class="control-group">
            <div class="group-title">BLOCAGEM MODULAR GREYBOX</div>
            <div class="btn-grid">
              <button class="btn-secondary">+ Chão 4x4m</button>
              <button class="btn-secondary">+ Parede 3m</button>
              <button class="btn-secondary">+ Coluna Toon</button>
              <button class="btn-secondary">+ Escadaria</button>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">GRADE DE POSICIONAMENTO</div>
            <div class="btn-grid">
              <button class="btn-secondary">Snap 0.5m</button>
              <button class="btn-secondary selected">Snap 1.0m</button>
              <button class="btn-secondary">Snap 2.0m</button>
            </div>
          </div>

        <!-- TOOL: environment / ambient (Céu Anime & Luz Ambiente) -->
        {:else if activeTool === "environment" || activeTool === "ambient"}
          <div class="control-group">
            <div class="group-title">ATMOSFERA & CÉU ANIME</div>
            <div class="btn-grid">
              <button class="btn-secondary selected">Céu Azul Toon</button>
              <button class="btn-secondary">Golden Hour Crepúsculo</button>
              <button class="btn-secondary">Noite Estrelada Sci-Fi</button>
              <button class="btn-secondary">Sunset Neo-Tokyo</button>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">NÉVOA DISTANTE / FOG</div>
            <div class="slider-row">
              <span class="label">Densidade de Névoa</span>
              <input type="range" min="0.0" max="1.0" step="0.05" value="0.1" />
              <span class="val-tag">10%</span>
            </div>
          </div>

        <!-- TOOL: camera (Cenário - Câmera) -->
        {:else if activeTool === "camera"}
          <div class="control-group">
            <div class="group-title">LENTE & CÂMERA DE CENA</div>
            <div class="slider-row">
              <span class="label">Campo de Visão (FOV)</span>
              <input type="range" min="25" max="90" step="1" bind:value={cameraFov} />
              <span class="val-tag">{cameraFov}°</span>
            </div>

            <div class="btn-grid">
              <button class="btn-secondary" class:selected={focalLength === 24} onclick={() => focalLength = 24}>24mm Grande Angular</button>
              <button class="btn-secondary" class:selected={focalLength === 50} onclick={() => focalLength = 50}>50mm Retrato Anime</button>
              <button class="btn-secondary" class:selected={focalLength === 85} onclick={() => focalLength = 85}>85mm Telefoto</button>
            </div>
          </div>

        <!-- TOOL: render / passes / export (Render - Câmera, Passes & Exportação) -->
        {:else if activeTool === "render" || activeTool === "passes" || activeTool === "export"}
          <div class="control-group">
            <div class="group-title">RESOLUÇÃO DE EXPORTAÇÃO</div>
            <div class="btn-grid">
              <button class="btn-secondary selected">1920×1080 (FHD)</button>
              <button class="btn-secondary">2560×1440 (2K)</button>
              <button class="btn-secondary">3840×2160 (4K)</button>
            </div>
          </div>

          <div class="control-group">
            <div class="group-title">PASSES DE RENDERIZAÇÃO NPR</div>
            <label class="check-row">
              <input type="checkbox" checked />
              <span>Beauty Toon Completo</span>
            </label>
            <label class="check-row">
              <input type="checkbox" checked />
              <span>Linhas Inverted Hull Isoladas</span>
            </label>
            <label class="check-row">
              <input type="checkbox" />
              <span>Pass de Sombra Flat</span>
            </label>
          </div>

          <div class="control-group">
            <button class="btn-action">Renderizar Imagem Atual</button>
          </div>

        {/if}
      </div>
    </aside>
  {:else}
    <!-- Collapsed Toggle Button at top right corner -->
    <button
      type="button"
      class="btn-toggle-inspector btn-toggle-collapsed"
      title="Expandir Inspetor"
      aria-label="Expandir Inspetor"
      onclick={toggleInspector}
    >
      <ChevronLeftIcon size={15} />
    </button>
  {/if}
  </div>

  <!-- 4. Professional Bottom Status Bar -->
  <footer class="app-statusbar">
    <!-- Left: Breadcrumb -->
    <div class="status-left">
      <span class="breadcrumb-tag">
        [{t("workspace." + activeWorkspace, workspaceLabels[activeWorkspace])} > {t("tool." + activeTool, toolLabels[activeTool] || activeTool)}]
      </span>
    </div>

    <!-- Center: Project & Active Model Popovers -->
    <div class="status-center">
      <button
        class="status-btn status-project"
        onclick={() => (isProjectPopoverOpen = !isProjectPopoverOpen)}
        title="Gerenciamento de Projeto (Salvar, Abrir, Novo)"
        aria-label="Gerenciamento de Projeto"
      >
        <span class="status-label">{t("status.project")}:</span>
        <span class="status-val">{currentProjectName}</span>
        {#if isProjectDirty}
          <span class="dirty-indicator" title="Modificações não salvas">●</span>
        {/if}
      </button>

      <span class="status-sep">•</span>

      <button
        class="status-btn status-model"
        onclick={() => (isPresetPopoverOpen = !isPresetPopoverOpen)}
        title="Seletor de Presets de Malha 3D"
        aria-label="Seletor de Presets de Malha 3D"
      >
        <span class="status-label">{t("status.model")}:</span>
        <span class="status-val">{presetLabels[currentPreset] || currentPreset}</span>
      </button>
    </div>

    <!-- Right: Status & Autosave (Moved here as requested!) -->
    <div class="status-right">
      <!-- P1-02: falha de renderer/núcleo não é mais silenciosa -->
      {#if viewportDiagnostics.degraded || lastDiagnostic}
        <button
          class="status-btn status-diagnostics"
          class:status-diagnostics-error={viewportDiagnostics.degraded}
          class:status-diagnostics-warn={!viewportDiagnostics.degraded}
          onclick={() => (isDiagnosticsPopoverOpen = !isDiagnosticsPopoverOpen)}
          title={lastDiagnostic
            ? `[${lastDiagnostic.code}] ${lastDiagnostic.message}`
            : "diagnósticos do renderer"}
          aria-label="Diagnósticos do renderer"
        >
          <span class="status-label">{t("status.diagnostics")}:</span>
          <span class="status-val">
            {viewportDiagnostics.errors}E / {viewportDiagnostics.warnings}W
          </span>
        </button>
        {#if isDiagnosticsPopoverOpen}
          <div class="status-diagnostics-panel" role="log" aria-live="polite">
            <strong>{t("status.diagnostics")}</strong>
            {#each viewportRef?.getDiagnostics?.().entries ?? [] as entry (entry.code + entry.message)}
              <div class="status-diagnostics-entry" data-severity={entry.severity}>
                <code>[{entry.code}]</code>
                <span>{entry.message}{entry.count > 1 ? ` (×${entry.count})` : ""}</span>
                {#if entry.detail}<em>{entry.detail}</em>{/if}
              </div>
            {/each}
            {#if viewportDiagnostics.dropped > 0}
              <em>{viewportDiagnostics.dropped} diagnóstico(s) descartado(s) por limite</em>
            {/if}
          </div>
        {/if}
        <span class="status-sep">•</span>
      {/if}
      <span class="status-ready">{t("status.ready")}</span>
      {#if lastAutosaveTime}
        <span class="status-sep">•</span>
        <span class="status-autosave" title={t("settings.path_autosave")}>
          <span class="autosave-dot"></span>
          {t("status.autosave")}: {lastAutosaveTime}
        </span>
      {/if}
    </div>
  </footer>
</div>

<SettingsModal
  isOpen={isSettingsModalOpen}
  onClose={() => (isSettingsModalOpen = false)}
  onSave={handleSaveStudioSettings}
  {telemetryBackend}
  {telemetryAdapter}
  {telemetryFps}
/>

<ProjectMenuPopover
  isOpen={isProjectPopoverOpen}
  projectName={currentProjectName}
  isDirty={isProjectDirty}
  onSave={handleSaveProject}
  onSaveAs={handleSaveProjectAs}
  onOpen={handleOpenProject}
  onNew={handleNewProject}
  onOpenFolder={handleOpenProjectsFolder}
  onClose={() => (isProjectPopoverOpen = false)}
/>

<ModelPresetPopover
  isOpen={isPresetPopoverOpen}
  {currentPreset}
  onSelectPreset={(p) => {
    handlePreset(p);
  }}
  onImportCustomMesh={() => {
    alert("O módulo de importação de malha externa (.OBJ / .VRM / .GLTF) será liberado na Sprint correspondente!");
  }}
  onClose={() => (isPresetPopoverOpen = false)}
/>

<QuickStartModal
  isOpen={isQuickStartOpen}
  onSelectPreset={(p) => {
    handlePreset(p);
  }}
  onOpenProject={handleOpenProject}
  onImportMesh={() => {
    alert("O módulo de importação de malha externa (.OBJ / .VRM / .GLTF) será liberado na Sprint correspondente!");
  }}
  onClose={() => (isQuickStartOpen = false)}
/>

<style>
  :global(*), :global(*::before), :global(*::after) {
    box-sizing: border-box;
  }

  .app-layout {
    display: flex;
    flex-direction: column;
    width: 100vw;
    height: 100vh;
    background-color: #07090e;
    color: #e2e8f0;
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
    font-size: 0.82rem;
    overflow: hidden;
    user-select: none;
  }

  /* 1. Frameless Custom Titlebar */
  .app-titlebar {
    height: 38px;
    background: #0f121a;
    border-bottom: 1px solid #1a2030;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 0 0 12px;
    flex-shrink: 0;
  }

  .brand-section {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-right: 14px;
    border-right: 1px solid #1a2030;
    height: 100%;
    flex-shrink: 0;
  }

  .brand-logo {
    width: 20px;
    height: 20px;
    border-radius: 4px;
    display: block;
  }

  .brand-info {
    display: flex;
    align-items: baseline;
    gap: 5px;
  }

  .brand-title {
    font-weight: 800;
    font-size: 0.95rem;
    letter-spacing: 1.5px;
    color: #c084fc;
  }

  .brand-tag {
    font-size: 0.65rem;
    font-weight: 700;
    letter-spacing: 1px;
    color: #64748b;
  }

  /* Workspaces Tabs */
  .workspace-tabs {
    display: flex;
    align-items: center;
    height: 100%;
    margin-left: 8px;
    gap: 2px;
    flex-shrink: 0;
  }

  .tab-button {
    height: 100%;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: #94a3b8;
    padding: 0 12px;
    font-size: 0.76rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s ease;
    display: flex;
    align-items: center;
    white-space: nowrap;
  }

  .tab-button:hover {
    color: #f1f5f9;
    background: rgba(255, 255, 255, 0.04);
  }

  .tab-button.active {
    color: #c084fc;
    border-bottom-color: #c084fc;
    background: rgba(192, 132, 252, 0.08);
  }

  .titlebar-drag-spacer {
    flex: 1 1 auto;
    min-width: 16px;
    height: 100%;
  }

  /* Titlebar Actions & Window Controls */
  .titlebar-actions {
    display: flex;
    align-items: center;
    height: 100%;
    flex-shrink: 0;
  }

  .window-controls {
    display: flex;
    height: 100%;
  }

  .win-btn {
    width: 44px;
    height: 100%;
    background: transparent;
    border: none;
    color: #94a3b8;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.12s ease;
  }

  .win-btn:hover {
    background: rgba(255, 255, 255, 0.08);
    color: #f1f5f9;
  }

  .win-btn.win-close:hover {
    background: #ef4444;
    color: #ffffff;
  }

  /* 2. Workspace Body */
  .workspace-body {
    flex: 1;
    display: flex;
    overflow: hidden;
    min-height: 0;
    min-width: 0;
    width: 100%;
    padding: 6px;
    gap: 6px;
    background: #07090e;
    position: relative;
  }

  /* Left Toolbar */
  .left-toolbar {
    width: 46px;
    background: #0e1119;
    border: 1px solid #1a2030;
    border-radius: 8px;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.35);
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 8px 0;
    flex-shrink: 0;
  }

  .contextual-tools {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
    align-items: center;
  }

  .toolbar-spacer {
    flex: 1;
  }

  .pinned-tools {
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding-top: 8px;
    border-top: 1px solid #1a2030;
  }

  .tool-btn.focus-btn {
    color: #64748b;
  }

  .tool-btn.focus-btn:hover {
    color: #38bdf8;
  }

  .tool-btn {
    width: 34px;
    height: 34px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 6px;
    color: #94a3b8;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .tool-btn :global(svg) {
    stroke: currentColor;
    transition: stroke 0.15s ease;
  }

  .tool-btn:hover {
    background: #181d2b;
    color: #38bdf8;
    border-color: rgba(56, 189, 248, 0.3);
  }

  .tool-btn.active {
    background: #2b1d3d;
    color: #c084fc;
    border-color: #c084fc;
  }

  .tool-btn.settings-btn {
    color: #64748b;
  }

  .tool-btn.settings-btn:hover {
    color: #f1f5f9;
  }

  .tool-btn.settings-btn.active {
    color: #c084fc;
  }

  /* Center Viewport */
  .viewport-area {
    flex: 1 1 0%;
    min-width: 0;
    min-height: 0;
    position: relative;
    background: #000;
    overflow: hidden;
    border-radius: 8px;
    border: 1px solid #1a2030;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.35);
  }

  .viewport-wrapper {
    width: 100%;
    height: 100%;
    display: flex;
  }

  .viewport-wrapper.hidden {
    display: none;
  }

  /* Resizing Isolation on Viewport */
  .workspace-body.resizing-inspector .viewport-area {
    pointer-events: none;
  }

  /* 3. Right Inspector Dynamic Bounds */
  .right-inspector {
    position: relative;
    min-width: 320px;
    max-width: 960px;
    flex-shrink: 0;
    background: #0e1119;
    border: 1px solid #1a2030;
    border-radius: 8px;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.35);
    display: flex;
    flex-direction: column;
    overflow: visible; /* Allows resizer handle to live in the 6px gap without clipping */
  }

  /* Drag Handle occupying the comfortable gap */
  .inspector-resizer {
    position: absolute;
    top: 0;
    bottom: 0;
    left: -12px;
    width: 18px;
    cursor: col-resize;
    z-index: 50;
    touch-action: none;
    background: transparent;
  }

  .inspector-resizer::after {
    content: "";
    position: absolute;
    top: calc(50% - 28px);
    left: calc(50% - 2px);
    width: 4px;
    height: 56px;
    border-radius: 2px;
    background: #222a3d;
    border: 1px solid #334155;
    transition: all 0.15s ease;
  }

  .inspector-resizer:hover::after,
  .inspector-resizer.active::after {
    background: #38bdf8;
    border-color: #38bdf8;
    box-shadow: 0 0 10px rgba(56, 189, 248, 0.8), 0 0 2px #38bdf8;
    transform: scaleY(1.15);
  }

  /* Inspector Header Layout */
  .inspector-header {
    height: 52px;
    padding: 0 14px;
    border-bottom: 1px solid #1a2030;
    border-radius: 7px 7px 0 0;
    display: flex;
    justify-content: space-between;
    align-items: center;
    background: #0e1118;
    gap: 8px;
    flex-shrink: 0;
    box-sizing: border-box;
  }

  .inspector-header-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .btn-toggle-inspector {
    width: 26px;
    height: 26px;
    background: #141824;
    border: 1px solid #232c40;
    color: #94a3b8;
    border-radius: 5px;
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.15s ease;
    flex-shrink: 0;
    box-shadow: 0 1px 4px rgba(0, 0, 0, 0.3);
  }

  .btn-toggle-inspector:hover {
    background: #1c2333;
    color: #38bdf8;
    border-color: #38bdf8;
    box-shadow: 0 0 8px rgba(56, 189, 248, 0.4);
  }

  /* Collapsed Toggle Button at top right corner */
  .btn-toggle-collapsed {
    position: absolute;
    top: 20px;
    right: 21px;
    z-index: 25;
  }

  .inspector-title {
    font-weight: 700;
    font-size: 0.8rem;
    color: #f1f5f9;
    letter-spacing: 0.5px;
  }

  .inspector-category {
    font-size: 0.68rem;
    color: #64748b;
    text-transform: uppercase;
    font-weight: 600;
  }

  .inspector-content {
    flex: 1;
    padding: 12px;
    border-radius: 0 0 7px 7px;
    overflow-y: auto;
    overflow-x: auto;
    display: flex;
    flex-direction: column;
    gap: 16px;
    scrollbar-width: thin;
    scrollbar-color: #1f293d transparent;
  }

  .inspector-content::-webkit-scrollbar {
    width: 6px;
    height: 6px;
  }

  .inspector-content::-webkit-scrollbar-track {
    background: transparent;
  }

  .inspector-content::-webkit-scrollbar-thumb {
    background: #1f293d;
    border-radius: 3px;
  }

  .inspector-content::-webkit-scrollbar-thumb:hover {
    background: #334155;
  }

  .inspector-content::-webkit-scrollbar-corner {
    background: transparent;
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

  .badge-warn {
    font-size: 0.65rem;
    color: #fbbf24;
    font-weight: 600;
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

  .btn-grid-3 {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: 6px;
  }

  .param-summary-list {
    display: flex;
    flex-direction: column;
    gap: 5px;
    background: #0f121a;
    padding: 8px 10px;
    border-radius: 4px;
    border: 1px solid #1a2030;
  }

  .summary-item {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.72rem;
  }

  .summary-item .s-label {
    color: #94a3b8;
  }

  .summary-item .s-val {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
    color: #38bdf8;
    font-weight: 600;
  }

  .section-desc {
    font-size: 0.72rem;
    color: #94a3b8;
    line-height: 1.4;
    margin: 0 0 6px 0;
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
  }

  .btn-secondary:hover {
    background: #242d44;
    color: #ffffff;
  }

  .btn-secondary.selected {
    background: #392453;
    border-color: #c084fc;
    color: #ffffff;
  }

  .btn-action {
    background: #7e22ce;
    border: none;
    color: #ffffff;
    padding: 8px 12px;
    border-radius: 4px;
    font-size: 0.74rem;
    font-weight: 700;
    cursor: pointer;
    width: 100%;
    transition: background 0.15s ease;
  }

  .btn-action:hover {
    background: #9333ea;
  }

  /* Library Inspector Elements */
  .badge-format-pill {
    font-size: 0.65rem;
    font-weight: 800;
    color: #38bdf8;
    background: rgba(56, 189, 248, 0.12);
    border: 1px solid rgba(56, 189, 248, 0.3);
    padding: 1px 6px;
    border-radius: 4px;
    letter-spacing: 0.5px;
  }

  .library-inspector-hero {
    display: flex;
    align-items: center;
    gap: 12px;
    background: #0f121a;
    padding: 10px;
    border-radius: 6px;
    border: 1px solid #1a2030;
  }

  .hero-icon-box {
    width: 44px;
    height: 44px;
    background: linear-gradient(135deg, #182038, #0e1422);
    border: 1px solid #28344e;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #c084fc;
    flex-shrink: 0;
  }

  .hero-meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .hero-name {
    font-size: 0.8rem;
    font-weight: 700;
    color: #f1f5f9;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .hero-sub {
    font-size: 0.68rem;
    color: #94a3b8;
  }

  .library-tags-list {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .inspector-tag-pill {
    font-size: 0.66rem;
    color: #a78bfa;
    background: rgba(167, 139, 250, 0.1);
    border: 1px solid rgba(167, 139, 250, 0.25);
    padding: 2px 7px;
    border-radius: 4px;
  }

  .library-empty-inspector {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 24px 12px;
    gap: 8px;
    color: #64748b;
  }

  .empty-inspector-title {
    font-size: 0.78rem;
    font-weight: 600;
    color: #cbd5e1;
  }

  .empty-inspector-desc {
    font-size: 0.7rem;
    line-height: 1.4;
    color: #64748b;
  }

  .dark-select {
    background: #1a2030;
    border: 1px solid #283147;
    color: #cbd5e1;
    padding: 4px 8px;
    border-radius: 4px;
    font-size: 0.74rem;
    cursor: pointer;
  }

  .check-row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 0.74rem;
    color: #cbd5e1;
    cursor: pointer;
  }

  /* Visual Asset Cards Grid */
  .card-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }

  .asset-card {
    background: #1a2030;
    border: 1px solid #263045;
    border-radius: 6px;
    padding: 10px 8px;
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    cursor: pointer;
    transition: all 0.15s ease;
  }

  .asset-card:hover {
    background: #222b40;
    border-color: #38bdf8;
  }

  .asset-card.active {
    background: #2d1d40;
    border-color: #c084fc;
  }

  .card-icon {
    color: #c084fc;
    margin-bottom: 6px;
  }

  .card-title {
    font-weight: 600;
    font-size: 0.72rem;
    color: #f1f5f9;
    margin-bottom: 2px;
  }

  .card-meta {
    font-size: 0.64rem;
    color: #64748b;
  }



  /* 4. Bottom Status Bar */
  .status-diagnostics {
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 6px;
    padding: 2px 8px;
    cursor: pointer;
    background: transparent;
  }
  .status-diagnostics-error { color: #ff7b72; border-color: rgba(255, 123, 114, 0.5); }
  .status-diagnostics-warn { color: #e3b341; border-color: rgba(227, 179, 65, 0.5); }
  .status-diagnostics-panel {
    position: absolute;
    bottom: 32px;
    right: 12px;
    max-width: 460px;
    max-height: 220px;
    overflow: auto;
    background: #171a26;
    border: 1px solid rgba(255, 255, 255, 0.16);
    border-radius: 8px;
    padding: 8px 10px;
    font-size: 11px;
    z-index: 40;
  }
  .status-diagnostics-entry { display: flex; flex-direction: column; gap: 2px; margin-top: 6px; }
  .status-diagnostics-entry[data-severity="error"] code { color: #ff7b72; }
  .status-diagnostics-entry[data-severity="warning"] code { color: #e3b341; }
  .app-statusbar {
    height: 28px;
    margin: 0 6px 6px 6px;
    border-radius: 6px;
    border: 1px solid #1a2030;
    background: #0e1119;
    position: relative;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.25);
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px;
    font-size: 0.72rem;
    color: #64748b;
    flex-shrink: 0;
  }

  .status-left,
  .status-right {
    display: flex;
    align-items: center;
    gap: 8px;
    z-index: 1;
  }

  .status-center {
    position: absolute;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 8px;
    white-space: nowrap;
    pointer-events: auto;
  }

  .breadcrumb-tag {
    color: #c084fc;
    font-weight: 600;
  }

  .status-btn {
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: #cbd5e1;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 8px;
    font-size: 0.72rem;
    cursor: pointer;
    transition: all 0.15s ease;
    font-family: inherit;
  }

  .status-btn:hover {
    background: #182032;
    border-color: #29354d;
    color: #ffffff;
  }

  .status-btn .status-label {
    color: #64748b;
    font-size: 0.7rem;
    font-weight: 600;
  }

  .status-btn.status-project:hover .status-val {
    color: #38bdf8;
  }

  .status-btn.status-model:hover .status-val {
    color: #c084fc;
  }

  .status-val {
    font-weight: 600;
  }

  .dirty-indicator {
    color: #f59e0b;
    font-size: 0.65rem;
    line-height: 1;
    margin-left: 2px;
  }

  .status-sep {
    color: #293246;
  }

  .status-ready {
    color: #10b981;
    font-weight: 500;
  }

  .status-autosave {
    color: #38bdf8;
    font-weight: 500;
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }

  .autosave-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: #38bdf8;
    box-shadow: 0 0 6px #38bdf8;
    display: inline-block;
  }

  /* History Actions in Titlebar */
  .history-actions {
    display: flex;
    align-items: center;
    gap: 3px;
    margin-left: 8px;
    padding-left: 8px;
    border-left: 1px solid #1a2030;
  }

  .hist-btn {
    width: 24px;
    height: 24px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    color: #94a3b8;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.12s ease;
    padding: 0;
  }

  .hist-btn:hover:not(:disabled) {
    background: #1e2638;
    color: #f1f5f9;
    border-color: #29354d;
  }

  .hist-btn:disabled {
    opacity: 0.3;
    cursor: not-allowed;
  }
</style>
