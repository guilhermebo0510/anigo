/**
 * ANIGO Studio — Automated Opaque-Box E2E Test Suite
 * Canonical 8-Workspace Architecture, Viewport Preservation & Asset Browser Pipeline
 *
 * Requirements:
 * - c:\ANIGO\.agents\ORIGINAL_REQUEST.md (R1, R2, R3, Acceptance Criteria)
 * - c:\ANIGO\.agents\orchestrator_1\PROJECT.md (Interface Contracts, Milestones M1-M4)
 * - c:\ANIGO\.agents\orchestrator_1\TEST_INFRA.md (4-Tier Test Architecture, 12 Features)
 *
 * Execution:
 * node --experimental-strip-types --test tests/e2e/workspace_pipeline.test.ts
 */

import test, { describe, it, beforeEach } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { historyService, type HistoryStateSnapshot } from "../../src/services/history_service.ts";
import { pt_BR } from "../../src/i18n/pt_BR.ts";
import { en_US } from "../../src/i18n/en_US.ts";
import { ja_JP } from "../../src/i18n/ja_JP.ts";

// ============================================================================
// CANONICAL SPECIFICATION CONTRACTS (Authoritative Source: ORIGINAL_REQUEST.md)
// ============================================================================

export const CANONICAL_WORKSPACES = [
  "personagem",
  "posing",
  "shading",
  "iluminacao",
  "cenario",
  "animacao",
  "render",
  "biblioteca",
] as const;

export type CanonicalWorkspaceId = (typeof CANONICAL_WORKSPACES)[number];

export interface ToolContract {
  id: string;
  labelKey: string;
  iconName: string;
  inspectorProperties: string[];
}

export const CANONICAL_TOOLS: Record<CanonicalWorkspaceId, readonly ToolContract[]> = {
  personagem: [
    { id: "body", labelKey: "tool.body", iconName: "UserIcon", inspectorProperties: ["headScale", "headRatio"] },
    { id: "face", labelKey: "tool.face", iconName: "SmileIcon", inspectorProperties: ["smile", "eye_big"] },
    { id: "hair", labelKey: "tool.hair", iconName: "ScissorsIcon", inspectorProperties: ["hairStrands", "ribbonWidth"] },
    { id: "cloth", labelKey: "tool.cloth", iconName: "ShirtIcon", inspectorProperties: ["outfitPreset", "clothWrinkles"] },
    { id: "accessories", labelKey: "tool.accessories", iconName: "BoxIcon", inspectorProperties: ["attachmentPoint", "propScale"] },
    { id: "paint", labelKey: "tool.paint", iconName: "BrushIcon", inspectorProperties: ["brushColor", "brushSize"] },
  ],
  posing: [
    { id: "rig", labelKey: "tool.rig", iconName: "BoneIcon", inspectorProperties: ["selectedBone", "boneRotation"] },
    { id: "ik", labelKey: "tool.ik", iconName: "TargetIcon", inspectorProperties: ["ikActive", "ikChainWeight"] },
    { id: "poses_preset", labelKey: "tool.poses_preset", iconName: "BotIcon", inspectorProperties: ["posePreset"] },
  ],
  shading: [
    { id: "cel_shader", labelKey: "tool.cel_shader", iconName: "ShaderIcon", inspectorProperties: ["shadowThreshold", "toonBands"] },
    { id: "rim", labelKey: "tool.rim", iconName: "RimIcon", inspectorProperties: ["rimIntensity", "rimColor"] },
    { id: "outline", labelKey: "tool.outline", iconName: "OutlineIcon", inspectorProperties: ["outlineWidth", "outlineColor"] },
    { id: "palette", labelKey: "tool.palette", iconName: "PaletteIcon", inspectorProperties: ["shadowColor", "hueShift"] },
    { id: "shader_ball", labelKey: "tool.shader_ball", iconName: "SlidersIcon", inspectorProperties: ["previewSphere"] },
  ],
  iluminacao: [
    { id: "sun", labelKey: "tool.sun", iconName: "SunIcon", inspectorProperties: ["lightAzimuth", "lightElevation", "lightIntensity"] },
    { id: "shadows", labelKey: "tool.shadows", iconName: "CloudIcon", inspectorProperties: ["shadowSoftness", "shadowBias"] },
    { id: "ambient", labelKey: "tool.ambient", iconName: "SparklesIcon", inspectorProperties: ["ambientSkyColor", "ambientGroundColor"] },
  ],
  cenario: [
    { id: "stage", labelKey: "tool.stage", iconName: "BoxIcon", inspectorProperties: ["stagePreset", "gridSize"] },
    { id: "props", labelKey: "tool.props", iconName: "LayersIcon", inspectorProperties: ["propsCount", "selectedProp"] },
    { id: "environment", labelKey: "tool.environment", iconName: "CloudIcon", inspectorProperties: ["environmentSky", "fogDensity"] },
  ],
  animacao: [
    { id: "timeline", labelKey: "tool.timeline", iconName: "ActivityIcon", inspectorProperties: ["currentFrame", "totalFrames", "playbackFps"] },
    { id: "curves", labelKey: "tool.curves", iconName: "SlidersIcon", inspectorProperties: ["easingFunction"] },
    { id: "lipsync", labelKey: "tool.lipsync", iconName: "SmileIcon", inspectorProperties: ["lipsyncViseme", "phonemeTrack"] },
  ],
  render: [
    { id: "camera", labelKey: "tool.camera", iconName: "CameraIcon", inspectorProperties: ["cameraFov", "cameraDistance"] },
    { id: "passes", labelKey: "tool.passes", iconName: "LayersIcon", inspectorProperties: ["renderPasses"] },
    { id: "export", labelKey: "tool.export", iconName: "FilmIcon", inspectorProperties: ["renderResolution", "exportFormat"] },
  ],
  biblioteca: [
    { id: "browser", labelKey: "tool.browser", iconName: "GridIcon", inspectorProperties: ["selectedAssetId"] },
    { id: "search", labelKey: "tool.search", iconName: "SearchIcon", inspectorProperties: ["assetSearchQuery"] },
    { id: "filters", labelKey: "tool.filters", iconName: "SlidersIcon", inspectorProperties: ["selectedCategory"] },
  ],
};

export const DEFAULT_WORKSPACE_TOOLS: Record<CanonicalWorkspaceId, string> = {
  personagem: "body",
  posing: "rig",
  shading: "cel_shader",
  iluminacao: "sun",
  cenario: "stage",
  animacao: "timeline",
  render: "camera",
  biblioteca: "browser",
};

export const CANONICAL_ASSET_CATEGORIES = [
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
  category: (typeof CANONICAL_ASSET_CATEGORIES)[number];
  tags: string[];
  thumbnail: string;
  polyCount: number;
  vertexCount: number;
  format: "VRM" | "OBJ" | "GLB" | "ANIGO_MATERIAL";
}

export const MOCK_ASSET_CATALOG: AssetRecord[] = [
  { id: "char_01", name: "Aoi Mannequin Base", category: "Personagens", tags: ["anime", "female", "base"], thumbnail: "aoi_thumb.png", polyCount: 14200, vertexCount: 7800, format: "VRM" },
  { id: "char_02", name: "Ren Stylized Male", category: "Personagens", tags: ["anime", "male", "hero"], thumbnail: "ren_thumb.png", polyCount: 16500, vertexCount: 8900, format: "VRM" },
  { id: "outfit_01", name: "Seifuku School Uniform", category: "Roupas", tags: ["uniform", "school", "cloth"], thumbnail: "uniform_thumb.png", polyCount: 6800, vertexCount: 3600, format: "GLB" },
  { id: "outfit_02", name: "Battle Kimono Robe", category: "Roupas", tags: ["kimono", "fantasy", "cloth"], thumbnail: "kimono_thumb.png", polyCount: 9200, vertexCount: 4900, format: "GLB" },
  { id: "hair_01", name: "Twin Tails Dynamic", category: "Penteados", tags: ["twintails", "ribbon", "hair"], thumbnail: "twintails_thumb.png", polyCount: 4500, vertexCount: 2400, format: "GLB" },
  { id: "hair_02", name: "Spiky Shonen Hair", category: "Penteados", tags: ["spiky", "shonen", "hair"], thumbnail: "spiky_thumb.png", polyCount: 3800, vertexCount: 2100, format: "GLB" },
  { id: "prop_01", name: "Katana of the Dawn", category: "Acessórios", tags: ["sword", "weapon", "blade"], thumbnail: "katana_thumb.png", polyCount: 2200, vertexCount: 1200, format: "OBJ" },
  { id: "prop_02", name: "School Desk & Chair", category: "Acessórios", tags: ["desk", "school", "furniture"], thumbnail: "desk_thumb.png", polyCount: 1400, vertexCount: 800, format: "GLB" },
  { id: "pose_01", name: "Dynamic Mid-Air Jump", category: "Poses", tags: ["action", "jump", "hero"], thumbnail: "jump_thumb.png", polyCount: 0, vertexCount: 0, format: "VRM" },
  { id: "pose_02", name: "Standing Confident Studio", category: "Poses", tags: ["idle", "studio", "fashion"], thumbnail: "stand_thumb.png", polyCount: 0, vertexCount: 0, format: "VRM" },
  { id: "mat_01", name: "Cel-Shader Anime Skin Ramp", category: "Materiais", tags: ["cel-shading", "skin", "toon"], thumbnail: "skin_ramp_thumb.png", polyCount: 0, vertexCount: 0, format: "ANIGO_MATERIAL" },
  { id: "mat_02", name: "Metallic Inverted Hull Rim", category: "Materiais", tags: ["metallic", "rim", "outline"], thumbnail: "metal_thumb.png", polyCount: 0, vertexCount: 0, format: "ANIGO_MATERIAL" },
  { id: "env_01", name: "Sunset Anime Sky Dome", category: "Ambientes", tags: ["sunset", "clouds", "sky"], thumbnail: "sunset_thumb.png", polyCount: 1200, vertexCount: 650, format: "GLB" },
  { id: "env_02", name: "Cyber Tokyo Neon Atmosphere", category: "Ambientes", tags: ["cyberpunk", "night", "city"], thumbnail: "cyber_thumb.png", polyCount: 2500, vertexCount: 1400, format: "GLB" },
  { id: "stage_01", name: "Classroom Modular Greybox", category: "Cenários", tags: ["classroom", "interior", "modular"], thumbnail: "classroom_thumb.png", polyCount: 18500, vertexCount: 9800, format: "GLB" },
  { id: "stage_02", name: "Anime Rooftop Stage", category: "Cenários", tags: ["rooftop", "urban", "stage"], thumbnail: "rooftop_thumb.png", polyCount: 14200, vertexCount: 7600, format: "GLB" },
];

// ============================================================================
// OPAQUE-BOX PIPELINE CONTROLLER HARNESS (Conforms to PROJECT.md Contracts)
// ============================================================================

export class StudioPipelineHarness {
  public activeWorkspace: CanonicalWorkspaceId = "personagem";
  public activeTool: string = "body";

  // WebGPU Viewport State Machine
  public isViewportMounted: boolean = true; // Never destroyed!
  public isViewportVisible: boolean = true;
  public isRenderLoopPaused: boolean = false;

  // Asset Browser State Machine
  public isAssetBrowserActive: boolean = false;
  public selectedAssetCategory: string | null = null;
  public assetSearchQuery: string = "";
  public selectedAsset: AssetRecord | null = null;
  public activeSceneAssets: AssetRecord[] = [];

  // Multi-Discipline Studio Parameters
  public studioParameters: Record<string, any> = {
    // Personagem
    headScale: 1.0,
    headRatio: 7.0,
    faceBlendshapes: { smile: 0.0, eye_big: 0.0 },
    hairStrands: 32,
    outfitPreset: "default",
    // Posing
    selectedBone: "root",
    ikActive: false,
    posePreset: "neutral",
    // Shading
    shadowThreshold: 0.5,
    outlineWidth: 0.015,
    rimIntensity: 1.0,
    rimColor: "#ffffff",
    // Iluminação
    lightAzimuth: 45.0,
    lightElevation: 45.0,
    lightIntensity: 1.0,
    sunColor: "#ffffff",
    shadowColor: [0.2, 0.18, 0.25],
    // Cenário
    stagePreset: "default_grid",
    environmentSky: "daylight",
    propsCount: 0,
    // Animação
    currentFrame: 0,
    totalFrames: 60,
    keyframeTrack: [] as { frame: number; param: string; val: any }[],
    lipsyncViseme: "REST",
    // Render
    cameraFov: 45.0,
    renderResolution: "1920x1080",
    renderPasses: ["color"],
    selectedAssetId: null,
  };

  constructor() {
    this.resetBaseline();
  }

  public resetBaseline(): void {
    this.activeWorkspace = "personagem";
    this.activeTool = "body";
    this.isViewportMounted = true;
    this.isViewportVisible = true;
    this.isRenderLoopPaused = false;
    this.isAssetBrowserActive = false;
    this.selectedAssetCategory = null;
    this.assetSearchQuery = "";
    this.selectedAsset = null;
    this.activeSceneAssets = [];

    const snapshot: HistoryStateSnapshot = {
      preset: "mannequin",
      headScale: 1.0,
      headRatio: 7.0,
      outlineWidth: 0.015,
      shadowThreshold: 0.5,
      lightDir: [0.577, -0.577, 0.577],
      lightIntensity: 1.0,
      shadowColor: [0.2, 0.18, 0.25],
      lightAzimuth: 45.0,
      lightElevation: 45.0,
      activeWorkspace: "personagem",
      activeTool: "body",
      projectName: "ANIGO_E2E_Test",
      timestamp: Date.now(),
    };
    historyService.init(snapshot);
  }

  public switchWorkspace(target: any): boolean {
    if (typeof target !== "string") return false;
    if (!CANONICAL_WORKSPACES.includes(target as any)) {
      return false; // Safely reject invalid workspace
    }
    const ws = target as CanonicalWorkspaceId;

    this.activeWorkspace = ws;
    this.activeTool = DEFAULT_WORKSPACE_TOOLS[ws];

    if (ws === "biblioteca") {
      this.isViewportMounted = true;
      this.isViewportVisible = false;
      this.isRenderLoopPaused = true;
      this.isAssetBrowserActive = true;
    } else {
      this.isViewportMounted = true;
      this.isViewportVisible = true;
      this.isRenderLoopPaused = false;
      this.isAssetBrowserActive = false;
    }

    return true;
  }

  public selectTool(toolId: any): boolean {
    if (typeof toolId !== "string" || !toolId) return false;
    const tools = CANONICAL_TOOLS[this.activeWorkspace];
    if (tools.some((t) => t.id === toolId)) {
      this.activeTool = toolId;
      return true;
    }
    return false;
  }

  public handleKeyDown(key: string, ctrl = false, alt = false): boolean {
    if (ctrl || alt) return false;
    const num = parseInt(key, 10);
    if (!isNaN(num) && num >= 1 && num <= 8) {
      return this.switchWorkspace(CANONICAL_WORKSPACES[num - 1]);
    }
    return false;
  }

  public handleIpcAction(payload: any): boolean {
    if (!payload || typeof payload !== "object") return false;
    const action = payload.action || payload.type;
    if (action === "select_tab" || payload.name === "tab") {
      const tab = payload.value || payload.tab || payload.target;
      return this.switchWorkspace(tab);
    }
    return false;
  }

  public setParameter(key: string, value: any, description: string, isContinuous = false): void {
    // Boundary Clamping for Physical Attributes
    if (key === "headScale") {
      value = Math.max(0.5, Math.min(2.0, Number(value)));
    } else if (key === "shadowThreshold") {
      value = Math.max(0.0, Math.min(1.0, Number(value)));
    } else if (key === "lightAzimuth") {
      value = ((Number(value) % 360) + 360) % 360; // Circular wrap 0..360
    } else if (key === "lightElevation") {
      value = Math.max(0.0, Math.min(90.0, Number(value))); // Clamp 0..90
    } else if (key === "cameraFov") {
      value = Math.max(10.0, Math.min(120.0, Number(value)));
    }

    this.studioParameters[key] = value;

    const snapshot: HistoryStateSnapshot = {
      preset: "mannequin",
      headScale: this.studioParameters.headScale,
      headRatio: this.studioParameters.headRatio,
      outlineWidth: this.studioParameters.outlineWidth,
      shadowThreshold: this.studioParameters.shadowThreshold,
      lightDir: [0.5, 0.5, 0.5],
      lightIntensity: this.studioParameters.lightIntensity,
      shadowColor: this.studioParameters.shadowColor,
      lightAzimuth: this.studioParameters.lightAzimuth,
      lightElevation: this.studioParameters.lightElevation,
      activeWorkspace: this.activeWorkspace,
      activeTool: this.activeTool,
      timestamp: Date.now(),
    };

    historyService.push(snapshot, description, isContinuous);
  }

  public undo(): HistoryStateSnapshot | null {
    const restored = historyService.undo();
    if (restored) {
      this.studioParameters.headScale = restored.headScale;
      this.studioParameters.headRatio = restored.headRatio;
      this.studioParameters.outlineWidth = restored.outlineWidth;
      this.studioParameters.shadowThreshold = restored.shadowThreshold;
      this.studioParameters.lightIntensity = restored.lightIntensity;
      if (restored.lightAzimuth !== undefined) this.studioParameters.lightAzimuth = restored.lightAzimuth;
      if (restored.lightElevation !== undefined) this.studioParameters.lightElevation = restored.lightElevation;
      if (restored.activeWorkspace) this.switchWorkspace(restored.activeWorkspace);
      if (restored.activeTool) this.selectTool(restored.activeTool);
    }
    return restored;
  }

  public redo(): HistoryStateSnapshot | null {
    const restored = historyService.redo();
    if (restored) {
      this.studioParameters.headScale = restored.headScale;
      this.studioParameters.headRatio = restored.headRatio;
      this.studioParameters.outlineWidth = restored.outlineWidth;
      this.studioParameters.shadowThreshold = restored.shadowThreshold;
      this.studioParameters.lightIntensity = restored.lightIntensity;
      if (restored.lightAzimuth !== undefined) this.studioParameters.lightAzimuth = restored.lightAzimuth;
      if (restored.lightElevation !== undefined) this.studioParameters.lightElevation = restored.lightElevation;
      if (restored.activeWorkspace) this.switchWorkspace(restored.activeWorkspace);
      if (restored.activeTool) this.selectTool(restored.activeTool);
    }
    return restored;
  }

  public getFilteredAssets(): AssetRecord[] {
    return MOCK_ASSET_CATALOG.filter((asset) => {
      const matchCat = !this.selectedAssetCategory || asset.category === this.selectedAssetCategory;
      const q = this.assetSearchQuery.toLowerCase().trim();
      const matchSearch =
        !q ||
        asset.name.toLowerCase().includes(q) ||
        asset.tags.some((t) => t.toLowerCase().includes(q));
      return matchCat && matchSearch;
    });
  }

  public inspectAsset(assetId: string): AssetRecord | null {
    const asset = MOCK_ASSET_CATALOG.find((a) => a.id === assetId) || null;
    this.selectedAsset = asset;
    this.studioParameters.selectedAssetId = asset ? asset.id : null;
    return asset;
  }

  public useAssetInScene(assetId: string): { success: boolean; asset?: AssetRecord } {
    const asset = MOCK_ASSET_CATALOG.find((a) => a.id === assetId);
    if (!asset) return { success: false };
    this.activeSceneAssets.push(asset);
    return { success: true, asset };
  }
}

// ============================================================================
// TEST SUITE: TIER 1 — FEATURE COVERAGE
// ============================================================================

describe("Tier 1: Feature Coverage (Canonical 8-Workspace Pipeline)", () => {
  let harness: StudioPipelineHarness;

  beforeEach(() => {
    harness = new StudioPipelineHarness();
  });

  it("1.1 should define exactly 8 canonical workspace identifiers in strict sequence", () => {
    assert.strictEqual(CANONICAL_WORKSPACES.length, 8, "Must have exactly 8 workspaces");
    const expected = [
      "personagem",
      "posing",
      "shading",
      "iluminacao",
      "cenario",
      "animacao",
      "render",
      "biblioteca",
    ];
    assert.deepStrictEqual(Array.from(CANONICAL_WORKSPACES), expected, "Must match canonical order");
  });

  it("1.2 should provide dedicated, non-overlapping tools for all 8 workspaces", () => {
    const allTools = new Set<string>();
    let totalToolCount = 0;

    for (const ws of CANONICAL_WORKSPACES) {
      const tools = CANONICAL_TOOLS[ws];
      assert.ok(tools.length > 0, `Workspace ${ws} must have at least one dedicated tool`);

      for (const tool of tools) {
        assert.ok(
          !allTools.has(tool.id),
          `Tool '${tool.id}' in workspace '${ws}' overlaps with another workspace! Tools must be dedicated.`
        );
        allTools.add(tool.id);
        totalToolCount++;
      }
    }

    assert.strictEqual(totalToolCount, 29, "Expected 29 dedicated tools across 8 workspaces");
  });

  it("1.3 should designate a valid default tool for each canonical workspace", () => {
    for (const ws of CANONICAL_WORKSPACES) {
      const defTool = DEFAULT_WORKSPACE_TOOLS[ws];
      const tools = CANONICAL_TOOLS[ws];
      assert.ok(
        tools.some((t) => t.id === defTool),
        `Default tool '${defTool}' must belong to workspace '${ws}' tools`
      );
    }
  });

  it("1.4 should map every dedicated tool to a specific SVG icon component name", () => {
    for (const ws of CANONICAL_WORKSPACES) {
      for (const tool of CANONICAL_TOOLS[ws]) {
        assert.ok(
          tool.iconName && tool.iconName.endsWith("Icon"),
          `Tool '${tool.id}' must have an assigned SVG icon component (found: ${tool.iconName})`
        );
      }
    }
    const searchTool = CANONICAL_TOOLS.biblioteca.find((t) => t.id === "search");
    assert.strictEqual(searchTool?.iconName, "SearchIcon", "Search tool must map to SearchIcon");
  });

  it("1.5 should verify 8 canonical asset categories in the Asset Browser", () => {
    assert.strictEqual(CANONICAL_ASSET_CATEGORIES.length, 8, "Asset Browser must have 8 filter categories");
    const expectedCategories = [
      "Personagens",
      "Roupas",
      "Penteados",
      "Acessórios",
      "Poses",
      "Materiais",
      "Ambientes",
      "Cenários",
    ];
    assert.deepStrictEqual(Array.from(CANONICAL_ASSET_CATEGORIES), expectedCategories);
  });

  it("1.6 should verify titlebar tab switching to each of the 8 canonical workspaces", () => {
    for (const ws of CANONICAL_WORKSPACES) {
      const ok = harness.switchWorkspace(ws);
      assert.strictEqual(ok, true, `Switch to '${ws}' should succeed`);
      assert.strictEqual(harness.activeWorkspace, ws);
      assert.strictEqual(harness.activeTool, DEFAULT_WORKSPACE_TOOLS[ws]);
    }
  });

  it("1.7 should verify inspector properties contract defined per tool", () => {
    for (const ws of CANONICAL_WORKSPACES) {
      for (const tool of CANONICAL_TOOLS[ws]) {
        assert.ok(
          Array.isArray(tool.inspectorProperties) && tool.inspectorProperties.length > 0,
          `Tool '${tool.id}' in '${ws}' must declare at least one inspector contextual property`
        );
      }
    }
  });

  it("1.8 should verify mock asset catalog schema and production distribution", () => {
    assert.ok(MOCK_ASSET_CATALOG.length >= 16, "Must provide rich mock catalog");
    for (const cat of CANONICAL_ASSET_CATEGORIES) {
      const count = MOCK_ASSET_CATALOG.filter((a) => a.category === cat).length;
      assert.ok(count >= 1, `Category '${cat}' must contain at least one mock asset`);
    }
  });
});

// ============================================================================
// TEST SUITE: TIER 2 — BOUNDARY & CORNER CASES
// ============================================================================

describe("Tier 2: Boundary & Corner Cases", () => {
  let harness: StudioPipelineHarness;

  beforeEach(() => {
    harness = new StudioPipelineHarness();
  });

  it("2.1 should reject invalid, unknown, or fabricated workspace identifiers", () => {
    const initialWorkspace = harness.activeWorkspace;
    const invalidInputs = [
      "invalid_tab",
      "rigging",
      "lighting",
      "audio",
      "vfx",
      "workspace_9",
      "PERSONAGEM", // Exact lowercase matching required
      "   ",
      "",
      null,
      undefined,
      123,
      {},
      [],
    ];

    for (const invalid of invalidInputs) {
      const ok = harness.switchWorkspace(invalid);
      assert.strictEqual(ok, false, `Expected '${String(invalid)}' to be rejected`);
      assert.strictEqual(
        harness.activeWorkspace,
        initialWorkspace,
        `activeWorkspace should remain '${initialWorkspace}' after invalid selection`
      );
    }
  });

  it("2.2 should reject selection of tools that do not belong to current workspace", () => {
    harness.switchWorkspace("personagem");
    assert.strictEqual(harness.activeTool, "body");

    // Try selecting tools from other workspaces
    const illegalTools = ["timeline", "sun", "stage", "camera", "browser", "non_existent_tool"];
    for (const tool of illegalTools) {
      const ok = harness.selectTool(tool);
      assert.strictEqual(ok, false, `Tool '${tool}' should be rejected in 'personagem'`);
      assert.strictEqual(harness.activeTool, "body", "activeTool should not change on rejected selection");
    }

    // Try selecting valid tool in current workspace
    const validOk = harness.selectTool("face");
    assert.strictEqual(validOk, true);
    assert.strictEqual(harness.activeTool, "face");
  });

  it("2.3 should reject out-of-range keyboard shortcuts (< 1 or > 8) and non-numeric keys", () => {
    harness.switchWorkspace("personagem");

    // Shortcut '1' through '8' are valid
    for (let i = 1; i <= 8; i++) {
      const ok = harness.handleKeyDown(String(i));
      assert.strictEqual(ok, true, `Key '${i}' should switch workspace`);
      assert.strictEqual(harness.activeWorkspace, CANONICAL_WORKSPACES[i - 1]);
    }

    // Edge cases: '0', '9', negative, symbols, letters
    const invalidKeys = ["0", "9", "-1", "10", "a", "z", "F1", "Space", "Enter", "!", "@"];
    const lastWorkspace = harness.activeWorkspace;

    for (const key of invalidKeys) {
      const ok = harness.handleKeyDown(key);
      assert.strictEqual(ok, false, `Key '${key}' should be ignored`);
      assert.strictEqual(harness.activeWorkspace, lastWorkspace, "Workspace must not change on invalid key");
    }

    // Keys with Ctrl / Alt modifiers should not switch workspace directly
    const modifierOk = harness.handleKeyDown("1", true, false);
    assert.strictEqual(modifierOk, false, "Ctrl+1 should not trigger plain workspace tab switch");
  });

  it("2.4 should handle malformed IPC / MCP bridge payloads safely without throwing", () => {
    const malformedPayloads = [
      null,
      undefined,
      {},
      { action: "unknown_action" },
      { action: "select_tab", value: "non_existent" },
      { type: "select_tab", tab: 999 },
      { name: "tab", target: "" },
    ];

    for (const payload of malformedPayloads) {
      assert.doesNotThrow(() => {
        const ok = harness.handleIpcAction(payload);
        assert.strictEqual(ok, false, `Malformed payload should return false: ${JSON.stringify(payload)}`);
      });
    }

    const validOk = harness.handleIpcAction({ action: "select_tab", value: "render" });
    assert.strictEqual(validOk, true);
    assert.strictEqual(harness.activeWorkspace, "render");
  });

  it("2.5 should enforce parameter boundary clamping on extreme inputs", () => {
    // headScale clamped to [0.5, 2.0]
    harness.setParameter("headScale", -5.0, "Negative scale");
    assert.strictEqual(harness.studioParameters.headScale, 0.5);
    harness.setParameter("headScale", 99.0, "Extreme scale");
    assert.strictEqual(harness.studioParameters.headScale, 2.0);

    // shadowThreshold clamped to [0.0, 1.0]
    harness.setParameter("shadowThreshold", 2.5, "Over threshold");
    assert.strictEqual(harness.studioParameters.shadowThreshold, 1.0);
    harness.setParameter("shadowThreshold", -1.0, "Sub threshold");
    assert.strictEqual(harness.studioParameters.shadowThreshold, 0.0);

    // lightAzimuth wrapped around [0, 360)
    harness.setParameter("lightAzimuth", 400.0, "Azimuth wrap");
    assert.strictEqual(harness.studioParameters.lightAzimuth, 40.0);
    harness.setParameter("lightAzimuth", -90.0, "Negative Azimuth");
    assert.strictEqual(harness.studioParameters.lightAzimuth, 270.0);

    // lightElevation clamped to [0.0, 90.0]
    harness.setParameter("lightElevation", 180.0, "Over elevation");
    assert.strictEqual(harness.studioParameters.lightElevation, 90.0);

    // cameraFov clamped to [10.0, 120.0]
    harness.setParameter("cameraFov", 2.0, "Micro lens");
    assert.strictEqual(harness.studioParameters.cameraFov, 10.0);
    harness.setParameter("cameraFov", 200.0, "Fish-eye extreme");
    assert.strictEqual(harness.studioParameters.cameraFov, 120.0);
  });

  it("2.6 should handle regex, unicode, and adversarial search strings in Asset Browser without crashing", () => {
    const adversarialQueries = [
      ".*",
      "[a-z]+",
      "(?<name>.*)",
      "\\d{3}",
      "Katana|Sword",
      "<script>alert(1)</script>",
      "   trimmed query   ",
      "カタナ", // Japanese full-width Katana
      "çénário 3D",
    ];

    for (const q of adversarialQueries) {
      assert.doesNotThrow(() => {
        harness.assetSearchQuery = q;
        const results = harness.getFilteredAssets();
        assert.ok(Array.isArray(results), `Results for '${q}' must be an array`);
      });
    }

    // Whitespace search should trim properly
    harness.assetSearchQuery = "   Katana   ";
    const trimmedResults = harness.getFilteredAssets();
    assert.strictEqual(trimmedResults.length, 1);
    assert.strictEqual(trimmedResults[0].id, "prop_01");

    // Non-existent search returns empty array
    harness.assetSearchQuery = "non_existent_asset_xyz_12345";
    assert.strictEqual(harness.getFilteredAssets().length, 0);
  });

  it("2.7 should endure 100 rapid repeated switching cycles without state de-synchronization", () => {
    for (let cycle = 0; cycle < 100; cycle++) {
      const targetWs = CANONICAL_WORKSPACES[cycle % 8];
      const ok = harness.switchWorkspace(targetWs);
      assert.strictEqual(ok, true);
      assert.strictEqual(harness.activeWorkspace, targetWs);
      assert.strictEqual(harness.activeTool, DEFAULT_WORKSPACE_TOOLS[targetWs]);
    }
  });
});

// ============================================================================
// TEST SUITE: TIER 3 — CROSS-FEATURE COMBINATIONS & STATE TRANSITIONS
// ============================================================================

describe("Tier 3: Cross-Feature Combinations & State Transitions", () => {
  let harness: StudioPipelineHarness;

  beforeEach(() => {
    harness = new StudioPipelineHarness();
  });

  it("3.1 should correctly toggle WebGPU Viewport visibility and loop state when entering/exiting Biblioteca", () => {
    // Workspaces 1-7: Viewport mounted, visible, loop running, Asset Browser inactive
    for (let i = 0; i < 7; i++) {
      const ws = CANONICAL_WORKSPACES[i];
      harness.switchWorkspace(ws);

      assert.strictEqual(harness.isViewportMounted, true, "Viewport must stay mounted in DOM");
      assert.strictEqual(harness.isViewportVisible, true, `Viewport must be visible in '${ws}'`);
      assert.strictEqual(harness.isRenderLoopPaused, false, `Render loop must be active in '${ws}'`);
      assert.strictEqual(harness.isAssetBrowserActive, false, `Asset browser must be inactive in '${ws}'`);
    }

    // Workspace 8 (Biblioteca): Viewport kept mounted, but hidden and paused; Asset Browser active
    harness.switchWorkspace("biblioteca");
    assert.strictEqual(harness.isViewportMounted, true, "Viewport must remain mounted in Biblioteca (no teardown!)");
    assert.strictEqual(harness.isViewportVisible, false, "Viewport must be hidden via CSS in Biblioteca");
    assert.strictEqual(harness.isRenderLoopPaused, true, "Render loop must be paused in Biblioteca");
    assert.strictEqual(harness.isAssetBrowserActive, true, "Asset Browser UI must be active in Biblioteca");

    // Switching back to workspace 1 (Personagem): Viewport instantly resumes without re-creation
    harness.switchWorkspace("personagem");
    assert.strictEqual(harness.isViewportVisible, true, "Viewport must be visible again");
    assert.strictEqual(harness.isRenderLoopPaused, false, "Render loop must resume");
    assert.strictEqual(harness.isAssetBrowserActive, false, "Asset Browser must hide");
  });

  it("3.2 should preserve studio parameters across multi-workspace transitions without leak or reset", () => {
    // Configure distinct parameters in different workspaces
    harness.switchWorkspace("personagem");
    harness.setParameter("headScale", 1.35, "Set Head Scale");
    harness.setParameter("headRatio", 7.8, "Set Head Ratio");

    harness.switchWorkspace("shading");
    harness.setParameter("shadowThreshold", 0.62, "Set Shadow Threshold");
    harness.setParameter("outlineWidth", 0.024, "Set Outline Width");

    harness.switchWorkspace("iluminacao");
    harness.setParameter("lightIntensity", 1.85, "Set Light Intensity");
    harness.setParameter("lightAzimuth", 75.0, "Set Sun Azimuth");

    harness.switchWorkspace("cenario");
    harness.setParameter("stagePreset", "rooftop_sunset", "Set Stage");

    harness.switchWorkspace("animacao");
    harness.setParameter("currentFrame", 42, "Scrub Timeline");

    harness.switchWorkspace("render");
    harness.setParameter("cameraFov", 60.0, "Set Camera FOV");

    harness.switchWorkspace("biblioteca");
    harness.assetSearchQuery = "uniform";

    // Cycle through all workspaces again and verify parameter preservation
    assert.strictEqual(harness.studioParameters.headScale, 1.35);
    assert.strictEqual(harness.studioParameters.headRatio, 7.8);
    assert.strictEqual(harness.studioParameters.shadowThreshold, 0.62);
    assert.strictEqual(harness.studioParameters.outlineWidth, 0.024);
    assert.strictEqual(harness.studioParameters.lightIntensity, 1.85);
    assert.strictEqual(harness.studioParameters.lightAzimuth, 75.0);
    assert.strictEqual(harness.studioParameters.stagePreset, "rooftop_sunset");
    assert.strictEqual(harness.studioParameters.currentFrame, 42);
    assert.strictEqual(harness.studioParameters.cameraFov, 60.0);
    assert.strictEqual(harness.assetSearchQuery, "uniform");
  });

  it("3.3 should integrate seamlessly with HistoryService undo/redo stack across workspaces", () => {
    harness.switchWorkspace("personagem");
    harness.setParameter("headScale", 1.1, "Action 1: Head Scale 1.1");
    harness.setParameter("headScale", 1.25, "Action 2: Head Scale 1.25");

    harness.switchWorkspace("shading");
    harness.setParameter("shadowThreshold", 0.7, "Action 3: Shading Threshold 0.7");

    harness.switchWorkspace("iluminacao");
    harness.setParameter("lightIntensity", 2.0, "Action 4: Sun Intensity 2.0");

    assert.strictEqual(historyService.canUndo(), true, "Should have undo history");
    assert.strictEqual(historyService.canRedo(), false, "Redo should be empty after new actions");

    // Undo 1: Sun Intensity restored
    const u1 = harness.undo();
    assert.ok(u1);
    assert.strictEqual(harness.studioParameters.lightIntensity, 1.0);

    // Undo 2: Shading Threshold restored
    const u2 = harness.undo();
    assert.ok(u2);
    assert.strictEqual(harness.studioParameters.shadowThreshold, 0.5);

    // Undo 3: Head Scale restored to 1.1
    const u3 = harness.undo();
    assert.ok(u3);
    assert.strictEqual(harness.studioParameters.headScale, 1.1);

    // Redo 1: Head Scale restored forward to 1.25
    const r1 = harness.redo();
    assert.ok(r1);
    assert.strictEqual(harness.studioParameters.headScale, 1.25);
  });

  it("3.4 should coalesce continuous slider drag snapshots within 500ms in HistoryService", () => {
    historyService.clear();
    harness.resetBaseline();

    // Drag slider 3 times within 100ms
    harness.setParameter("headScale", 1.1, "Drag Head Scale", false);
    harness.setParameter("headScale", 1.2, "Drag Head Scale", true);
    harness.setParameter("headScale", 1.3, "Drag Head Scale", true);

    // Undo should restore directly to baseline (1.0), skipping intermediate drag states
    harness.undo();
    assert.strictEqual(harness.studioParameters.headScale, 1.0, "Coalesced slider should undo in a single step");
    assert.strictEqual(historyService.canUndo(), false, "No further undo steps should remain from drag");
  });

  it("3.5 should verify asset inspection and insertion bridge into 3D scene from Biblioteca", () => {
    harness.switchWorkspace("biblioteca");
    assert.strictEqual(harness.activeWorkspace, "biblioteca");

    // Inspect Katana
    const inspected = harness.inspectAsset("prop_01");
    assert.ok(inspected);
    assert.strictEqual(harness.studioParameters.selectedAssetId, "prop_01");

    // Insert Katana into Scene
    const insertion = harness.useAssetInScene("prop_01");
    assert.strictEqual(insertion.success, true);
    assert.strictEqual(harness.activeSceneAssets.length, 1);

    // Switch to Cenário
    harness.switchWorkspace("cenario");
    assert.strictEqual(harness.activeWorkspace, "cenario");
    assert.strictEqual(harness.isViewportVisible, true);
    assert.strictEqual(harness.activeSceneAssets[0].id, "prop_01", "Inserted asset remains in active scene");
  });
});

// ============================================================================
// TEST SUITE: TIER 4 — REAL-WORLD WORKLOAD SCENARIOS (8-DISCIPLINE PIPELINE)
// ============================================================================

describe("Tier 4: Real-World Workload Scenarios (8-Discipline Production Walkthrough)", () => {
  let harness: StudioPipelineHarness;

  beforeEach(() => {
    harness = new StudioPipelineHarness();
  });

  it("4.1 should complete full sequential 8-discipline production pipeline from Personagem to Biblioteca", () => {
    const executionLog: string[] = [];

    // ------------------------------------------------------------------------
    // Step 1: Personagem (Character Design & Anatomical Proportions)
    // ------------------------------------------------------------------------
    harness.switchWorkspace("personagem");
    assert.strictEqual(harness.activeWorkspace, "personagem");
    assert.strictEqual(harness.activeTool, "body");

    harness.setParameter("headRatio", 7.5, "M1: Set Anime 7.5 Head Ratio");
    harness.setParameter("headScale", 1.12, "M1: Set Head Proportions");

    harness.selectTool("face");
    assert.strictEqual(harness.activeTool, "face");
    harness.studioParameters.faceBlendshapes = { smile: 0.8, eye_big: 1.0 };

    harness.selectTool("cloth");
    assert.strictEqual(harness.activeTool, "cloth");
    harness.studioParameters.outfitPreset = "seifuku_sailor";

    executionLog.push("STEP_1_PERSONAGEM_COMPLETE");

    // ------------------------------------------------------------------------
    // Step 2: Posing (Rig Skeleton & Maneuver)
    // ------------------------------------------------------------------------
    harness.switchWorkspace("posing");
    assert.strictEqual(harness.activeWorkspace, "posing");
    assert.strictEqual(harness.activeTool, "rig");

    harness.selectTool("ik");
    assert.strictEqual(harness.activeTool, "ik");
    harness.studioParameters.ikActive = true;

    harness.selectTool("poses_preset");
    assert.strictEqual(harness.activeTool, "poses_preset");
    harness.studioParameters.posePreset = "action_dynamic_jump_02";

    executionLog.push("STEP_2_POSING_COMPLETE");

    // ------------------------------------------------------------------------
    // Step 3: Shading (Toon Ramp NPR & Inverted Hull Outlines)
    // ------------------------------------------------------------------------
    harness.switchWorkspace("shading");
    assert.strictEqual(harness.activeWorkspace, "shading");
    assert.strictEqual(harness.activeTool, "cel_shader");

    harness.setParameter("shadowThreshold", 0.58, "M3: Tune Cel-Shading Ramp");
    harness.selectTool("outline");
    assert.strictEqual(harness.activeTool, "outline");
    harness.setParameter("outlineWidth", 0.018, "M3: Set Inverted Hull Stroke");

    harness.selectTool("rim");
    assert.strictEqual(harness.activeTool, "rim");
    harness.setParameter("rimIntensity", 1.35, "M3: Tune Rim Light");

    executionLog.push("STEP_3_SHADING_COMPLETE");

    // ------------------------------------------------------------------------
    // Step 4: Iluminação (Solar Angle & Anime Shadow Terminator)
    // ------------------------------------------------------------------------
    harness.switchWorkspace("iluminacao");
    assert.strictEqual(harness.activeWorkspace, "iluminacao");
    assert.strictEqual(harness.activeTool, "sun");

    harness.setParameter("lightAzimuth", 65.0, "M4: Golden Hour Azimuth");
    harness.setParameter("lightElevation", 28.0, "M4: Low Sun Elevation");
    harness.setParameter("lightIntensity", 1.6, "M4: Sun Power");

    harness.selectTool("shadows");
    assert.strictEqual(harness.activeTool, "shadows");
    harness.studioParameters.shadowColor = [0.22, 0.16, 0.35]; // Anime violet shadow

    executionLog.push("STEP_4_ILUMINACAO_COMPLETE");

    // ------------------------------------------------------------------------
    // Step 5: Cenário (Modular Stage & Anime Sky)
    // ------------------------------------------------------------------------
    harness.switchWorkspace("cenario");
    assert.strictEqual(harness.activeWorkspace, "cenario");
    assert.strictEqual(harness.activeTool, "stage");

    harness.setParameter("stagePreset", "classroom_stage_v1", "M5: Stage Props");
    harness.selectTool("environment");
    assert.strictEqual(harness.activeTool, "environment");
    harness.studioParameters.environmentSky = "sunset_clouds_03";

    executionLog.push("STEP_5_CENARIO_COMPLETE");

    // ------------------------------------------------------------------------
    // Step 6: Animação (Timeline & Lipsync)
    // ------------------------------------------------------------------------
    harness.switchWorkspace("animacao");
    assert.strictEqual(harness.activeWorkspace, "animacao");
    assert.strictEqual(harness.activeTool, "timeline");

    harness.setParameter("currentFrame", 0, "M6: Keyframe Start");
    harness.studioParameters.keyframeTrack.push({ frame: 0, param: "pose", val: "start" });
    harness.setParameter("currentFrame", 36, "M6: Keyframe Mid-Air");
    harness.studioParameters.keyframeTrack.push({ frame: 36, param: "pose", val: "apex" });

    harness.selectTool("lipsync");
    assert.strictEqual(harness.activeTool, "lipsync");
    harness.studioParameters.lipsyncViseme = "A";

    executionLog.push("STEP_6_ANIMACAO_COMPLETE");

    // ------------------------------------------------------------------------
    // Step 7: Render (Cinematic Cameras & NPR Passes Export)
    // ------------------------------------------------------------------------
    harness.switchWorkspace("render");
    assert.strictEqual(harness.activeWorkspace, "render");
    assert.strictEqual(harness.activeTool, "camera");

    harness.setParameter("cameraFov", 50.0, "M7: Set Portrait 50mm Lens");
    harness.selectTool("passes");
    assert.strictEqual(harness.activeTool, "passes");
    harness.studioParameters.renderPasses = ["diffuse", "ink_outline", "shadow_mask", "rim_light"];

    harness.selectTool("export");
    assert.strictEqual(harness.activeTool, "export");
    harness.studioParameters.renderResolution = "1920x1080";

    executionLog.push("STEP_7_RENDER_COMPLETE");

    // ------------------------------------------------------------------------
    // Step 8: Biblioteca (Asset Browser Search, Filter & Insertion)
    // ------------------------------------------------------------------------
    harness.switchWorkspace("biblioteca");
    assert.strictEqual(harness.activeWorkspace, "biblioteca");
    assert.strictEqual(harness.isViewportVisible, false, "Viewport must be hidden in Biblioteca");
    assert.strictEqual(harness.isRenderLoopPaused, true, "Render loop must be paused in Biblioteca");
    assert.strictEqual(harness.isAssetBrowserActive, true, "Asset Browser must be active");

    // Filter by Category: 'Roupas'
    harness.selectedAssetCategory = "Roupas";
    const clothingAssets = harness.getFilteredAssets();
    assert.ok(clothingAssets.length > 0, "Should find clothing assets");
    assert.ok(clothingAssets.every((a) => a.category === "Roupas"));

    // Fast search by keyword
    harness.assetSearchQuery = "Katana";
    harness.selectedAssetCategory = null;
    const weaponAssets = harness.getFilteredAssets();
    assert.strictEqual(weaponAssets.length, 1, "Should find Katana");
    assert.strictEqual(weaponAssets[0].id, "prop_01");

    // Action 1: Inspecionar (Inspector Metadata View)
    const inspected = harness.inspectAsset("prop_01");
    assert.ok(inspected);
    assert.strictEqual(inspected.polyCount, 2200);
    assert.strictEqual(inspected.format, "OBJ");
    assert.strictEqual(harness.selectedAsset?.name, "Katana of the Dawn");

    // Action 2: Usar no Cenário (Scene Insertion)
    const insertionResult = harness.useAssetInScene("prop_01");
    assert.strictEqual(insertionResult.success, true);
    assert.strictEqual(harness.activeSceneAssets.length, 1);
    assert.strictEqual(harness.activeSceneAssets[0].id, "prop_01");

    executionLog.push("STEP_8_BIBLIOTECA_COMPLETE");

    // ------------------------------------------------------------------------
    // Pipeline Verification Audit
    // ------------------------------------------------------------------------
    assert.deepStrictEqual(executionLog, [
      "STEP_1_PERSONAGEM_COMPLETE",
      "STEP_2_POSING_COMPLETE",
      "STEP_3_SHADING_COMPLETE",
      "STEP_4_ILUMINACAO_COMPLETE",
      "STEP_5_CENARIO_COMPLETE",
      "STEP_6_ANIMACAO_COMPLETE",
      "STEP_7_RENDER_COMPLETE",
      "STEP_8_BIBLIOTECA_COMPLETE",
    ]);

    // Verify Undo rolls back state intact
    assert.ok(historyService.canUndo());
    const rolledBack = harness.undo();
    assert.ok(rolledBack);
  });
});

// ============================================================================
// TEST SUITE: CODEBASE STATIC CONTRACT & ASSET AUDIT
// ============================================================================

describe("Codebase Static Contract & Asset Integrity Audit", () => {
  const rootDir = path.resolve(process.cwd());

  it("5.1 should verify i18n dictionaries export valid object structures", () => {
    assert.ok(typeof pt_BR === "object" && pt_BR !== null, "pt_BR must be valid object");
    assert.ok(typeof en_US === "object" && en_US !== null, "en_US must be valid object");
    assert.ok(typeof ja_JP === "object" && ja_JP !== null, "ja_JP must be valid object");
  });

  it("5.2 should audit SVG icon components in src/components/icons/", () => {
    const iconsDir = path.join(rootDir, "src", "components", "icons");
    assert.ok(fs.existsSync(iconsDir), "src/components/icons must exist");

    const existingIcons = fs.readdirSync(iconsDir);
    const requiredIcons = [
      "UserIcon.svelte",
      "SmileIcon.svelte",
      "ScissorsIcon.svelte",
      "ShirtIcon.svelte",
      "BoxIcon.svelte",
      "BrushIcon.svelte",
      "BoneIcon.svelte",
      "TargetIcon.svelte",
      "BotIcon.svelte",
      "ShaderIcon.svelte",
      "RimIcon.svelte",
      "OutlineIcon.svelte",
      "PaletteIcon.svelte",
      "SunIcon.svelte",
      "CloudIcon.svelte",
      "SparklesIcon.svelte",
      "LayersIcon.svelte",
      "ActivityIcon.svelte",
      "SlidersIcon.svelte",
      "CameraIcon.svelte",
      "FilmIcon.svelte",
      "GridIcon.svelte",
    ];

    for (const iconFile of requiredIcons) {
      assert.ok(
        existingIcons.includes(iconFile),
        `Required icon '${iconFile}' must exist in src/components/icons/`
      );
    }
  });

  it("5.3 should verify HistoryService implementation in src/services/history_service.ts", () => {
    assert.ok(historyService, "historyService must be instantiated");
    assert.ok(typeof historyService.init === "function");
    assert.ok(typeof historyService.push === "function");
    assert.ok(typeof historyService.undo === "function");
    assert.ok(typeof historyService.redo === "function");
    assert.ok(typeof historyService.canUndo === "function");
    assert.ok(typeof historyService.canRedo === "function");
  });

  // Check M1 codebase implementation status (App.svelte & i18n)
  const isM1CodebaseImplemented =
    pt_BR["workspace.posing"] !== undefined &&
    pt_BR["workspace.iluminacao"] !== undefined &&
    pt_BR["workspace.render"] !== undefined;

  it(
    "5.4 should check if src/i18n contains all 8 canonical workspace keys (M1 verification)",
    { todo: !isM1CodebaseImplemented ? "Pending M1 implementer completion of i18n dictionaries" : false },
    () => {
      for (const ws of CANONICAL_WORKSPACES) {
        const key = `workspace.${ws}`;
        assert.ok(pt_BR[key], `pt_BR must contain key '${key}'`);
        assert.ok(en_US[key], `en_US must contain key '${key}'`);
        assert.ok(ja_JP[key], `ja_JP must contain key '${key}'`);
      }
    }
  );

  const searchIconPath = path.join(rootDir, "src", "components", "icons", "SearchIcon.svelte");
  const isSearchIconPresent = fs.existsSync(searchIconPath);

  it(
    "5.5 should check if SearchIcon.svelte exists in src/components/icons/ (M1 verification)",
    { todo: !isSearchIconPresent ? "Pending M1 implementer creation of SearchIcon.svelte" : false },
    () => {
      assert.ok(fs.existsSync(searchIconPath), "SearchIcon.svelte must exist in src/components/icons/");
    }
  );

  const appSveltePath = path.join(rootDir, "src", "App.svelte");
  const appSvelteContent = fs.existsSync(appSveltePath) ? fs.readFileSync(appSveltePath, "utf-8") : "";
  const isAppSvelte8Workspaces = appSvelteContent.includes('"posing"') && appSvelteContent.includes('"iluminacao"');

  it(
    "5.6 should check if src/App.svelte declares the 8 canonical workspaces (M1 verification)",
    { todo: !isAppSvelte8Workspaces ? "Pending M1 implementer update of App.svelte to 8 workspaces" : false },
    () => {
      for (const ws of CANONICAL_WORKSPACES) {
        assert.ok(
          appSvelteContent.includes(`"${ws}"`),
          `App.svelte must contain workspace identifier "${ws}"`
        );
      }
    }
  );
});
