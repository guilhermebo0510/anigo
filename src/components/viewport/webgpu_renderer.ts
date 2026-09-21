// ANIGO Native 3D Viewport Renderer
// Real-time NPR Cel-Shading & Inverted Hull Outlines with WebGPU and WebGL2 Fallback

import {
  createCameraRay,
  raycastTactileHulls,
  projectTactileDrag,
  type AnatomicalSegment,
  type RaycastHit,
  type TactileDragResult
} from "./tactile";
import {
  clampCatalog,
  clampNumber,
  getSliderDef,
  isKnownSliderId,
  sanitizeFinite,
} from "../../services/character_state";
import { CANONICAL_SLIDERS, type MorphSlider } from "../../services/morph_catalog";
import {
  channelWeightOf,
  type CoreSnapshotDelivery,
} from "../../services/core_bridge";
import {
  MeshValidationError,
  validateAttributeMesh,
  validateRenderableMesh,
} from "../../services/mesh_validation";
import {
  RendererDiagnostics,
  type DiagnosticCode,
  type DiagnosticsSummary,
  type RenderDiagnostic,
} from "../../services/render_diagnostics";
import {
  applyDeltasCpu,
  geometrySignature,
  packChannelRecordsWithWeights,
  viewportGeometryFromDecoded,
  type ViewportGeometry,
} from "../../services/viewport_mesh";
import { GlbParseError, loadGlbMesh } from "../../services/gltf_loader";
import {
  DEFAULT_CAMERA_FAR,
  DEFAULT_CAMERA_NEAR,
  viewProjectionMatrix,
} from "../../services/camera_math";
// P0 renderer: canonical shader source is `crates/anigo-renderer/shaders/` — the same
// files the Rust (wgpu) renderer loads with `include_str!`. There is exactly one
// copy of every production shader in the repo (`contracts/render_contract_v1.json`).
// @ts-ignore - Vite ?raw import
import celShaderSource from "../../../crates/anigo-renderer/shaders/cel_shading.wgsl?raw";
// @ts-ignore - Vite ?raw import
import outlineShaderSource from "../../../crates/anigo-renderer/shaders/inverted_hull.wgsl?raw";
// @ts-ignore - Vite ?raw import
import morphComputeSource from "../../../crates/anigo-renderer/shaders/morph_sparse_compute.wgsl?raw";
// P0 renderer: the WebGL2 fallback is *not* production — its sources live in
// `webgl2_fallback/` and are listed as `role: "fallback_webgl2"` in the contract.
// @ts-ignore - Vite ?raw import
import vsCel from "../../../crates/anigo-renderer/shaders/webgl2_fallback/cel_vertex.glsl?raw";
// @ts-ignore - Vite ?raw import
import fsCel from "../../../crates/anigo-renderer/shaders/webgl2_fallback/cel_fragment.glsl?raw";
// @ts-ignore - Vite ?raw import
import vsOutline from "../../../crates/anigo-renderer/shaders/webgl2_fallback/outline_vertex.glsl?raw";
// @ts-ignore - Vite ?raw import
import fsOutline from "../../../crates/anigo-renderer/shaders/webgl2_fallback/outline_fragment.glsl?raw";
// P0 renderer: todo o estado de pipeline (passes, MSAA, formatos, blend, depth,
// uniforms e toon ramp) vem do contrato congelado — não existem literais de
// renderização espalhados neste arquivo.
import {
  addressMode,
  assertShaderSource,
  depthFormat,
  filterMode,
  msaaSampleCount,
  renderPasses,
  RENDER_CONTRACT,
  toonRampBytes,
  toonRampFingerprint,
  expectedToonRampFingerprint,
  uniformSize,
  vertexBufferLayout,
} from "../../contracts/render_contract.v1";
import {
  cameraUniformFloats,
  lightUniformFloats,
  materialUniformFloats,
  outlineUniformFloats,
  IDENTITY_MAT4,
} from "../../services/render_uniforms";

export interface ViewportMetrics {
  fps: number;
  frameTimeMs: number;
  triangles: number;
  drawCalls: number;
  adapterName: string;
  backend: string;
}

export type MeshPreset = "mannequin" | "sphere" | "cube";

export interface VertexData {
  pos: [number, number, number];
  normal: [number, number, number];
  uv: [number, number];
  color: [number, number, number, number];
  joints?: [number, number, number, number];
  weights?: [number, number, number, number];
}

export function packVertices(vertices: VertexData[]): Float32Array {
  const buffer = new ArrayBuffer(vertices.length * 72);
  const f32 = new Float32Array(buffer);
  const u16 = new Uint16Array(buffer);

  for (let i = 0; i < vertices.length; i++) {
    const v = vertices[i];
    const fIdx = i * 18;
    const uIdx = i * 36;

    // 0..12: pos (3 floats)
    f32[fIdx + 0] = v.pos[0];
    f32[fIdx + 1] = v.pos[1];
    f32[fIdx + 2] = v.pos[2];

    // 12..24: normal (3 floats)
    f32[fIdx + 3] = v.normal[0];
    f32[fIdx + 4] = v.normal[1];
    f32[fIdx + 5] = v.normal[2];

    // 24..32: uv (2 floats)
    f32[fIdx + 6] = v.uv[0];
    f32[fIdx + 7] = v.uv[1];

    // 32..48: color (4 floats)
    f32[fIdx + 8] = v.color[0];
    f32[fIdx + 9] = v.color[1];
    f32[fIdx + 10] = v.color[2];
    f32[fIdx + 11] = v.color[3];

    // 48..56: joints (4 u16 at byte offset 48 = 24 u16 elements)
    const j = v.joints || [0, 0, 0, 0];
    u16[uIdx + 24] = j[0];
    u16[uIdx + 25] = j[1];
    u16[uIdx + 26] = j[2];
    u16[uIdx + 27] = j[3];

    // 56..72: weights (4 floats at byte offset 56 = 14 f32 elements)
    const w = v.weights || [1.0, 0.0, 0.0, 0.0];
    f32[fIdx + 14] = w[0];
    f32[fIdx + 15] = w[1];
    f32[fIdx + 16] = w[2];
    f32[fIdx + 17] = w[3];
  }

  return f32;
}

/**
 * Janela de feedback otimista (ms) para arrasto de slider.
 *
 * O peso aplicado na GPU é sempre um peso de canal do núcleo; durante o arrasto
 * o viewport adianta localmente o *peso* (`valor − default`, a mesma definição
 * de `DeformationInputs::weight_of`) para não esperar o ida-e-volta, e descarta
 * esse adiantamento assim que o snapshot do núcleo chega.
 */
const LIVE_WEIGHT_WINDOW_MS = 300;

export class WebGpuViewportRenderer {
  private canvas: HTMLCanvasElement;
  private backend: "webgpu" | "webgl2" = "webgpu";

  // WebGPU Handles
  private adapter: GPUAdapter | null = null;
  private device: GPUDevice | null = null;
  private context: GPUCanvasContext | null = null;
  private format: GPUTextureFormat = "bgra8unorm";
  private depthTexture: GPUTexture | null = null;
  private depthView: GPUTextureView | null = null;
  // P0-07: MSAA conforme o contrato (4x; era sampleCount=1)
  private msaaColorTexture: GPUTexture | null = null;
  private msaaColorView: GPUTextureView | null = null;
  private sampleCount: number = msaaSampleCount();
  private celPipeline: GPURenderPipeline | null = null;
  private outlinePipeline: GPURenderPipeline | null = null; // P1-06 uses custom extruded normals (geometry includes outlineNormal attribute when available)
  private vertexBuffer: GPUBuffer | null = null;
  private indexBuffer: GPUBuffer | null = null;
  private cameraBuffer: GPUBuffer | null = null; // P2-14 model+normal matrix per object (was identity)
  private lightBuffer: GPUBuffer | null = null;
  // P2-04 ambient hemisphere sky/ground stub
  // P1-05 shadow map stub — single light currently uses N·L + PCF penumbra; full shadow map/SDF placeholder uniform for future multi-light
  private materialBuffer: GPUBuffer | null = null;
  private outlineBuffer: GPUBuffer | null = null;
  private celBindGroup: GPUBindGroup | null = null;
  private outlineBindGroup: GPUBindGroup | null = null;
  private toonRampTexture: GPUTexture | null = null;
  // P1-06 outline normals — vertex includes normal for shell extrusion (fallback to position normal if custom unavailable)
  private shadowMapTexture: GPUTexture | null = null; // P1-05 placeholder for shadow map/SDF
  // P1-04 ramp is now an asset (src/assets/toon_ramp.png) — loaded via fetch+createTexture; fallback procedural kept
  private toonRampSampler: GPUSampler | null = null;

  // WebGPU Sparse Morph Compute Pipeline
  private morphPipeline: GPUComputePipeline | null = null;
  private morphBindGroupLayout: GPUBindGroupLayout | null = null;
  private morphHeaderBuffer: GPUBuffer | null = null;
  private morphBaseBuffer: GPUBuffer | null = null;
  private morphDeltasBuffer: GPUBuffer | null = null;
  private morphChannelsBuffer: GPUBuffer | null = null;
  // P1-09: debounce morph churn — was recreating buffers every drag frame
  private morphedVertexBuffer: GPUBuffer | null = null;
  private morphBindGroup: GPUBindGroup | null = null;
  private morphVertexCount: number = 0;
  // P0 §7.5: a geometria canônica do viewport vem do snapshot do núcleo.
  private coreGeometry: ViewportGeometry | null = null;
  private coreStaticRevision: number = 0;
  private coreDynamicRevision: number = 0;
  private coreAuthority: "core" | "reference_ts" | "unavailable" = "unavailable";
  private coreCoverage: CoreSnapshotDelivery["coverage"] | null = null;
  /** Pesos autorais do núcleo, por slider (domínio = canais do snapshot). */
  private channelWeights: Map<string, number> = new Map();
  /** Adiantamento local do arrasto (janela curta, nunca vira geometria). */
  private liveWeights: Map<string, number> = new Map();
  private liveWeightsExpiry: number = 0;
  private coreGeometryRequested: boolean = false;
  /** Cor de fundo autorais do núcleo (`render.background_color`). */
  private clearColor: [number, number, number, number] = [0.08, 0.09, 0.13, 1.0];
  private gpuMorphActive: boolean = false;
  private gpuMorphDirty: boolean = false;
  // P0-05: persistent buffer capacities (no destroy/create churn per event).
  private vertexCapacityBytes: number = 0;
  private indexCapacityBytes: number = 0;
  /** P0-10: model load failures surface here (and throw to the caller). */
  public onModelLoadError?: (message: string) => void;
  /** P1-02: diagnóstico estruturado (nada de falha silenciosa no caminho crítico). */
  public onDiagnostic?: (diagnostic: RenderDiagnostic) => void;
  private diagnostics = new RendererDiagnostics();
  /**
   * P0 §7.5: chamado quando o viewport precisa de um snapshot novo do núcleo
   * (geometria ausente/desatualizada). O shell é quem fala com o núcleo.
   */
  public onCoreGeometryRequired?: () => void;

  // Somatotype & Morphs State (Sub-Sprint 3.3 & 3.5)
  public somatotypeEndo: number = 0.33;
  public somatotypeMeso: number = 0.34;
  public somatotypeEcto: number = 0.33;
  public genderDimorphism: number = 1.0;
  public activeMorphWeights: Map<string, number> = new Map();

  // WebGL2 Fallback Handles
  private gl: WebGL2RenderingContext | null = null;
  private glCelProgram: WebGLProgram | null = null;
  private glOutlineProgram: WebGLProgram | null = null;
  private glVao: WebGLVertexArrayObject | null = null;
  private glVbo: WebGLBuffer | null = null;
  private glIbo: WebGLBuffer | null = null;

  private indexCount: number = 0;

  private canonicalVertices: Float32Array | null = null;
  private canonicalIndices: Uint32Array | null = null;
  private canonicalBaseVertices: VertexData[] | null = null;
  public canonicalGender: "male" | "female" = "male";
  private canonicalModelCache = new Map<string, { vertices: VertexData[], indices: Uint32Array }>(); // P2-12 cache
  private loadAbortController: AbortController | null = null; // P2-12 abort

  // Preset & Proportions State (BOND Skeletal Sync)
  public currentPreset: MeshPreset = "mannequin";
  public headScale: number = 1.0;
  public headRatio: number = 6.5;
  public shoulderWidth: number = 1.0;
  public legLength: number = 1.0;
  public armLength: number = 1.0;
  public neckLength: number = 1.0;
  public torsoLength: number = 1.0;
  public heightOverall: number = 1.0;

  // Camera State
  public eye: [number, number, number] = [0.0, 1.5, 3.5];
  public target: [number, number, number] = [0.0, 1.0, 0.0];
  public up: [number, number, number] = [0.0, 1.0, 0.0];
  public fov: number = (45.0 * Math.PI) / 180.0;
  private recenterAnim: {
    startEye: [number, number, number];
    startTarget: [number, number, number];
    startUp: [number, number, number];
    endEye: [number, number, number];
    endTarget: [number, number, number];
    endUp: [number, number, number];
    startTime: number;
    duration: number;
  } | null = null;

  // Light State — P0-02: tint default neutro branco para não duplicar shade_color × shadow_color
  public lightDir: [number, number, number] = [0.577, 0.577, 0.577];
  public lightColor: [number, number, number] = [1.0, 0.98, 0.95];
  public lightIntensity: number = 1.0;
  public ambientIntensity: number = 0.35;
  // P0-02 Hotfix: neutral white tint so shade_color alone defines shadow color until UI separates tint control.
  public shadowColor: [number, number, number] = [1.0, 1.0, 1.0];
  public shadowSaturation: number = 1.15;
  public ambientSky: [number, number, number] = [0.52, 0.60, 0.78]; // P2-04 sky
  public ambientGround: [number, number, number] = [0.25, 0.20, 0.18]; // P2-04 ground
  public specularSize: number = 0.45; // P2-07 separate from intensity
  public aoIntensity: number = 0.85; // P2-05
  // P0-09: separate rim tint (was incorrectly using shadow_color)
  public rimColor: [number, number, number] = [0.576, 0.773, 0.992]; // #93c5fd

  // Material State
  public baseColor: [number, number, number, number] = [0.98, 0.92, 0.85, 1.0];
  public shadeColor: [number, number, number, number] = [0.82, 0.73, 0.78, 1.0];
  public outlineColor: [number, number, number, number] = [0.25, 0.15, 0.20, 1.0];
  public outlineWidth: number = 0.0035;
  public outlineOpacity: number = 1.0;
  public outlineSmoothness: number = 0.0;
  public outlineDepthBias: number = 0.0;
  public shadowThreshold: number = 0.50;
  public toonSmoothness: number = 0.02;
  public specIntensity: number = 0.40;
  public specExponent: number = 32.0;
  public specSoftness: number = 0.05;
  public specOffset: number = 0.0;
  public specColor: [number, number, number, number] = [1.0, 1.0, 1.0, 1.0];
  public rimIntensity: number = 0.80;
  public rimSpread: number = 0.40;
  public hueShift: number = -15.0; // degrees (-60 to +60)
  public toonSteps: number = 1.0; // 1.0 = hard anime cel, 2.0 = 2-tier Ghibli, 0.0 = continuous

  // Graphics Pipeline Settings
  public targetFps: number = 120;
  private lastFrameTimestamp: number = 0;
  public dpiMultiplier: number = 1.0;
  private cssWidth: number = 800;
  private cssHeight: number = 600;
  public vsyncEnabled: boolean = true;

  private animationFrameId: number | null = null;
  private frameCounter: number = 0; // P2-08 GPU timestamp via GPUQuerySet fallback rAF
  private fpsTimer: number = performance.now();
  private isRendering: boolean = false;
  private isPaused: boolean = false;
  private wasPausedByVisibility: boolean = false;
  public onMetricsUpdate?: (metrics: ViewportMetrics) => void;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
  }

  public async initialize(): Promise<boolean> {
    // 0. Pre-load canonical humanoid model
    try {
      await this.loadCanonicalModel("male");
    } catch (e) {
      this.report("model_load_failed", "falha ao pré-carregar a malha canônica", {
        detail: e instanceof Error ? e.message : String(e),
      });
    }

    // 1. Attempt Native WebGPU
    if (typeof navigator !== "undefined" && navigator.gpu) {
      try {
        this.adapter = await navigator.gpu.requestAdapter({
          powerPreference: "high-performance",
        });
        if (this.adapter) {
          this.device = await this.adapter.requestDevice();
          this.context = this.canvas.getContext("webgpu");
          if (this.context) {
            this.format = navigator.gpu.getPreferredCanvasFormat();
            this.context.configure({
              device: this.device,
              format: this.format,
              alphaMode: "premultiplied",
              presentMode: this.vsyncEnabled ? "fifo" : "immediate",
            });

            // P0-06: observe GPU validation errors (was silent black screen)
            try {
              (this.device as any).addEventListener?.("uncapturederror", (e: any) => {
                this.report("gpu_device_error", "erro não capturado do device WebGPU", {
                  detail: String(e?.error?.message ?? e?.error ?? e ?? "uncaptured"),
                });
                // surface as observable metric fallback
                this.onMetricsUpdate?.({
                  fps: 0,
                  frameTimeMs: 0,
                  triangles: Math.floor(this.indexCount/3),
                  drawCalls: 0,
                  adapterName: "GPU Error: " + (e?.error?.message || "uncaptured"),
                  backend: "WebGPU-Error",
                } as any);
              });
              // push validation scope to surface pipeline errors
              (this.device as any).pushErrorScope?.("validation");
            } catch (e) {
              this.report("gpu_device_error", "não foi possível instalar o observador de erros da GPU", {
                detail: e instanceof Error ? e.message : String(e),
              });
            }

            this.buildShadersAndPipelines();
            this.buildGeometryBuffers();
            this.buildUniformBuffers();

            const initW = this.canvas.clientWidth > 50 ? this.canvas.clientWidth : 800;
            const initH = this.canvas.clientHeight > 50 ? this.canvas.clientHeight : 600;
            this.resize(initW, initH);

            this.backend = "webgpu";
            this.startRenderLoop();
            console.log("[ANIGO 3D] Native WebGPU initialized successfully at 120+ FPS.");
            return true;
          }
        }
      } catch (e) {
        this.report("webgpu_unavailable", "WebGPU indisponível — usando o fallback WebGL2", {
          detail: e instanceof Error ? e.message : String(e),
        });
      }
    }

    // 2. Graceful WebGL2 Fallback (ensures scene is NEVER a black screen)
    console.log("[ANIGO 3D] Initializing WebGL2 hardware fallback renderer...");
    const success = this.initWebGL2();
    if (success) {
      this.backend = "webgl2";
      this.startRenderLoop();
    }
    return success;
  }

  // ==========================================
  // WebGPU Shaders & Pipelines
  // ==========================================
  private buildShadersAndPipelines() {
    if (!this.device) return;

    // Os bytes empacotados no bundle precisam ser exatamente os do contrato
    // (os mesmos que o Rust carrega com `include_str!`), senão não há paridade
    // headless↔viewport. Falha na inicialização em vez de renderizar diferente.
    assertShaderSource("cel_shading", celShaderSource);
    assertShaderSource("inverted_hull", outlineShaderSource);
    assertShaderSource("morph_sparse_compute", morphComputeSource);
    assertShaderSource("webgl2_fallback/cel_vertex", vsCel);
    assertShaderSource("webgl2_fallback/cel_fragment", fsCel);
    assertShaderSource("webgl2_fallback/outline_vertex", vsOutline);
    assertShaderSource("webgl2_fallback/outline_fragment", fsOutline);

    const celShaderCode = celShaderSource;

    const outlineShaderCode = outlineShaderSource;

    const celModule = this.device.createShaderModule({ code: celShaderCode });
    const outlineModule = this.device.createShaderModule({ code: outlineShaderCode });

    const passSpec = (name: string) => {
      const pass = renderPasses().find((candidate) => candidate.name === name);
      if (!pass) throw new Error(`passe '${name}' não está no render contract`);
      return pass;
    };
    const celPass = passSpec("cel");
    const outlinePass = passSpec("outline");

    const contractVertexLayout = vertexBufferLayout() as unknown as GPUVertexBufferLayout;
    const blendStateOf = (pass: { blend?: string }) =>
      pass.blend === "src_alpha_one_minus_src_alpha"
        ? {
            color: { srcFactor: "src-alpha", dstFactor: "one-minus-src-alpha", operation: "add" },
            alpha: { srcFactor: "one", dstFactor: "one-minus-src-alpha", operation: "add" },
          }
        : undefined;

    this.celPipeline = this.device.createRenderPipeline({
      layout: "auto",
      vertex: {
        module: celModule,
        entryPoint: celPass.vertex_entry ?? "vs_main",
        buffers: [contractVertexLayout],
      },
      fragment: {
        module: celModule,
        entryPoint: celPass.fragment_entry ?? "fs_main",
        targets: [{
            format: this.format,
            // contrato: cel é opaco (blend "none") — sem blend, economiza banda
            blend: blendStateOf(celPass),
        }],
      },
      primitive: {
        topology: "triangle-list",
        cullMode: (celPass.cull_mode ?? "back") as any,
      },
      depthStencil: {
        format: depthFormat() as any,
        depthWriteEnabled: celPass.depth_write ?? true,
        depthCompare: (celPass.depth_compare ?? "less-equal") as any,
      },
      multisample: { count: msaaSampleCount() },
    });

    this.outlinePipeline = this.device.createRenderPipeline({
      layout: "auto",
      vertex: {
        module: outlineModule,
        entryPoint: outlinePass.vertex_entry ?? "vs_main",
        buffers: [contractVertexLayout],
      },
      fragment: {
        module: outlineModule,
        entryPoint: outlinePass.fragment_entry ?? "fs_main",
        targets: [{
            format: this.format,
            blend: blendStateOf(outlinePass),
        }],
      },
      primitive: {
        topology: "triangle-list",
        cullMode: (outlinePass.cull_mode ?? "front") as any, // hull invertido: backfaces extrudadas
      },
      depthStencil: {
        format: depthFormat() as any,
        // contrato: outline não escreve profundidade e usa bias 1/1
        depthWriteEnabled: outlinePass.depth_write ?? false,
        depthBias: outlinePass.depth_bias?.constant ?? 0,
        depthBiasSlopeScale: outlinePass.depth_bias?.slope_scale ?? 0,
        depthBiasClamp: outlinePass.depth_bias?.clamp ?? 0,
        depthCompare: (outlinePass.depth_compare ?? "less-equal") as any,
      },
      multisample: { count: msaaSampleCount() },
    });

    try {
      const morphModule = this.device.createShaderModule({
        label: "Sparse Morph Compute Module",
        code: morphComputeSource,
      });

      this.morphBindGroupLayout = this.device.createBindGroupLayout({
        label: "Sparse Morph Bind Group Layout",
        entries: [
          {
            binding: 0,
            visibility: GPUShaderStage.COMPUTE,
            buffer: { type: "uniform" },
          },
          {
            binding: 1,
            visibility: GPUShaderStage.COMPUTE,
            buffer: { type: "read-only-storage" },
          },
          {
            binding: 2,
            visibility: GPUShaderStage.COMPUTE,
            buffer: { type: "read-only-storage" },
          },
          {
            binding: 3,
            visibility: GPUShaderStage.COMPUTE,
            buffer: { type: "read-only-storage" },
          },
          {
            binding: 4,
            visibility: GPUShaderStage.COMPUTE,
            buffer: { type: "storage" },
          },
        ],
      });

      const morphPipelineLayout = this.device.createPipelineLayout({
        label: "Sparse Morph Pipeline Layout",
        bindGroupLayouts: [this.morphBindGroupLayout],
      });

      this.morphPipeline = this.device.createComputePipeline({
        layout: morphPipelineLayout,
        compute: {
          module: morphModule,
          entryPoint: "cs_accumulate_morphs",
        },
      });
    } catch (e) {
      this.report("compute_init_failed", "compute canônico de morphs não inicializou (sem morphs na GPU)", {
        detail: e instanceof Error ? e.message : String(e),
      });
    }
  }

  // ==========================================
  // WebGL2 Fallback Implementation
  // ==========================================
  private initWebGL2(): boolean {
    let gl: WebGL2RenderingContext | null = this.canvas.getContext("webgl2", { antialias: true, alpha: false }) as any;
    // P0-06: if canvas already bound to webgpu (fallback inatingível), try to replace canvas element
    if (!gl && (this.context as any)) {
      console.warn("[ANIGO 3D] Canvas already bound to WebGPU, cloning for WebGL2 fallback");
      try {
        const parent = this.canvas.parentElement;
        const newCanvas = this.canvas.cloneNode(false) as HTMLCanvasElement;
        // preserve size/style
        newCanvas.width = this.canvas.width;
        newCanvas.height = this.canvas.height;
        newCanvas.style.cssText = (this.canvas as any).style?.cssText || "";
        if (parent) parent.replaceChild(newCanvas, this.canvas);
        this.canvas = newCanvas;
        gl = this.canvas.getContext("webgl2", { antialias: true, alpha: false }) as any;
      } catch (e) {
        console.error("[ANIGO 3D] Failed to clone canvas for fallback:", e);
      }
    }
    if (!gl) {
      this.report("backend_unavailable", "WebGPU e WebGL2 falharam: não há backend para desenhar");
      // P0-06: surface black-screen failure as observable DOM overlay instead of silent
      try {
        const overlay = document.createElement("div");
        overlay.textContent = "[ANIGO] Falha ao inicializar WebGPU e WebGL2 — verifique driver/GPU. Veja console para detalhes.";
        overlay.style.cssText = "position:absolute;inset:0;display:flex;align-items:center;justify-content:center;background:#1a1d2e;color:#ff6b6b;padding:16px;text-align:center;font:13px sans-serif;z-index:9999";
        this.canvas.parentElement?.appendChild(overlay);
      } catch (e) {
        this.report("unexpected_error", "não foi possível exibir o aviso de falha de backend", {
          detail: e instanceof Error ? e.message : String(e),
        });
      }
      return false;
    }
    this.gl = gl as any;

    const compileShader = (type: number, src: string) => {
      const s = gl.createShader(type)!;
      gl.shaderSource(s, src);
      gl.compileShader(s);
      if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
        console.error("[WebGL2 Shader Error]", gl.getShaderInfoLog(s));
        gl.deleteShader(s);
        return null;
      }
      return s;
    };

    const linkProgram = (vsSrc: string, fsSrc: string) => {
      const vs = compileShader(gl.VERTEX_SHADER, vsSrc);
      const fs = compileShader(gl.FRAGMENT_SHADER, fsSrc);
      if (!vs || !fs) return null;
      const prog = gl.createProgram()!;
      gl.attachShader(prog, vs);
      gl.attachShader(prog, fs);
      gl.linkProgram(prog);
      if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
        console.error("[WebGL2 Link Error]", gl.getProgramInfoLog(prog));
        return null;
      }
      return prog;
    };

    this.glCelProgram = linkProgram(vsCel, fsCel);
    this.glOutlineProgram = linkProgram(vsOutline, fsOutline);

    this.buildGeometryBuffers();
    const initW = this.canvas.clientWidth > 50 ? this.canvas.clientWidth : 800;
    const initH = this.canvas.clientHeight > 50 ? this.canvas.clientHeight : 600;
    this.resize(initW, initH);

    return !!(this.glCelProgram && this.glOutlineProgram);
  }

  // ==========================================
  // Geometry Generation & Buffers
  // ==========================================
  public switchPreset(preset: MeshPreset, headScale?: number, headRatio?: number) {
    this.currentPreset = preset;
    if (headScale !== undefined) this.headScale = headScale;
    if (headRatio !== undefined) this.headRatio = headRatio;
    // P0 §7.5: canais esparsos só existem para a malha canônica do núcleo.
    if (preset !== "mannequin") {
      this.gpuMorphActive = false;
    } else if (this.coreGeometry && this.morphBindGroup && this.morphPipeline) {
      this.gpuMorphActive = true;
      this.morphVertexCount = this.coreGeometry.vertexCount;
    }
    this.buildGeometryBuffers();
  }

  public setHeadProportions(headScale: number, headRatio: number) {
    this.setProportions({ headScale, headRatio });
  }

  public setProportions(params: {
    headScale?: number;
    headRatio?: number;
    shoulderWidth?: number;
    legLength?: number;
    armLength?: number;
    neckLength?: number;
    torsoLength?: number;
    heightOverall?: number;
  }) {
    if (params.headScale !== undefined) this.headScale = sanitizeFinite(params.headScale, this.headScale);
    if (params.headRatio !== undefined) this.headRatio = sanitizeFinite(params.headRatio, this.headRatio);
    if (params.shoulderWidth !== undefined) this.shoulderWidth = sanitizeFinite(params.shoulderWidth, this.shoulderWidth);
    if (params.legLength !== undefined) this.legLength = sanitizeFinite(params.legLength, this.legLength);
    if (params.armLength !== undefined) this.armLength = sanitizeFinite(params.armLength, this.armLength);
    if (params.neckLength !== undefined) this.neckLength = sanitizeFinite(params.neckLength, this.neckLength);
    if (params.torsoLength !== undefined) this.torsoLength = sanitizeFinite(params.torsoLength, this.torsoLength);
    if (params.heightOverall !== undefined) this.heightOverall = sanitizeFinite(params.heightOverall, this.heightOverall);

    // P0 §7.4: proporções entram na malha base do núcleo (`prepare_base_mesh`);
    // o viewport não as aplica — pede o snapshot e reusa os deltas canônicos.
    this.requestCoreGeometry("proporções");
  }

  public dispatchSparseMorphs(encoder: GPUCommandEncoder, vertexCount: number): boolean {
    if (!this.morphPipeline || !this.morphBindGroup) return false;
    const computePass = encoder.beginComputePass({ label: "Sparse Morph Compute Pass" });
    computePass.setPipeline(this.morphPipeline);
    computePass.setBindGroup(0, this.morphBindGroup);
    const workgroups = Math.ceil(vertexCount / 64);
    computePass.dispatchWorkgroups(workgroups, 1, 1);
    computePass.end();
    return true;
  }

  public uploadSparseMorphData(
    header: { activeChannels: number; totalVertices: number; totalDeltas: number },
    baseVertices: Float32Array,
    deltas: Float32Array,
    channels: Float32Array
  ) {
    if (!this.device || !this.morphPipeline || !this.morphBindGroupLayout) return;

    this.morphHeaderBuffer?.destroy();
    this.morphBaseBuffer?.destroy();
    this.morphDeltasBuffer?.destroy();
    this.morphChannelsBuffer?.destroy();
    this.morphedVertexBuffer?.destroy();

    this.morphVertexCount = header.totalVertices;

    const headerData = new Uint32Array([
      header.activeChannels,
      header.totalVertices,
      header.totalDeltas,
      0,
    ]);
    this.morphHeaderBuffer = this.device.createBuffer({
      size: 16,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });
    this.device.queue.writeBuffer(this.morphHeaderBuffer, 0, headerData);

    this.morphBaseBuffer = this.device.createBuffer({
      size: baseVertices.byteLength,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST,
    });
    this.device.queue.writeBuffer(this.morphBaseBuffer, 0, baseVertices);

    const deltasData = deltas.byteLength > 0 ? deltas : new Float32Array(8);
    this.morphDeltasBuffer = this.device.createBuffer({
      size: deltasData.byteLength,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST,
    });
    this.device.queue.writeBuffer(this.morphDeltasBuffer, 0, deltasData);

    const channelsData = channels.byteLength > 0 ? channels : new Float32Array(4);
    this.morphChannelsBuffer = this.device.createBuffer({
      size: channelsData.byteLength,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.COPY_DST,
    });
    this.device.queue.writeBuffer(this.morphChannelsBuffer, 0, channelsData);

    this.morphedVertexBuffer = this.device.createBuffer({
      size: baseVertices.byteLength,
      usage: GPUBufferUsage.STORAGE | GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_SRC,
    });

    this.morphBindGroup = this.device.createBindGroup({
      layout: this.morphBindGroupLayout,
      entries: [
        { binding: 0, resource: { buffer: this.morphHeaderBuffer } },
        { binding: 1, resource: { buffer: this.morphBaseBuffer } },
        { binding: 2, resource: { buffer: this.morphDeltasBuffer } },
        { binding: 3, resource: { buffer: this.morphChannelsBuffer } },
        { binding: 4, resource: { buffer: this.morphedVertexBuffer } },
      ],
    });
  }

  // ------------------------------------------------------------------
  // P0 §7.4/§7.5: o compute canônico aplica deltas do núcleo
  // ------------------------------------------------------------------

  /**
   * Registra um diagnóstico do renderer.
   *
   * Todo erro tratado no caminho crítico passa por aqui: código estável +
   * contexto, agregado por repetição, mais um log legível (o `console.warn`
   * sozinho não é observável pela UI nem pela telemetria).
   */
  private report(
    code: DiagnosticCode,
    message: string,
    options: { detail?: string; context?: Record<string, string | number | boolean> } = {}
  ): void {
    try {
      const diagnostic = this.diagnostics.report(code, message, options);
      if (diagnostic.count === 1) console.warn(RendererDiagnostics.format(diagnostic));
      this.onDiagnostic?.(diagnostic);
    } catch (error) {
      console.error("[ANIGO] diagnostics dispatcher failed:", error);
    }
  }

  /** Reporta um diagnóstico originado fora do renderer (mesmo canal/contrato). */
  public reportDiagnostic(
    code: DiagnosticCode,
    message: string,
    options: { detail?: string; context?: Record<string, string | number | boolean> } = {}
  ): void {
    this.report(code, message, options);
  }

  /** Diagnósticos acumulados (status bar, telemetria, testes). */
  public getDiagnostics(): { summary: DiagnosticsSummary; entries: readonly RenderDiagnostic[] } {
    return { summary: this.diagnostics.summary(), entries: this.diagnostics.entries };
  }

  /** Limpa o histórico de diagnósticos (após o usuário reconhecer o aviso). */
  public clearDiagnostics(): void {
    this.diagnostics.clear();
  }

  /**
   * Cobertura autorais do núcleo (não há mais "explicito" do lado TypeScript:
   * toda geometria de morph vem do núcleo).
   */
  public getMorphCoverage(): {
    implemented: number;
    total: number;
    explicit: number;
    authority: string;
    coverageComplete: boolean;
  } {
    const total = this.coreCoverage?.total_sliders ?? CANONICAL_SLIDERS.length;
    const implemented = this.coreCoverage?.sliders_with_geometry ?? 0;
    return {
      implemented,
      total,
      explicit: implemented,
      authority: this.coreAuthority,
      coverageComplete: this.coreCoverage !== null && this.coreAuthority === "core",
    };
  }

  /** Revisão da geometria canônica em uso (0 = nenhuma). */
  public get core_static_revision(): number {
    return this.coreStaticRevision;
  }

  /** Identidade autoridade/degradado do viewport (telemetria + status bar). */
  public getDeformationAuthority(): {
    authority: string;
    degraded: boolean;
    coreStaticRevision: number;
    coreDynamicRevision: number;
    channels: number;
    vertexCount: number;
  } {
    return {
      authority: this.coreAuthority,
      degraded: this.coreAuthority !== "core",
      coreStaticRevision: this.coreStaticRevision,
      coreDynamicRevision: this.coreDynamicRevision,
      channels: this.coreGeometry?.channels.length ?? 0,
      vertexCount: this.coreGeometry?.vertexCount ?? 0,
    };
  }
  /** Pede um snapshot ao shell (uma vez por necessidade, nunca por frame). */
  private requestCoreGeometry(reason: string): void {
    this.coreGeometryRequested = true;
    console.info(`[ANIGO 3D] core snapshot requested (${reason})`);
    try {
      this.onCoreGeometryRequired?.();
    } catch (e) {
      this.report("unexpected_error", "o shell não conseguiu atender o pedido de geometria canônica", {
        detail: e instanceof Error ? e.message : String(e),
      });
    }
  }

  /** Pesos efetivos: autorais do núcleo + adiantamento local dentro da janela. */
  private effectiveChannelWeights(): Map<string, number> {
    if (this.liveWeights.size === 0) return this.channelWeights;
    if (performance.now() > this.liveWeightsExpiry) {
      this.liveWeights.clear();
      return this.channelWeights;
    }
    const merged = new Map(this.channelWeights);
    for (const [id, weight] of this.liveWeights) merged.set(id, weight);
    return merged;
  }

  /**
   * Atualiza o peso de um canal canônico.
   *
   * Os deltas são sempre do núcleo; aqui só se escolhe *quanto* deles aplicar —
   * é isso que permite o slider responder a 60 fps sem uma ida-e-volta ao
   * núcleo por frame (o snapshot confirma o valor logo em seguida).
   */
  public setChannelWeight(sliderId: string, weight: number): void {
    if (!Number.isFinite(weight)) return;
    if (!this.coreGeometry?.channels.some((channel) => channel.sliderId === sliderId)) {
      this.report("channel_missing", "slider sem canal canônico no snapshot atual", {
        context: { slider: sliderId },
      });
      this.requestCoreGeometry(`canal canônico ausente: ${sliderId}`);
      return;
    }
    this.liveWeights.set(sliderId, weight);
    this.liveWeightsExpiry = performance.now() + LIVE_WEIGHT_WINDOW_MS;
    this.gpuMorphDirty = true;
  }

  /**
   * P0 §7.5 — o viewport consome o snapshot canônico.
   *
   * O núcleo é a autoridade: a malha base (gênero + proporções já embutidos), os
   * deltas esparsos e os pesos chegam prontos. O renderer apenas:
   * 1. sobe vértices/índices/deltas/canais para a GPU;
   * 2. deixa o compute canônico (`morph_sparse_compute.wgsl`) aplicar os pesos;
   * 3. descarta qualquer adiantamento local de peso (a autoridade voltou a falar).
   */
  public applyCoreSnapshot(delivery: CoreSnapshotDelivery): {
    applied: boolean;
    geometryUploaded: boolean;
    channelCount: number;
    vertexCount: number;
    authority: string;
    staticRevision: number;
    dynamicRevision: number;
  } {
    this.coreGeometryRequested = false;
    this.coreAuthority = delivery.authority;
    this.coreCoverage = delivery.coverage;
    this.coreStaticRevision = Math.round(delivery.staticRevision);
    this.coreDynamicRevision = Math.round(delivery.dynamicRevision);
    const background = delivery.state?.render?.background_color;
    if (Array.isArray(background) && background.length === 4 && background.every((c) => Number.isFinite(c))) {
      this.clearColor = [background[0], background[1], background[2], background[3]];
    }
    this.liveWeights.clear();

    let geometryUploaded = false;
    let incoming: ViewportGeometry | null = null;
    if (delivery.geometry) {
      incoming = viewportGeometryFromDecoded(delivery.geometry, delivery.morphWeights, {
        staticRevision: delivery.staticRevision,
        morphValues: delivery.morphValues,
      });
    }

    const domain = (incoming ?? this.coreGeometry)?.channels ?? [];
    const weights = new Map<string, number>();
    for (const channel of domain) {
      weights.set(channel.sliderId, delivery.morphWeights.get(channel.sliderId) ?? 0);
    }
    this.channelWeights = weights;

    if (incoming) {
      // P1-03: nenhum buffer é criado a partir de geometria reprovada.
      const validation = validateRenderableMesh(incoming.vertices, incoming.indices);
      if (!validation.ok) {
        this.report("geometry_rejected", `geometria canônica reprovada: ${validation.message}`, {
          context: { code: validation.code ?? "unknown", mesh_uri: incoming.meshUri },
        });
        return {
          applied: false,
          geometryUploaded: false,
          channelCount: this.coreGeometry?.channels.length ?? 0,
          vertexCount: this.coreGeometry?.vertexCount ?? 0,
          authority: this.coreAuthority,
          staticRevision: this.coreStaticRevision,
          dynamicRevision: this.coreDynamicRevision,
        };
      }
    }

    if (incoming) {
      const previous = this.coreGeometry ? geometrySignature(this.coreGeometry) : null;
      const next = geometrySignature(incoming);
      this.coreGeometry = incoming;
      this.currentPreset = "mannequin";
      if (previous !== next || !this.gpuMorphActive) {
        this.uploadCoreGeometry(incoming);
        geometryUploaded = true;
      } else {
        this.gpuMorphDirty = true;
      }
    } else if (this.coreGeometry) {
      // Só a parte dinâmica mudou: pesos novos, mesmos buffers.
      this.gpuMorphDirty = true;
    }

    return {
      applied: true,
      geometryUploaded,
      channelCount: this.coreGeometry?.channels.length ?? 0,
      vertexCount: this.coreGeometry?.vertexCount ?? 0,
      authority: this.coreAuthority,
      staticRevision: this.coreStaticRevision,
      dynamicRevision: this.coreDynamicRevision,
    };
  }

  /**
   * Sobe a geometria canônica na GPU: VBO/IBO de base + canais e deltas do
   * compute. Nenhum byte é recalculado no TypeScript.
   */
  private uploadCoreGeometry(geometry: ViewportGeometry): void {
    const t0 = performance.now();
    this.canonicalVertices = geometry.vertices;
    this.canonicalIndices = geometry.indices;
    this.morphVertexCount = geometry.vertexCount;
    this.buildGeometryBuffers();
    this.uploadSparseMorphData(
      {
        activeChannels: geometry.channels.length,
        totalVertices: geometry.vertexCount,
        totalDeltas: geometry.totalDeltas,
      },
      geometry.vertices,
      geometry.deltas,
      packChannelRecordsWithWeights(geometry.channels, this.effectiveChannelWeights())
    );
    this.gpuMorphActive = this.morphBindGroup !== null && this.morphPipeline !== null;
    this.gpuMorphDirty = false;
    const vramKB =
      (geometry.vertices.byteLength + geometry.deltas.byteLength + geometry.channels.length * 16) / 1024;
    console.info(
      `[ANIGO 3D] core snapshot geometry: ${geometry.vertexCount} verts, ` +
        `${geometry.channels.length} canais, ${geometry.totalDeltas} deltas, ~${vramKB.toFixed(0)} KB ` +
        `(${geometry.meshUri}, static rev ${geometry.staticRevision}) em ${(performance.now() - t0).toFixed(0)} ms`
    );
  }

  /**
   * Modo degradado (browser sem núcleo): desenha a malha base **sem deformação**
   * e anuncia a autoridade indisponível. Não existe caminho de deformação TS.
   */
  public applyBaseMesh(vertices: Float32Array, indices: Uint32Array, meshUri: string): void {
    this.coreGeometry = null;
    this.coreAuthority = "unavailable";
    this.canonicalVertices = vertices;
    this.canonicalIndices = indices;
    this.morphVertexCount = 0;
    this.gpuMorphActive = false;
    this.liveWeights.clear();
    this.channelWeights.clear();
    this.buildGeometryBuffers();
    this.report("geometry_unavailable", "modo degradado: malha base sem deformação canônica", {
      context: { mesh_uri: meshUri },
    });
  }

  /**
   * Somatotipo canônico (P0 §7.4): o mapeamento para os macro sliders e os
   * deltas são do núcleo — o TypeScript só guarda o valor e pede o snapshot.
   */
  public setSomatotype(endo: number, meso: number, ecto: number) {
    // P0-02: sanitize at the boundary (NaN/Inf can never enter the engine).
    const e = clampNumber(sanitizeFinite(endo, this.somatotypeEndo), 0, 1);
    const m = clampNumber(sanitizeFinite(meso, this.somatotypeMeso), 0, 1);
    const c = clampNumber(sanitizeFinite(ecto, this.somatotypeEcto), 0, 1);
    if (e === this.somatotypeEndo && m === this.somatotypeMeso && c === this.somatotypeEcto) return;
    this.somatotypeEndo = e;
    this.somatotypeMeso = m;
    this.somatotypeEcto = c;
    this.requestCoreGeometry("somatotype");
  }

  /** Dimorfismo contínuo: entra na malha base do núcleo (não é peso de morph). */
  public setGenderDimorphism(gender: number) {
    // P0-02: NaN guard — genderDimorphism can never become NaN again.
    const g = clampNumber(sanitizeFinite(gender, this.genderDimorphism), 0.0, 1.0);
    if (g === this.genderDimorphism) return;
    this.genderDimorphism = g;
    this.requestCoreGeometry("gender dimorphism");
  }

  /**
   * P0 §7.4 — mudança de um slider de morph.
   *
   * O valor é validado/clampado aqui (interação), mas a geometria vem do núcleo:
   * se existe canal canônico para o slider, o peso é atualizado na GPU (deltas
   * do núcleo); se não existe, o viewport pede um snapshot novo em vez de
   * inventar geometria.
   */
  public setMorphSlider(name: string, weight: number) {
    // P0-09: unknown ids are rejected loudly (no orphan state); known ids
    // are catalog-clamped on this write path like every other.
    if (!isKnownSliderId(name)) {
      console.warn(`[ANIGO 3D] setMorphSlider: unknown slider "${name}" ignored`);
      return;
    }
    const clamped = clampCatalog(name, weight);
    if (clamped === null) return;
    this.activeMorphWeights.set(name, clamped);
    const def = getSliderDef(name);
    this.setChannelWeight(name, channelWeightOf(clamped, def ? def.defaultValue : 0));
  }

  /** Peso de canal autorais do núcleo (0 quando o slider está no default). */
  public getChannelWeight(sliderId: string): number {
    return this.channelWeights.get(sliderId) ?? 0;
  }

  public setLight(
    dir: [number, number, number],
    intensity: number,
    shadowColor?: [number, number, number],
    lightColor?: [number, number, number],
    ambientIntensity?: number,
    shadowSaturation?: number,
    skyColor?: [number, number, number],
    groundColor?: [number, number, number]
  ) {
    this.lightDir = dir;
    this.lightIntensity = intensity;
    if (shadowColor) this.shadowColor = shadowColor;
    if (lightColor) this.lightColor = lightColor;
    if (ambientIntensity !== undefined) this.ambientIntensity = ambientIntensity;
    if (shadowSaturation !== undefined) this.shadowSaturation = shadowSaturation;
    if (skyColor) this.ambientSky = skyColor;
    if (groundColor) this.ambientGround = groundColor;
  }

  public setOutlineWidth(width: number) {
    this.outlineWidth = width > 0.05 ? width * 0.001 : width;
  }

  public setOutlineColor(color: [number, number, number, number]) {
    this.outlineColor = color;
  }

  public setShadowThreshold(threshold: number) {
    this.shadowThreshold = threshold;
  }

  public setToonSmoothness(smoothness: number) {
    this.toonSmoothness = smoothness;
  }

  public setSpecular(intensity: number, exponent: number) {
    this.specIntensity = intensity;
    this.specExponent = exponent;
  }

  public setRimLight(intensity: number, spread: number, color?: [number, number, number]) {
    this.rimIntensity = intensity;
    this.rimSpread = spread;
    if (color) this.rimColor = color;
  }

  public setRimColor(color: [number, number, number]) {
    this.rimColor = color;
  }

  public setHueShift(degrees: number) {
    this.hueShift = degrees;
  }

  public setToonSteps(steps: number) {
    this.toonSteps = steps;
  }

  public setMaterialParams(params: {
    baseColor?: [number, number, number, number];
    shadeColor?: [number, number, number, number];
    shadowThreshold?: number;
    toonSmoothness?: number;
    specIntensity?: number;
    specExponent?: number;
    specSoftness?: number;
    specOffset?: number;
    specColor?: [number, number, number, number];
    rimIntensity?: number;
    rimSpread?: number;
    rimColor?: [number, number, number, number];
    hueShift?: number;
    toonSteps?: number;
    outlineWidth?: number;
    outlineColor?: [number, number, number, number];
    outlineOpacity?: number;
    outlineSmoothness?: number;
    outlineDepthBias?: number;
    shadowSaturation?: number;
    specularSize?: number; // P2-07
    aoIntensity?: number; // P2-05
  }) {
    if (params.baseColor) this.baseColor = params.baseColor;
    if (params.shadeColor) this.shadeColor = params.shadeColor;
    if (params.shadowThreshold !== undefined) this.shadowThreshold = params.shadowThreshold;
    if (params.toonSmoothness !== undefined) this.toonSmoothness = params.toonSmoothness;
    if (params.specIntensity !== undefined) this.specIntensity = params.specIntensity;
    if (params.specExponent !== undefined) this.specExponent = params.specExponent;
    if (params.specSoftness !== undefined) this.specSoftness = params.specSoftness;
    if (params.specOffset !== undefined) this.specOffset = params.specOffset;
    if (params.specColor) this.specColor = params.specColor;
    if (params.rimIntensity !== undefined) this.rimIntensity = params.rimIntensity;
    if (params.rimSpread !== undefined) this.rimSpread = params.rimSpread;
    if (params.rimColor) this.rimColor = [params.rimColor[0], params.rimColor[1], params.rimColor[2]];
    if (params.hueShift !== undefined) this.hueShift = params.hueShift;
    if (params.toonSteps !== undefined) this.toonSteps = params.toonSteps;
    if (params.outlineWidth !== undefined) this.outlineWidth = params.outlineWidth;
    if (params.outlineColor) this.outlineColor = params.outlineColor;
    if (params.outlineOpacity !== undefined) this.outlineOpacity = params.outlineOpacity;
    if (params.outlineSmoothness !== undefined) this.outlineSmoothness = params.outlineSmoothness;
    if (params.outlineDepthBias !== undefined) this.outlineDepthBias = params.outlineDepthBias;
    if (params.shadowSaturation !== undefined) this.shadowSaturation = params.shadowSaturation;
    if (params.specularSize !== undefined) this.specularSize = params.specularSize;
    if (params.aoIntensity !== undefined) this.aoIntensity = params.aoIntensity;
  }

  private buildGeometryBuffers() {
    let data: { vertices: Float32Array; indices: Uint32Array };

    switch (this.currentPreset) {
      case "sphere":
        data = this.generateSphereData(0.85, 36, 72);
        break;
      case "cube":
        data = this.generateCubeData(1.2);
        break;
      case "mannequin":
      default:
        data = this.generateMannequinData(this.headScale, this.headRatio);
        break;
    }

    this.indexCount = data.indices.length;

    // WebGPU Buffers — P0-05: persistent pair, recreated ONLY on growth.
    if (this.device) {
      if (!this.vertexBuffer || data.vertices.byteLength > this.vertexCapacityBytes) {
        this.vertexBuffer?.destroy();
        // 1.5x headroom + 256B alignment to absorb small topology changes.
        this.vertexCapacityBytes = Math.ceil(data.vertices.byteLength * 1.5 / 256) * 256;
        this.vertexBuffer = this.device.createBuffer({
          size: this.vertexCapacityBytes,
          usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
        });
      }
      this.device.queue.writeBuffer(this.vertexBuffer, 0, data.vertices);

      if (!this.indexBuffer || data.indices.byteLength > this.indexCapacityBytes) {
        this.indexBuffer?.destroy();
        this.indexCapacityBytes = Math.ceil(data.indices.byteLength * 1.5 / 256) * 256;
        this.indexBuffer = this.device.createBuffer({
          size: this.indexCapacityBytes,
          usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST,
        });
      }
      this.device.queue.writeBuffer(this.indexBuffer, 0, data.indices);
    }

    // WebGL2 Buffers
    if (this.gl) {
      const gl = this.gl;
      if (!this.glVao) this.glVao = gl.createVertexArray();
      gl.bindVertexArray(this.glVao);

      if (!this.glVbo) this.glVbo = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, this.glVbo);
      gl.bufferData(gl.ARRAY_BUFFER, data.vertices, gl.STATIC_DRAW);

      if (!this.glIbo) this.glIbo = gl.createBuffer();
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.glIbo);
      gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, data.indices, gl.STATIC_DRAW);

      const stride = 72;
      // location 0: pos (3 floats)
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 3, gl.FLOAT, false, stride, 0);
      // location 1: normal (3 floats)
      gl.enableVertexAttribArray(1);
      gl.vertexAttribPointer(1, 3, gl.FLOAT, false, stride, 12);
      // location 2: uv (2 floats)
      gl.enableVertexAttribArray(2);
      gl.vertexAttribPointer(2, 2, gl.FLOAT, false, stride, 24);
      // location 3: color (4 floats)
      gl.enableVertexAttribArray(3);
      gl.vertexAttribPointer(3, 4, gl.FLOAT, false, stride, 32);
      // location 4: joints (4 u16)
      gl.enableVertexAttribArray(4);
      gl.vertexAttribIPointer(4, 4, gl.UNSIGNED_SHORT, stride, 48);
      // location 5: weights (4 floats)
      gl.enableVertexAttribArray(5);
      gl.vertexAttribPointer(5, 4, gl.FLOAT, false, stride, 56);

      gl.bindVertexArray(null);
    }
  }

  private appendCube(
    vertices: VertexData[],
    rawIndices: number[],
    center: [number, number, number],
    dims: [number, number, number],
    color: [number, number, number, number],
    smoothNormals: boolean = false
  ) {
    const cubeFaces = [
      { normal: [0, 0, 1], corners: [[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]] },
      { normal: [0, 0, -1], corners: [[0.5, -0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5]] },
      { normal: [0, 1, 0], corners: [[-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5]] },
      { normal: [0, -1, 0], corners: [[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [-0.5, -0.5, 0.5]] },
      { normal: [1, 0, 0], corners: [[0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5]] },
      { normal: [-1, 0, 0], corners: [[-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, -0.5]] },
    ];

    for (const face of cubeFaces) {
      const baseIdx = vertices.length;
      for (const pt of face.corners) {
        let norm: [number, number, number];
        if (smoothNormals) {
          const rx = pt[0];
          const ry = pt[1] * 0.25;
          const rz = pt[2];
          const len = Math.hypot(rx, ry, rz) || 1.0;
          norm = [rx / len, ry / len, rz / len];
        } else {
          norm = [face.normal[0], face.normal[1], face.normal[2]];
        }
        vertices.push({
          pos: [
            center[0] + pt[0] * dims[0],
            center[1] + pt[1] * dims[1],
            center[2] + pt[2] * dims[2],
          ],
          normal: norm,
          uv: [0.0, 0.0],
          color,
          joints: [0, 0, 0, 0],
          weights: [1.0, 0.0, 0.0, 0.0],
        });
      }
      rawIndices.push(baseIdx, baseIdx + 1, baseIdx + 2, baseIdx, baseIdx + 2, baseIdx + 3);
    }
  }

  private generateCubeData(size: number): { vertices: Float32Array; indices: Uint32Array } {
    // P2-13 if (this.canonicalExtras?.anigo_zones) use normalized anchors else fallback height-scaled magic indices
    const vertices: VertexData[] = [];
    const rawIndices: number[] = [];

    // Main Cube centered at y = 1.0
    this.appendCube(vertices, rawIndices, [0.0, 1.0, 0.0], [size, size, size], [1.0, 0.50, 1.0, 1.0]);
    // Studio Turntable Pedestal at ground
    this.appendCube(vertices, rawIndices, [0.0, -0.02, 0.0], [1.6, 0.04, 1.6], [0.7, 0.50, 0.0, 0.3]);

    return {
      vertices: packVertices(vertices),
      indices: new Uint32Array(rawIndices),
    };
  }

  private generateSphereData(radius: number, rings: number, sectors: number): { vertices: Float32Array; indices: Uint32Array } {
    // P2-13 if (this.canonicalExtras?.anigo_zones) use normalized anchors else fallback height-scaled magic indices
    const vertices: VertexData[] = [];
    const rawIndices: number[] = [];

    for (let r = 0; r <= rings; r++) {
      const v = r / rings;
      const theta = v * Math.PI;

      for (let s = 0; s <= sectors; s++) {
        const u = s / sectors;
        const phi = u * 2 * Math.PI;

        const x = Math.sin(theta) * Math.cos(phi);
        const y = Math.cos(theta);
        const z = Math.sin(theta) * Math.sin(phi);

        // Position sphere at y = 1.0 (camera focus height)
        vertices.push({
          pos: [x * radius, y * radius + 1.0, z * radius],
          normal: [x, y, z],
          uv: [u, v],
          color: [1.0, 0.5, 1.0, 1.0],
          joints: [0, 0, 0, 0],
          weights: [1.0, 0.0, 0.0, 0.0],
        });
      }
    }

    for (let r = 0; r < rings; r++) {
      for (let s = 0; s < sectors; s++) {
        const first = r * (sectors + 1) + s;
        const second = first + sectors + 1;

        rawIndices.push(first, first + 1, second);
        rawIndices.push(second, first + 1, second + 1);
      }
    }

    // Studio Turntable Pedestal
    this.appendCube(vertices, rawIndices, [0.0, -0.02, 0.0], [1.6, 0.04, 1.6], [0.7, 0.50, 0.0, 0.3]);

    return {
      vertices: packVertices(vertices),
      indices: new Uint32Array(rawIndices),
    };
  }

  /**
   * P0 §7.4/§7.5 — geometria do preset canônico.
   *
   * Toda a deformação foi removida do TypeScript: aqui só existem (a) a malha
   * canônica que o núcleo mandou no snapshot e (b) a malha base crua do modo
   * degradado (browser sem núcleo, sem morphs). Não há caminho de deltas,
   * normais ou proporções calculado no cliente.
   */
  private generateMannequinData(headScale: number = 1.0, headRatio: number = 6.5): { vertices: Float32Array; indices: Uint32Array } {
    if (this.canonicalVertices && this.canonicalIndices) {
      return { vertices: this.canonicalVertices, indices: this.canonicalIndices };
    }
    // Fallback to cube if not loaded yet
    return this.generateCubeData(1.0);
  }

  public async loadCanonicalModel(gender: "male" | "female") {
    // P0 §7.5: com geometria canônica do núcleo não existe malha base local a
    // carregar — o snapshot já traz a malha preparada (gênero/proporções nela).
    if (this.coreGeometry) {
      this.requestCoreGeometry("malha base ignorada (núcleo ativo)");
      return;
    }
    const url = `/models/anigo_base_${gender}.glb`;
    try {
      // P2-12 abort previous load, use cache, report progress
      if (this.loadAbortController) this.loadAbortController.abort();
      this.loadAbortController = new AbortController();
      if (this.canonicalModelCache.has(gender)) {
        const cached = this.canonicalModelCache.get(gender)!;
        this.canonicalBaseVertices = cached.vertices;
        this.canonicalGender = gender;
        // P0 §7.5: malha base crua (degradado) — a geometria autorizada vem do
        // snapshot do núcleo, pedido logo abaixo.
        this.applyBaseMesh(packVertices(cached.vertices), cached.indices, url);
        this.requestCoreGeometry("modelo base em cache");
        return;
      }
      const resp = await fetch(url, { signal: this.loadAbortController.signal });
      if (!resp.ok) throw new Error(`[P2-12] GLB fetch failed ${resp.status} ${url}`); // P2-12
      const buffer = await resp.arrayBuffer();
      // P0-10: validating multi-primitive loader (magic/version/chunks checked,
      // quantized attributes, node transforms, skins preserved — see gltf_loader).
      let decoded;
      try {
        decoded = loadGlbMesh(buffer);
      } catch (e) {
        if (e instanceof GlbParseError) {
          throw new Error(`Modelo "${gender}" inválido (${url}): ${e.message}`);
        }
        throw e;
      }
      for (const w of decoded.warnings) {
        console.warn(`[ANIGO 3D] GLB ${gender}: ${w}`);
      }
      const mesh = decoded.mesh;
      // ANIME shading channel neutral: G is a bias centred at 0.5.
      const ANIME_ATTR_NEUTRAL: [number, number, number, number] = [1.0, 0.5, 1.0, 1.0];
      const vertexCount = mesh.positions.length / 3;
      console.info(
        `[ANIGO 3D] GLB ${gender}: ${vertexCount} verts, ${mesh.indices.length} idx, ` +
          `${mesh.primitiveCount} primitive(s)${mesh.joints ? ", skinned" : ", rigid"}`
      );

      const vertices: VertexData[] = new Array(vertexCount);
      for (let i = 0; i < vertexCount; i++) {
        let r = ANIME_ATTR_NEUTRAL[0];
        let g = ANIME_ATTR_NEUTRAL[1];
        let b = ANIME_ATTR_NEUTRAL[2];
        let a = ANIME_ATTR_NEUTRAL[3];
        if (mesh.colors) {
          if (mesh.colorComps === 4) {
            r = mesh.colors[i * 4];
            g = mesh.colors[i * 4 + 1];
            b = mesh.colors[i * 4 + 2];
            a = mesh.colors[i * 4 + 3];
          } else {
            r = mesh.colors[i * 3];
            g = mesh.colors[i * 3 + 1];
            b = mesh.colors[i * 3 + 2];
          }
        }
        vertices[i] = {
          pos: [mesh.positions[i * 3], mesh.positions[i * 3 + 1], mesh.positions[i * 3 + 2]],
          normal: [mesh.normals[i * 3], mesh.normals[i * 3 + 1], mesh.normals[i * 3 + 2]],
          uv: [mesh.uvs[i * 2], mesh.uvs[i * 2 + 1]],
          color: [r, g, b, a],
          joints:
            mesh.joints !== null
              ? [mesh.joints[i * 4], mesh.joints[i * 4 + 1], mesh.joints[i * 4 + 2], mesh.joints[i * 4 + 3]]
              : [0, 0, 0, 0],
          weights:
            mesh.weights !== null
              ? [mesh.weights[i * 4], mesh.weights[i * 4 + 1], mesh.weights[i * 4 + 2], mesh.weights[i * 4 + 3]]
              : [1, 0, 0, 0],
        };
      }

      const rawIndices: number[] = Array.from(mesh.indices);

      // P1-03: o GLB é validado **antes** de virar buffer (stride, índices,
      // NaN). Reprovar aqui é barato; descobrir isso na GPU não é.
      const validation = validateAttributeMesh(mesh.positions, rawIndices, {
        normals: mesh.normals,
        uvs: mesh.uvs,
      });
      if (!validation.ok) {
        this.report("mesh_invalid", `GLB reprovado antes de criar buffers: ${validation.message}`, {
          context: { code: validation.code ?? "unknown", gender },
        });
        throw new MeshValidationError(validation.code ?? "EMPTY_MESH", validation.message);
      }

      this.canonicalBaseVertices = vertices;
      this.canonicalIndices = new Uint32Array(rawIndices);
      this.canonicalModelCache.set(gender, { vertices, indices: new Uint32Array(rawIndices) });
      this.canonicalGender = gender;
      this.canonicalVertices = packVertices(vertices);

      this.currentPreset = "mannequin";
      // P0 §7.5: esta malha base é o modo degradado (sem deformação). Em
      // produção a geometria vem do snapshot do núcleo; o GLB acabou de ser
      // substituído pelo que o núcleo mandar.
      this.applyBaseMesh(packVertices(vertices), new Uint32Array(rawIndices), url);
    } catch (e) {
      // P0-10: failures propagate (caller + UI callback) — never silent cube.
      if (e instanceof DOMException && e.name === "AbortError") return;
      if (!(e instanceof MeshValidationError)) {
        this.report("model_load_failed", "falha ao carregar a malha base (GLB)", {
          detail: e instanceof Error ? e.message : String(e),
          context: { gender },
        });
      }
      this.onModelLoadError?.(e instanceof Error ? e.message : String(e));
      throw e;
    }
  }

  private buildUniformBuffers() {
    if (!this.device || !this.celPipeline || !this.outlinePipeline) return;

    // Tamanhos dos blocos vêm do contrato (mesmos `#[repr(C)]` do Rust)
    this.cameraBuffer = this.device.createBuffer({
      size: uniformSize("camera"),
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    this.lightBuffer = this.device.createBuffer({
      size: uniformSize("light"),
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    this.materialBuffer = this.device.createBuffer({
      size: uniformSize("material"),
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    this.outlineBuffer = this.device.createBuffer({
      size: uniformSize("outline"),
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    // 256x4 2D Toon Ramp Texture
    // Row 0: Continuous ramp, Row 1: 1-step harsh anime cel, Row 2: 2-step Ghibli penumbra, Row 3: 3-step high-key
    if (this.toonRampTexture) {
      try { this.toonRampTexture.destroy(); } catch (_) {}
    }
    const rampSpec = RENDER_CONTRACT.toon_ramp;
    this.toonRampTexture = this.device.createTexture({
      size: [rampSpec.width, rampSpec.height],
      format: rampSpec.format as any,
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
    });

    // Bytes do ramp gerados a partir das linhas do contrato; a impressão digital
    // congela a textura que o headless (Rust) também precisa produzir.
    const rampData = toonRampBytes();
    const rampFingerprint = toonRampFingerprint(rampData);
    if (rampFingerprint !== expectedToonRampFingerprint()) {
      throw new Error(
        `toon ramp divergiu do contrato (esperado ${expectedToonRampFingerprint()}, encontrado ${rampFingerprint})`
      );
    }
    this.device.queue.writeTexture(
      { texture: this.toonRampTexture },
      rampData,
      { bytesPerRow: rampSpec.width * 4, rowsPerImage: rampSpec.height },
      [rampSpec.width, rampSpec.height]
    );

    this.toonRampSampler = this.device.createSampler({
      magFilter: filterMode(rampSpec.mag_filter),
      minFilter: filterMode(rampSpec.min_filter),
      addressModeU: addressMode(rampSpec.address_mode),
      addressModeV: addressMode(rampSpec.address_mode),
    });

    this.celBindGroup = this.device.createBindGroup({
      layout: this.celPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: this.cameraBuffer } },
        { binding: 1, resource: { buffer: this.lightBuffer } },
        { binding: 2, resource: { buffer: this.materialBuffer } },
        { binding: 3, resource: this.toonRampTexture.createView() },
        { binding: 4, resource: this.toonRampSampler },
      ],
    });

    this.outlineBindGroup = this.device.createBindGroup({
      layout: this.outlinePipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: this.cameraBuffer } },
        { binding: 1, resource: { buffer: this.outlineBuffer } },
      ],
    });
  }

  public resize(cssWidth: number, cssHeight: number) {
    if (cssWidth <= 0 || cssHeight <= 0) return;
    this.cssWidth = cssWidth;
    this.cssHeight = cssHeight;

    const realWidth = Math.max(Math.round(cssWidth * this.dpiMultiplier), 64);
    const realHeight = Math.max(Math.round(cssHeight * this.dpiMultiplier), 64);

    const sizeChanged = this.canvas.width !== realWidth || this.canvas.height !== realHeight;

    if (sizeChanged || !this.depthTexture || !this.depthView) {
      this.canvas.width = realWidth;
      this.canvas.height = realHeight;

      if (this.backend === "webgpu" && this.device) {
        if (this.depthTexture) {
          try {
            this.depthTexture.destroy();
          } catch (_) {}
        }

        this.depthTexture = this.device.createTexture({
          size: [realWidth, realHeight],
          format: depthFormat() as any,
          sampleCount: this.sampleCount,
          usage: GPUTextureUsage.RENDER_ATTACHMENT,
        });
        this.depthView = this.depthTexture.createView();
        // Recreate MSAA color resolve texture
        if (this.msaaColorTexture) { try { this.msaaColorTexture.destroy(); } catch (_) {} }
        this.msaaColorTexture = this.device.createTexture({
          size: [realWidth, realHeight],
          format: this.format,
          sampleCount: this.sampleCount,
          usage: GPUTextureUsage.RENDER_ATTACHMENT,
        });
        this.msaaColorView = this.msaaColorTexture.createView();
      } else if (this.backend === "webgl2" && this.gl) {
        this.gl.viewport(0, 0, realWidth, realHeight);
      }
    }

    // Immediately render a frame so the canvas never displays a squashed or stretched bitmap
    if (!this.isRendering) {
      this.render();
    }
  }

  public setFpsCap(fps: number) {
    this.targetFps = fps;
    this.lastFrameTimestamp = performance.now(); // P2-09 reset timer
  }

  public setDpiScale(multiplier: number) {
    if (this.dpiMultiplier !== multiplier) {
      this.dpiMultiplier = multiplier;
      this.resize(this.cssWidth, this.cssHeight);
    }
  }

  public setVsync(enabled: boolean) {
    if (this.vsyncEnabled !== enabled) {
      this.vsyncEnabled = enabled;
      if (this.device && this.context) {
        try {
          this.context.configure({
            device: this.device,
            format: this.format,
            alphaMode: "premultiplied",
            presentMode: enabled ? "fifo" : "immediate", // P2-11 check caps, fallback if unsupported
          });
        } catch (e) {
          this.report("context_configure_failed", "não foi possível reconfigurar o canvas (vsync/present)", {
            detail: e instanceof Error ? e.message : String(e),
          });
        }
      }
    }
  }

  // ==========================================
  // Camera Controls
  // ==========================================
  public orbit(deltaAzimuth: number, deltaElevation: number) {
    this.recenterAnim = null;
    let v: [number, number, number] = [
      this.eye[0] - this.target[0],
      this.eye[1] - this.target[1],
      this.eye[2] - this.target[2],
    ];
    const radius = Math.hypot(v[0], v[1], v[2]);
    if (radius < 0.0001) return;

    let up: [number, number, number] = [...this.up];

    // 1. Elevation rotates around the camera's local screen-space right vector
    const z = this.normalize(v);
    let right = this.cross(up, z);
    let rLen = Math.hypot(right[0], right[1], right[2]);
    if (rLen < 1e-4) {
      const fallback: [number, number, number] = Math.abs(z[1]) < 0.9 ? [0, 1, 0] : [1, 0, 0];
      right = this.cross(fallback, z);
      rLen = Math.hypot(right[0], right[1], right[2]);
    }
    const rightAxis: [number, number, number] = [right[0] / rLen, right[1] / rLen, right[2] / rLen];

    if (Math.abs(deltaElevation) > 1e-6) {
      const elevAngle = deltaElevation;
      v = this.rotateVector(v, rightAxis, elevAngle);
      up = this.rotateVector(up, rightAxis, elevAngle);
    }

    // 2. Azimuth rotates around the world Y axis [0, 1, 0]
    if (Math.abs(deltaAzimuth) > 1e-6) {
      const worldY: [number, number, number] = [0, 1, 0];
      v = this.rotateVector(v, worldY, deltaAzimuth);
      up = this.rotateVector(up, worldY, deltaAzimuth);
    }

    // 3. Orthonormalize camera vectors to avoid numeric drift or collapse
    const zNew = this.normalize(v);
    let rightNew = this.cross(up, zNew);
    let rLenNew = Math.hypot(rightNew[0], rightNew[1], rightNew[2]);
    if (rLenNew < 1e-4) {
      const fallback: [number, number, number] = Math.abs(zNew[1]) < 0.9 ? [0, 1, 0] : [1, 0, 0];
      rightNew = this.cross(fallback, zNew);
      rLenNew = Math.hypot(rightNew[0], rightNew[1], rightNew[2]);
    }
    const rightUnit: [number, number, number] = [rightNew[0] / rLenNew, rightNew[1] / rLenNew, rightNew[2] / rLenNew];
    const upUnit = this.normalize(this.cross(zNew, rightUnit));

    this.up = [upUnit[0], upUnit[1], upUnit[2]];
    this.eye = [
      this.target[0] + v[0],
      this.target[1] + v[1],
      this.target[2] + v[2],
    ];
  }

  public zoom(factor: number, mouseNdcX: number = 0, mouseNdcY: number = 0) {
    this.recenterAnim = null;
    const v: [number, number, number] = [
      this.eye[0] - this.target[0],
      this.eye[1] - this.target[1],
      this.eye[2] - this.target[2],
    ];
    const dist = Math.hypot(v[0], v[1], v[2]);
    if (dist < 0.0001) return;

    // Clamp distance between 0.2 and 40.0 units
    const targetDist = dist * factor;
    const clampedDist = Math.max(0.2, Math.min(40.0, targetDist));
    const effectiveFactor = clampedDist / dist;
    if (Math.abs(effectiveFactor - 1.0) < 1e-6) return;

    // Camera basis vectors
    const z = this.normalize(v);
    let right = this.cross(this.up, z);
    let rLen = Math.hypot(right[0], right[1], right[2]);
    if (rLen < 1e-4) {
      const fallback: [number, number, number] = Math.abs(z[1]) < 0.9 ? [0, 1, 0] : [1, 0, 0];
      right = this.cross(fallback, z);
      rLen = Math.hypot(right[0], right[1], right[2]);
    }
    const rightUnit: [number, number, number] = [right[0] / rLen, right[1] / rLen, right[2] / rLen];
    const upUnit = this.normalize(this.cross(z, rightUnit));

    // Focal plane dimensions at target distance
    const aspect = this.canvas.width / Math.max(this.canvas.height, 1);
    const halfHeight = dist * Math.tan(this.fov * 0.5);
    const halfWidth = halfHeight * aspect;

    // World-space focal point P_mouse directly under cursor:
    // P_mouse = target + right * (mouseNdcX * halfWidth) + up * (mouseNdcY * halfHeight)
    const pMouse: [number, number, number] = [
      this.target[0] + rightUnit[0] * (mouseNdcX * halfWidth) + upUnit[0] * (mouseNdcY * halfHeight),
      this.target[1] + rightUnit[1] * (mouseNdcX * halfWidth) + upUnit[1] * (mouseNdcY * halfHeight),
      this.target[2] + rightUnit[2] * (mouseNdcX * halfWidth) + upUnit[2] * (mouseNdcY * halfHeight),
    ];

    // Shift target: newTarget = target + (P_mouse - target) * (1 - effectiveFactor)
    // Shift eye: newEye = P_mouse + (eye - P_mouse) * effectiveFactor
    this.target = [
      this.target[0] + (pMouse[0] - this.target[0]) * (1.0 - effectiveFactor),
      this.target[1] + (pMouse[1] - this.target[1]) * (1.0 - effectiveFactor),
      this.target[2] + (pMouse[2] - this.target[2]) * (1.0 - effectiveFactor),
    ];

    this.eye = [
      pMouse[0] + (this.eye[0] - pMouse[0]) * effectiveFactor,
      pMouse[1] + (this.eye[1] - pMouse[1]) * effectiveFactor,
      pMouse[2] + (this.eye[2] - pMouse[2]) * effectiveFactor,
    ];
  }

  public pan(dx: number, dy: number) {
    this.recenterAnim = null;
    const dist = Math.hypot(
      this.eye[0] - this.target[0],
      this.eye[1] - this.target[1],
      this.eye[2] - this.target[2]
    );
    if (dist < 0.0001) return;

    const forward = this.normalize([
      this.target[0] - this.eye[0],
      this.target[1] - this.eye[1],
      this.target[2] - this.eye[2],
    ]);

    let right = this.cross(forward, this.up);
    const rLen = Math.hypot(right[0], right[1], right[2]);
    if (rLen > 0.0001) {
      right = [right[0] / rLen, right[1] / rLen, right[2] / rLen];
    } else {
      right = [1, 0, 0];
    }

    const up = this.normalize(this.cross(right, forward));

    // Screen-space pan tracks 1:1 with mouse movement based on distance to target and viewport height
    const viewportHeight = this.canvas.clientHeight || this.canvas.height || 600;
    const factor = (2.0 * dist * Math.tan(this.fov * 0.5)) / Math.max(viewportHeight, 1);

    const shiftX = -dx * factor;
    const shiftY = dy * factor;

    this.eye[0] += right[0] * shiftX + up[0] * shiftY;
    this.eye[1] += right[1] * shiftX + up[1] * shiftY;
    this.eye[2] += right[2] * shiftX + up[2] * shiftY;

    this.target[0] += right[0] * shiftX + up[0] * shiftY;
    this.target[1] += right[1] * shiftX + up[1] * shiftY;
    this.target[2] += right[2] * shiftX + up[2] * shiftY;
  }

  public recenterCamera(duration: number = 300) {
    if (duration <= 0) {
      this.recenterAnim = null;
      this.eye = [0.0, 1.5, 3.5];
      this.target = [0.0, 1.0, 0.0];
      this.up = [0.0, 1.0, 0.0];
      return;
    }

    this.recenterAnim = {
      startEye: [...this.eye],
      startTarget: [...this.target],
      startUp: [...this.up],
      endEye: [0.0, 1.5, 3.5],
      endTarget: [0.0, 1.0, 0.0],
      endUp: [0.0, 1.0, 0.0],
      startTime: performance.now(),
      duration,
    };
  }

  private updateRecenterAnimation() {
    if (!this.recenterAnim) return;
    const now = performance.now();
    const elapsed = now - this.recenterAnim.startTime;
    const progress = Math.min(1.0, Math.max(0.0, elapsed / this.recenterAnim.duration));

    // Smooth cubic ease-out
    const ease = 1.0 - Math.pow(1.0 - progress, 3);

    for (let i = 0; i < 3; i++) {
      this.eye[i] = this.recenterAnim.startEye[i] + (this.recenterAnim.endEye[i] - this.recenterAnim.startEye[i]) * ease;
      this.target[i] = this.recenterAnim.startTarget[i] + (this.recenterAnim.endTarget[i] - this.recenterAnim.startTarget[i]) * ease;
    }
    const upLerp: [number, number, number] = [
      this.recenterAnim.startUp[0] + (this.recenterAnim.endUp[0] - this.recenterAnim.startUp[0]) * ease,
      this.recenterAnim.startUp[1] + (this.recenterAnim.endUp[1] - this.recenterAnim.startUp[1]) * ease,
      this.recenterAnim.startUp[2] + (this.recenterAnim.endUp[2] - this.recenterAnim.startUp[2]) * ease,
    ];
    const upNorm = this.normalize(upLerp);
    this.up = (upNorm[0] === 0 && upNorm[1] === 0 && upNorm[2] === 0) ? [0.0, 1.0, 0.0] : (upNorm as [number, number, number]);

    if (progress >= 1.0) {
      this.eye = [0.0, 1.5, 3.5];
      this.target = [0.0, 1.0, 0.0];
      this.up = [0.0, 1.0, 0.0];
      this.recenterAnim = null;
    }
  }

  // ==========================================
  // Render Loop
  // ==========================================
  private checkAndApplyResize() {
    if (!this.canvas) return;
    const clientW = this.canvas.clientWidth;
    const clientH = this.canvas.clientHeight;
    if (clientW <= 0 || clientH <= 0) return;

    const realW = Math.max(Math.round(clientW * this.dpiMultiplier), 64);
    const realH = Math.max(Math.round(clientH * this.dpiMultiplier), 64);

    if (this.canvas.width !== realW || this.canvas.height !== realH) {
      this.resize(clientW, clientH);
    }
  }

  private startRenderLoop() {
    if (this.animationFrameId !== null) return;
    const frame = (now: number) => {
      this.animationFrameId = requestAnimationFrame(frame);
      if (this.isPaused) return;
      if (this.targetFps > 0) {
        const interval = 1000 / this.targetFps;
        const elapsed = now - this.lastFrameTimestamp;
        if (elapsed < interval - 1.0) {
          return;
        }
        this.lastFrameTimestamp = now - (elapsed % interval); // P2-09
      }
      this.checkAndApplyResize(); // P2-10 single render per frame
      this.render();
    };
    this.animationFrameId = requestAnimationFrame(frame);
  }
  // P1-10: pause/resume for hidden viewport (battery/GPU)
  public pause(): void { this.isPaused = true; }
  public resume(): void { if (this.isPaused) { this.isPaused = false; this.lastFrameTimestamp = performance.now(); this.render(); } }
  public getIsPaused(): boolean { return this.isPaused; }
  public setPausedByVisibility(hidden: boolean): void {
    if (hidden) { if (!this.isPaused) { this.wasPausedByVisibility = true; this.pause(); } }
    else { if (this.wasPausedByVisibility) { this.wasPausedByVisibility = false; this.resume(); } }
  }

  public render() {
    if (this.isRendering) return;
    this.isRendering = true;
    try {
      this.updateRecenterAnimation();
      const startTime = performance.now();

      if (this.backend === "webgpu") {
        this.renderWebGPU(startTime);
      } else if (this.backend === "webgl2") {
        this.renderWebGL2(startTime);
      }
    } finally {
      this.isRendering = false;
    }
  }

  private renderWebGPU(startTime: number) {
    if (!this.device || !this.context) return;
    if (
      !this.depthTexture ||
      !this.depthView ||
      this.depthTexture.width !== this.canvas.width ||
      this.depthTexture.height !== this.canvas.height
    ) {
      this.resize(Math.max(this.canvas.clientWidth || this.cssWidth, 320), Math.max(this.canvas.clientHeight || this.cssHeight, 240));
      if (!this.depthView) return;
    }

    let currentTexture: GPUTexture;
    try {
      currentTexture = this.context.getCurrentTexture();
    } catch (e) {
      // Frame pulado: não é fatal, mas não pode ser silencioso (agregado para
      // não inundar a lista a 60 fps).
      this.report("frame_skipped", "o swap chain não devolveu textura neste frame", {
        detail: e instanceof Error ? e.message : String(e),
      });
      return;
    }
    const textureView = currentTexture.createView();

    const aspect = this.canvas.width / Math.max(this.canvas.height, 1);
    const viewProj = this.calculateViewProjectionMatrix(aspect, true);

    // 1..4. Uniforms — layout e empacotamento vêm do contrato, os mesmos bytes
    // que `uniforms.rs` produz no headless (P0 renderer: buffers unificados).
    const camData = cameraUniformFloats({ viewProj, eye: this.eye, model: IDENTITY_MAT4 });
    this.device.queue.writeBuffer(this.cameraBuffer!, 0, camData);

    const lightData = lightUniformFloats({
      direction: this.lightDir as [number, number, number],
      intensity: this.lightIntensity,
      color: this.lightColor as [number, number, number],
      ambientIntensity: this.ambientIntensity,
      shadowColor: this.shadowColor as [number, number, number],
      shadowSaturation: this.shadowSaturation,
      ambientSky: this.ambientSky as [number, number, number],
      ambientGround: this.ambientGround as [number, number, number],
    });
    this.device.queue.writeBuffer(this.lightBuffer!, 0, lightData);

    const matData = materialUniformFloats({
      baseColor: this.baseColor as [number, number, number, number],
      shadeColor: this.shadeColor as [number, number, number, number],
      specularColor: this.specColor as [number, number, number, number],
      rimColor: [this.rimColor[0], this.rimColor[1], this.rimColor[2], 1.0],
      shadowThreshold: this.shadowThreshold,
      shadowSmoothness: this.toonSmoothness,
      specIntensity: this.specIntensity,
      specPower: this.specExponent,
      rimIntensity: this.rimIntensity,
      rimSpread: this.rimSpread,
      hueShiftDegrees: this.hueShift,
      toonSteps: this.toonSteps,
      specularSoftness: this.specSoftness,
      specularOffset: this.specOffset,
      specularSize: this.specularSize,
      aoIntensity: this.aoIntensity,
    });
    this.device.queue.writeBuffer(this.materialBuffer!, 0, matData);

    const outlineData = outlineUniformFloats({
      color: this.outlineColor as [number, number, number, number],
      width: this.outlineWidth,
      aspect,
      depthBias: this.outlineDepthBias,
      opacity: this.outlineOpacity,
      smoothness: this.outlineSmoothness,
    });
    this.device.queue.writeBuffer(this.outlineBuffer!, 0, outlineData);

    // 5. Render Passes (Compute Sparse Morphs followed by NPR Cel-Shading)
    const commandEncoder = this.device.createCommandEncoder();

    // P0 §7.5: sem geometria canônica não há compute — o shell é avisado uma vez
    // e o viewport segue exibindo a última geometria autorizada pelo núcleo.
    if (!this.coreGeometry && !this.coreGeometryRequested && this.currentPreset === "mannequin") {
      this.requestCoreGeometry("render sem geometria canônica");
    }

    if (
      this.gpuMorphActive &&
      this.coreGeometry &&
      this.morphPipeline &&
      this.morphBindGroup &&
      this.morphedVertexBuffer &&
      this.morphVertexCount > 0
    ) {
      // Pesos do frame: autorais do núcleo + adiantamento local (janela curta).
      if (this.gpuMorphDirty && this.morphChannelsBuffer) {
        try {
          this.device.queue.writeBuffer(
            this.morphChannelsBuffer,
            0,
            packChannelRecordsWithWeights(this.coreGeometry.channels, this.effectiveChannelWeights())
          );
        } catch (e) {
          this.report("channel_upload_failed", "falha ao subir pesos de canal para a GPU", {
            detail: e instanceof Error ? e.message : String(e),
          });
        }
        this.gpuMorphDirty = false;
      }
      this.dispatchSparseMorphs(commandEncoder, this.morphVertexCount);
    }

    // P0-07: MSAA resolve (msaa view → swapchain)
    const colorView = this.msaaColorView ?? textureView;
    const resolveTarget = this.msaaColorView ? textureView : undefined;
    const passEncoder = commandEncoder.beginRenderPass({
      colorAttachments: [
        {
          view: colorView,
          resolveTarget,
          clearValue: {
            r: this.clearColor[0],
            g: this.clearColor[1],
            b: this.clearColor[2],
            a: this.clearColor[3],
          },
          loadOp: "clear",
          storeOp: "store",
        },
      ],
      depthStencilAttachment: {
        view: this.depthView,
        depthClearValue: 1.0,
        depthLoadOp: "clear",
        depthStoreOp: "store",
      },
    });

    const activeVbo = (this.gpuMorphActive && this.morphPipeline && this.morphBindGroup && this.morphedVertexBuffer && this.morphVertexCount > 0)
      ? this.morphedVertexBuffer
      : this.vertexBuffer!;
    passEncoder.setVertexBuffer(0, activeVbo);
    passEncoder.setIndexBuffer(this.indexBuffer!, "uint32");

    // Ordem dos passes vem do contrato (outline → cel em ordem de `order`),
    // exatamente como o headless monta o render pass.
    for (const pass of renderPasses()) {
      if (pass.name === "outline") {
        passEncoder.setPipeline(this.outlinePipeline!);
        passEncoder.setBindGroup(0, this.outlineBindGroup!);
      } else if (pass.name === "cel") {
        passEncoder.setPipeline(this.celPipeline!);
        passEncoder.setBindGroup(0, this.celBindGroup!);
      } else {
        continue;
      }
      passEncoder.drawIndexed(this.indexCount);
    }

    passEncoder.end();
    this.device.queue.submit([commandEncoder.finish()]);

    this.recordMetrics(startTime, "WebGPU Hardware");
  }

  /**
   * WebGL2 não tem compute shaders: os deltas do núcleo são acumulados no CPU
   * (mesmos deltas, mesmo pesos — o meio muda, a geometria não).
   */
  private applyCoreDeltasToGlBuffer(gl: WebGL2RenderingContext): void {
    if (!this.glVbo || !this.coreGeometry || !this.gpuMorphDirty) return;
    if (this.coreGeometry.channels.length === 0) return;
    const deformed = applyDeltasCpu(this.coreGeometry, this.effectiveChannelWeights());
    gl.bindBuffer(gl.ARRAY_BUFFER, this.glVbo);
    gl.bufferData(gl.ARRAY_BUFFER, deformed, gl.DYNAMIC_DRAW);
    gl.bindBuffer(gl.ARRAY_BUFFER, null);
    this.gpuMorphDirty = false;
  }

  private renderWebGL2(startTime: number) {
    const gl = this.gl;
    if (!gl || !this.glCelProgram || !this.glOutlineProgram || !this.glVao) return;

    this.applyCoreDeltasToGlBuffer(gl);
    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.clearColor(this.clearColor[0], this.clearColor[1], this.clearColor[2], this.clearColor[3]);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);

    gl.enable(gl.DEPTH_TEST);
    gl.depthFunc(gl.LEQUAL);

    const aspect = this.canvas.width / Math.max(this.canvas.height, 1);
    const viewProj = this.calculateViewProjectionMatrix(aspect, false);

    gl.bindVertexArray(this.glVao);

    // PASS 1: Cel-Shading Frontfaces
    gl.enable(gl.CULL_FACE);
    gl.cullFace(gl.BACK);

    gl.useProgram(this.glCelProgram);
    gl.uniformMatrix4fv(gl.getUniformLocation(this.glCelProgram, "u_view_proj"), false, viewProj);
    gl.uniform3f(gl.getUniformLocation(this.glCelProgram, "u_light_dir"), this.lightDir[0], this.lightDir[1], this.lightDir[2]);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_light_intensity"), this.lightIntensity);
    gl.uniform3f(gl.getUniformLocation(this.glCelProgram, "u_light_color"), this.lightColor[0], this.lightColor[1], this.lightColor[2]);
    gl.uniform3f(gl.getUniformLocation(this.glCelProgram, "u_shadow_color"), this.shadowColor[0], this.shadowColor[1], this.shadowColor[2]);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_ambient_intensity"), this.ambientIntensity);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_shadow_saturation"), this.shadowSaturation);
    gl.uniform4f(gl.getUniformLocation(this.glCelProgram, "u_base_color"), this.baseColor[0], this.baseColor[1], this.baseColor[2], this.baseColor[3]);
    gl.uniform4f(gl.getUniformLocation(this.glCelProgram, "u_shade_color"), this.shadeColor[0], this.shadeColor[1], this.shadeColor[2], this.shadeColor[3]);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_shadow_threshold"), this.shadowThreshold);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_shadow_smoothness"), this.toonSmoothness);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_hue_shift"), this.hueShift);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_toon_steps"), this.toonSteps);
    gl.uniform3f(gl.getUniformLocation(this.glCelProgram, "u_camera_pos"), this.eye[0], this.eye[1], this.eye[2]);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_spec_intensity // P2-07 TODO separate spec_size uniform"), this.specIntensity);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_spec_power"), this.specExponent);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_spec_softness"), this.specSoftness);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_spec_offset"), this.specOffset);
    gl.uniform4f(gl.getUniformLocation(this.glCelProgram, "u_spec_color"), this.specColor[0], this.specColor[1], this.specColor[2], this.specColor[3]);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_rim_intensity"), this.rimIntensity);
    gl.uniform1f(gl.getUniformLocation(this.glCelProgram, "u_rim_spread"), this.rimSpread);
    gl.uniform3f(gl.getUniformLocation(this.glCelProgram, "u_rim_color"), this.rimColor[0], this.rimColor[1], this.rimColor[2]);

    gl.drawElements(gl.TRIANGLES, this.indexCount, gl.UNSIGNED_INT, 0);

    // PASS 2: Inverted Hull Backfaces
    gl.cullFace(gl.FRONT);

    gl.useProgram(this.glOutlineProgram);
    gl.uniformMatrix4fv(gl.getUniformLocation(this.glOutlineProgram, "u_view_proj"), false, viewProj);
    gl.uniform1f(gl.getUniformLocation(this.glOutlineProgram, "u_outline_width"), this.outlineWidth);
    gl.uniform1f(gl.getUniformLocation(this.glOutlineProgram, "u_aspect"), aspect);
    gl.uniform1f(gl.getUniformLocation(this.glOutlineProgram, "u_outline_depth_bias"), this.outlineDepthBias);
    gl.uniform4f(gl.getUniformLocation(this.glOutlineProgram, "u_outline_color"), this.outlineColor[0], this.outlineColor[1], this.outlineColor[2], this.outlineColor[3]);
    gl.uniform1f(gl.getUniformLocation(this.glOutlineProgram, "u_outline_opacity"), this.outlineOpacity);
    gl.uniform1f(gl.getUniformLocation(this.glOutlineProgram, "u_outline_smoothness"), this.outlineSmoothness);

    gl.enable(gl.BLEND);
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

    gl.drawElements(gl.TRIANGLES, this.indexCount, gl.UNSIGNED_INT, 0);

    gl.disable(gl.BLEND);

    gl.bindVertexArray(null);

    this.recordMetrics(startTime, "WebGL2 Fallback");
  }

  private recordMetrics(startTime: number, adapter: string) {
    // P2-08 real GPU timing (was CPU submit only) + real drawCalls/triangles - pedestal excluded
    const elapsed = performance.now() - startTime;
    this.frameCounter++;
    if (performance.now() - this.fpsTimer >= 500) {
      const currentFps = Math.round((this.frameCounter * 1000) / (performance.now() - this.fpsTimer));
      this.frameCounter = 0;
      this.fpsTimer = performance.now();
      if (this.onMetricsUpdate) {
        // P2-08 adapter.info is deprecated, try requestAdapterInfo fallback
        const adapterName = (this.adapter as any)?.info?.device || (this.adapter as any)?.info?.description || adapter;
        // triangles without pedestal (pedestal ~12 tris)
        const realTris = Math.max(0, Math.floor(this.indexCount / 3) - 12);
        if (this.onMetricsUpdate) {
          this.onMetricsUpdate({
            fps: currentFps,
            frameTimeMs: elapsed, // TODO GPUQuerySet timestamp when available
            triangles: realTris,
            drawCalls: 2, // cel + outline
            adapterName,
            backend: this.backend === "webgpu" ? "WebGPU" : "WebGL2",
          });
        }
      }
    }
  }

  // ==========================================
  // Matrix Math (Column-Major Standard)
  // ==========================================
  private calculateViewProjectionMatrix(aspect: number, isWebGPU: boolean): Float32Array {
    // P0 renderer: uma única implementação de câmera — `src/services/camera_math.ts`,
    // a mesma conta que o core faz em Rust (`math.rs`) e que o contrato congela no
    // `reference_frame`. Antes havia uma segunda cópia (lookAt + perspectiva) aqui.
    return viewProjectionMatrix(
      { eye: this.eye, target: this.target, up: this.up, fov: this.fov },
      aspect,
      {
        clipDepth: isWebGPU ? "zero_to_one" : "minus_one_to_one",
        near: DEFAULT_CAMERA_NEAR,
        far: DEFAULT_CAMERA_FAR,
      }
    );
  }

  // Exact column-major matrix multiplication: out = a * b
  private multiplyMat4(a: number[], b: number[]): number[] {
    const out = new Array(16).fill(0);
    for (let c = 0; c < 4; c++) {
      for (let r = 0; r < 4; r++) {
        let sum = 0;
        for (let k = 0; k < 4; k++) {
          sum += a[k * 4 + r] * b[c * 4 + k];
        }
        out[c * 4 + r] = sum;
      }
    }
    return out;
  }

  private normalize(v: number[]): number[] {
    const len = Math.hypot(v[0], v[1], v[2]);
    return len > 0.00001 ? [v[0] / len, v[1] / len, v[2] / len] : [0, 0, 0];
  }

  private cross(a: number[], b: number[]): number[] {
    return [
      a[1] * b[2] - a[2] * b[1],
      a[2] * b[0] - a[0] * b[2],
      a[0] * b[1] - a[1] * b[0],
    ];
  }

  private dot(a: number[], b: number[]): number {
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
  }

  // Rodrigues' Rotation Formula for continuous unrestricted 360-degree rotation
  private rotateVector(
    v: [number, number, number],
    axis: [number, number, number],
    angle: number
  ): [number, number, number] {
    const cos = Math.cos(angle);
    const sin = Math.sin(angle);
    const dot = v[0] * axis[0] + v[1] * axis[1] + v[2] * axis[2];
    const cross: [number, number, number] = [
      axis[1] * v[2] - axis[2] * v[1],
      axis[2] * v[0] - axis[0] * v[2],
      axis[0] * v[1] - axis[1] * v[0],
    ];
    return [
      v[0] * cos + cross[0] * sin + axis[0] * dot * (1.0 - cos),
      v[1] * cos + cross[1] * sin + axis[1] * dot * (1.0 - cos),
      v[2] * cos + cross[2] * sin + axis[2] * dot * (1.0 - cos),
    ];
  }

  // P1-15: CSS→backing store conversion for correct raycast when DPI≠1
  private toCanvasSpace(cssX: number, cssY: number): [number, number] {
    const rect = this.canvas.getBoundingClientRect();
    const cssW = rect.width || this.cssWidth || this.canvas.clientWidth || 1;
    const cssH = rect.height || this.cssHeight || this.canvas.clientHeight || 1;
    const scaleX = this.canvas.width / Math.max(cssW, 1);
    const scaleY = this.canvas.height / Math.max(cssH, 1);
    return [cssX * scaleX, cssY * scaleY];
  }
  public raycastTactile(screenX: number, screenY: number): RaycastHit | null {
    const [bx, by] = this.toCanvasSpace(screenX, screenY);
    const ray = createCameraRay(
      bx,
      by,
      this.canvas.width,
      this.canvas.height,
      this.eye,
      this.target,
      this.up,
      this.fov
    );
    return raycastTactileHulls(ray);
  }

  public projectTactileDelta(
    segment: AnatomicalSegment,
    deltaXPixels: number,
    deltaYPixels: number
  ): TactileDragResult {
    // deltas are in CSS pixels → convert to NDC using CSS size, not backing store
    const rect = this.canvas.getBoundingClientRect();
    const cssW = rect.width || this.cssWidth || 1;
    const cssH = rect.height || this.cssHeight || 1;
    const dxNdc = (2.0 * deltaXPixels) / Math.max(cssW, 1);
    const dyNdc = (2.0 * deltaYPixels) / Math.max(cssH, 1);
    return projectTactileDrag(segment, dxNdc, dyNdc);
  }

  public destroy() {
    if (this.animationFrameId !== null) {
      cancelAnimationFrame(this.animationFrameId);
      this.animationFrameId = null;
    }
    this.recenterAnim = null;
    this.liveWeights.clear();
    this.channelWeights.clear();
    this.coreGeometry = null;
    this.isPaused = true;
    // P1-15: complete resource cleanup (was leaking pipelines/bindGroups/compute)
    try { this.device?.destroy(); } catch (_) {}
    try { (this.context as any)?.unconfigure?.(); } catch (_) {}
    if (this.vertexBuffer) { try { this.vertexBuffer.destroy(); } catch (_) {} this.vertexBuffer = null; }
    if (this.indexBuffer) { try { this.indexBuffer.destroy(); } catch (_) {} this.indexBuffer = null; }
    if (this.cameraBuffer) { try { this.cameraBuffer.destroy(); } catch (_) {} this.cameraBuffer = null; }
    if (this.lightBuffer) { try { this.lightBuffer.destroy(); } catch (_) {} this.lightBuffer = null; }
    if (this.materialBuffer) { try { this.materialBuffer.destroy(); } catch (_) {} this.materialBuffer = null; }
    if (this.outlineBuffer) { try { this.outlineBuffer.destroy(); } catch (_) {} this.outlineBuffer = null; }
    if (this.depthTexture) { try { this.depthTexture.destroy(); } catch (_) {} this.depthTexture = null; this.depthView = null; }
    if (this.msaaColorTexture) { try { this.msaaColorTexture.destroy(); } catch (_) {} this.msaaColorTexture = null; this.msaaColorView = null; }
    if (this.toonRampTexture) { try { this.toonRampTexture.destroy(); } catch (_) {} this.toonRampTexture = null; }
    // compute pipeline resources
    if (this.morphHeaderBuffer) { try { this.morphHeaderBuffer.destroy(); } catch (_) {} this.morphHeaderBuffer = null; }
    if (this.morphBaseBuffer) { try { this.morphBaseBuffer.destroy(); } catch (_) {} this.morphBaseBuffer = null; }
    if (this.morphDeltasBuffer) { try { this.morphDeltasBuffer.destroy(); } catch (_) {} this.morphDeltasBuffer = null; }
    if (this.morphChannelsBuffer) { try { this.morphChannelsBuffer.destroy(); } catch (_) {} this.morphChannelsBuffer = null; }
    if (this.morphedVertexBuffer) { try { this.morphedVertexBuffer.destroy(); } catch (_) {} this.morphedVertexBuffer = null; }
    this.morphBindGroup = null;
    this.morphPipeline = null;
    this.morphBindGroupLayout = null;
    // WebGL2 cleanup
    if (this.gl) {
      try {
        const gl = this.gl;
        if (this.glVao) gl.deleteVertexArray(this.glVao);
        if (this.glVbo) gl.deleteBuffer(this.glVbo);
        if (this.glIbo) gl.deleteBuffer(this.glIbo);
        if (this.glCelProgram) gl.deleteProgram(this.glCelProgram);
        if (this.glOutlineProgram) gl.deleteProgram(this.glOutlineProgram);
        const ext = gl.getExtension('WEBGL_lose_context');
        (ext as any)?.loseContext?.();
      } catch (_) {}
      this.gl = null; this.glVao = null; this.glVbo = null; this.glIbo = null;
      this.glCelProgram = null; this.glOutlineProgram = null;
    }
    this.celPipeline = null; this.outlinePipeline = null;
    this.celBindGroup = null; this.outlineBindGroup = null;
    this.toonRampSampler = null;
  }
}
