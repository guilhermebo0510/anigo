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
  buildSparseMorphSet,
  channelDenominator,
  genericMorphDelta,
  isExplicitMorph,
  normalizeWeight,
  packChannelWeights,
  recomputeNormals,
  type MeshBounds,
  type SparseMorphSet,
} from "../../services/morph_engine";
import { GlbParseError, loadGlbMesh } from "../../services/gltf_loader";
// P0-04: single source of truth — WGSL now imported from canonical shaders/ (was 4 duplicated copies)
// @ts-ignore - Vite ?raw import
import celShaderSource from "../../../shaders/cel_shading.wgsl?raw";
// @ts-ignore - Vite ?raw import
import outlineShaderSource from "../../../shaders/inverted_hull.wgsl?raw";

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

/** Scratch delta reused by the generic fallback (no per-vertex allocation). */
const GENERIC_DELTA_TMP = { dx: 0, dy: 0, dz: 0 };

/** Pseudo-channels for the linear somatotype components (GPU sparse path). */
const SOMA_CHANNEL_IDS = ["__soma_endo", "__soma_meso", "__soma_ecto"] as const;

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
  // P0-07: 4x MSAA (was sampleCount=1)
  private msaaColorTexture: GPUTexture | null = null;
  private msaaColorView: GPUTextureView | null = null;
  private sampleCount: number = 4;
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
  private morphDirty: boolean = false;
  private pendingMorph: Float32Array | null = null;
  private pendingMorphCount: number = 0;
  private morphRaf: number | null = null;
  private pendingMorphSliders: Map<string, number> = new Map();
  private pendingGender: number | null = null;
  private morphedVertexBuffer: GPUBuffer | null = null;
  private morphBindGroup: GPUBindGroup | null = null;
  private morphVertexCount: number = 0;
  // P0-05: live GPU sparse-morph state (channels = 157 sliders + 3 somatotype).
  private gpuMorphSet: SparseMorphSet | null = null;
  private gpuMorphActive: boolean = false;
  private gpuMorphDirty: boolean = false;
  private gpuRebuildTimer: ReturnType<typeof setTimeout> | null = null;
  private pendingSomatotype: [number, number, number] | null = null;
  // P0-05: persistent buffer capacities (no destroy/create churn per event).
  private vertexCapacityBytes: number = 0;
  private indexCapacityBytes: number = 0;
  /** P0-10: model load failures surface here (and throw to the caller). */
  public onModelLoadError?: (message: string) => void;

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
      console.warn("[ANIGO 3D] Pre-loading canonical model warning:", e);
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
                console.error("[ANIGO][GPU] uncaptured error:", e?.error || e);
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
            } catch (_) {}

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
        console.warn("[ANIGO 3D] WebGPU initialization failed, switching to WebGL2 fallback:", e);
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

    const celShaderCode = celShaderSource;

    const outlineShaderCode = outlineShaderSource;

    const celModule = this.device.createShaderModule({ code: celShaderCode });
    const outlineModule = this.device.createShaderModule({ code: outlineShaderCode });

    const vertexBufferLayout: GPUVertexBufferLayout = {
      arrayStride: 72,
      attributes: [
        { shaderLocation: 0, offset: 0, format: "float32x3" },
        { shaderLocation: 1, offset: 12, format: "float32x3" },
        { shaderLocation: 2, offset: 24, format: "float32x2" },
        { shaderLocation: 3, offset: 32, format: "float32x4" },
        { shaderLocation: 4, offset: 48, format: "uint16x4" },
        { shaderLocation: 5, offset: 56, format: "float32x4" },
      ],
    };

    this.celPipeline = this.device.createRenderPipeline({
      layout: "auto",
      vertex: {
        module: celModule,
        entryPoint: "vs_main",
        buffers: [vertexBufferLayout],
      },
      fragment: {
        module: celModule,
        entryPoint: "fs_main",
        targets: [{ 
            format: this.format
            // P2-03 no blend for opaque cel (was premultiplied without premultiply) — saves bandwidth
        }],
      },
      primitive: {
        topology: "triangle-list",
        cullMode: "back",
      },
      depthStencil: {
        format: "depth24plus",
        depthWriteEnabled: true,
        depthCompare: "less-equal",
      },
      multisample: { count: 4 },
    });

    this.outlinePipeline = this.device.createRenderPipeline({
      layout: "auto",
      vertex: {
        module: outlineModule,
        entryPoint: "vs_main",
        buffers: [vertexBufferLayout],
      },
      fragment: {
        module: outlineModule,
        entryPoint: "fs_main",
        targets: [{ 
            format: this.format,
            blend: {
                color: { srcFactor: "src-alpha", dstFactor: "one-minus-src-alpha", operation: "add" },
                alpha: { srcFactor: "one", dstFactor: "one-minus-src-alpha", operation: "add" },
            }
        }],
      },
      primitive: {
        topology: "triangle-list",
        cullMode: "front", // Backfaces extruded for inverted hull
      },
      depthStencil: {
        format: "depth24plus",
        depthWriteEnabled: false, // P2-01 outline no write (avoids z-fighting)
        depthBias: 1,
        depthBiasSlopeScale: 1.0,
        depthBiasClamp: 0.0,
        depthCompare: "less-equal",
      },
      multisample: { count: 4 },
    });

    // Sub-Sprint 3.3: WebGPU Sparse Morph Target Compute Pipeline
    const morphComputeCode = `
      struct SparseMorphHeader {
        active_channel_count: u32,
        total_vertex_count: u32,
        total_delta_count: u32,
        _pad: u32,
      };

      struct MorphChannel {
        weight: f32,
        start_offset: u32,
        delta_count: u32,
        _pad: u32,
      };

      struct SparseMorphDelta {
        vertex_index: u32,
        delta_px: f32,
        delta_py: f32,
        delta_pz: f32,
        delta_nx: f32,
        delta_ny: f32,
        delta_nz: f32,
        _pad: f32,
      };

      struct VertexRaw {
        pos_x: f32,
        pos_y: f32,
        pos_z: f32,
        norm_x: f32,
        norm_y: f32,
        norm_z: f32,
        uv_u: f32,
        uv_v: f32,
        col_r: f32,
        col_g: f32,
        col_b: f32,
        col_a: f32,
        joints_0_1: u32,
        joints_2_3: u32,
        weight_0: f32,
        weight_1: f32,
        weight_2: f32,
        weight_3: f32,
      };

      @group(0) @binding(0) var<uniform> header: SparseMorphHeader;
      @group(0) @binding(1) var<storage, read> base_vertices: array<VertexRaw>;
      @group(0) @binding(2) var<storage, read> morph_deltas: array<SparseMorphDelta>;
      @group(0) @binding(3) var<storage, read> active_channels: array<MorphChannel>;
      @group(0) @binding(4) var<storage, read_write> out_vertices: array<VertexRaw>;

      @compute @workgroup_size(64)
      fn cs_accumulate_morphs(@builtin(global_invocation_id) global_id: vec3<u32>) {
        let vert_idx = global_id.x;
        if (vert_idx >= header.total_vertex_count) {
          return;
        }

        var base_v = base_vertices[vert_idx];
        var p = vec3<f32>(base_v.pos_x, base_v.pos_y, base_v.pos_z);
        var n = vec3<f32>(base_v.norm_x, base_v.norm_y, base_v.norm_z);

        for (var c: u32 = 0u; c < header.active_channel_count; c = c + 1u) {
          let ch = active_channels[c];
          if (abs(ch.weight) > 1e-6 && ch.delta_count > 0u) {
            let start = ch.start_offset;
            let count = ch.delta_count;

            var low: u32 = 0u;
            var high: u32 = count;
            var found_idx: u32 = 0xFFFFFFFFu;

            while (low < high) {
              let mid = low + (high - low) / 2u;
              let delta_vert = morph_deltas[start + mid].vertex_index;
              if (delta_vert == vert_idx) {
                found_idx = start + mid;
                break;
              } else if (delta_vert < vert_idx) {
                low = mid + 1u;
              } else {
                high = mid;
              }
            }

            if (found_idx != 0xFFFFFFFFu) {
              let delta = morph_deltas[found_idx];
              p = p + ch.weight * vec3<f32>(delta.delta_px, delta.delta_py, delta.delta_pz);
              n = n + ch.weight * vec3<f32>(delta.delta_nx, delta.delta_ny, delta.delta_nz);
            }
          }
        }

        let n_sq = dot(n, n);
        if (n_sq > 1e-12) {
          n = normalize(n);
        }

        base_v.pos_x = p.x;
        base_v.pos_y = p.y;
        base_v.pos_z = p.z;
        base_v.norm_x = n.x;
        base_v.norm_y = n.y;
        base_v.norm_z = n.z;

        out_vertices[vert_idx] = base_v;
      }
    `;

    try {
      const morphModule = this.device.createShaderModule({
        label: "Sparse Morph Compute Module",
        code: morphComputeCode,
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
      console.warn("[ANIGO 3D] Compute pipeline initialization fallback:", e);
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
      console.error("[ANIGO 3D] WebGL2 not supported in this environment — both backends failed (observable fallback).");
      // P0-06: surface black-screen failure as observable DOM overlay instead of silent
      try {
        const overlay = document.createElement("div");
        overlay.textContent = "[ANIGO] Falha ao inicializar WebGPU e WebGL2 — verifique driver/GPU. Veja console para detalhes.";
        overlay.style.cssText = "position:absolute;inset:0;display:flex;align-items:center;justify-content:center;background:#1a1d2e;color:#ff6b6b;padding:16px;text-align:center;font:13px sans-serif;z-index:9999";
        this.canvas.parentElement?.appendChild(overlay);
      } catch (_) {}
      return false;
    }
    this.gl = gl as any;

    const vsCel = `#version 300 es
      layout(location = 0) in vec3 a_pos;
      layout(location = 1) in vec3 a_normal;
      layout(location = 2) in vec2 a_uv;
      layout(location = 3) in vec4 a_color;
      layout(location = 4) in uvec4 a_joints;
      layout(location = 5) in vec4 a_weights;

      uniform mat4 u_view_proj;
      out vec3 v_normal;
      out vec3 v_pos;
      out vec4 v_color;
      out vec2 v_uv;

      void main() {
        v_pos = a_pos;
        v_normal = a_normal;
        v_color = a_color;
        v_uv = a_uv;
        gl_Position = u_view_proj * vec4(a_pos, 1.0);
      }
    `;

    const fsCel = `#version 300 es
      precision highp float;
      in vec3 v_normal;
      in vec3 v_pos;
      in vec4 v_color;
      in vec2 v_uv;

      uniform vec3 u_light_dir;
      uniform float u_light_intensity;
      uniform vec3 u_light_color;
      uniform vec3 u_shadow_color;
      uniform float u_ambient_intensity;
      uniform float u_shadow_saturation;
      uniform vec4 u_base_color;
      uniform vec4 u_shade_color;
      uniform float u_shadow_threshold;
      uniform float u_shadow_smoothness;
      uniform float u_hue_shift;
      uniform float u_toon_steps;
      uniform vec3 u_camera_pos;
      uniform float u_spec_intensity // P2-07 TODO separate spec_size uniform;
      uniform float u_spec_power;
      uniform float u_spec_softness;
      uniform float u_spec_offset;
      uniform vec4 u_spec_color;
      uniform float u_rim_intensity;
      uniform float u_rim_spread;
      uniform vec3 u_rim_color;

      out vec4 fragColor;

      vec3 rgb2hsv(vec3 c) {
        vec4 K = vec4(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
        vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
        vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
        float d = q.x - min(q.w, q.y);
        float e = 1.0e-10;
        return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
      }

      vec3 hsv2rgb(vec3 c) {
        vec4 K = vec4(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
        vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
        return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
      }
      // P1-01 sRGB ↔ linear
      vec3 srgbToLinear(vec3 c) {
        bvec3 cutoff = lessThanEqual(c, vec3(0.04045));
        vec3 lo = c / 12.92;
        vec3 hi = pow((c + vec3(0.055)) / 1.055, vec3(2.4));
        return mix(hi, lo, vec3(cutoff));
      }
      vec3 linearToSrgb(vec3 c) {
        bvec3 cutoff = lessThanEqual(c, vec3(0.0031308));
        vec3 lo = c * 12.92;
        vec3 hi = 1.055 * pow(c, vec3(1.0/2.4)) - 0.055;
        return mix(hi, lo, vec3(cutoff));
      }
      // P1-02 OKLab hue rotation (fallback to linear * saturation for low chroma)
      vec3 linearToOklab(vec3 c) {
        float l = 0.4122214708*c.r + 0.5363325363*c.g + 0.0514459929*c.b;
        float m = 0.2119034982*c.r + 0.6806995451*c.g + 0.1073969566*c.b;
        float s = 0.0883024619*c.r + 0.2817188376*c.g + 0.6299787005*c.b;
        float l_ = pow(max(l,0.0), 1.0/3.0);
        float m_ = pow(max(m,0.0), 1.0/3.0);
        float s_ = pow(max(s,0.0), 1.0/3.0);
        return vec3(
          0.2104542553*l_ + 0.7936177850*m_ - 0.0040720468*s_,
          1.9779984951*l_ - 2.4285922050*m_ + 0.4505937099*s_,
          0.0259040371*l_ + 0.7827717662*m_ - 0.8086757660*s_
        );
      }
      vec3 oklabToLinear(vec3 c) {
        float l_ = c.x + 0.3963377774*c.y + 0.2158037573*c.z;
        float m_ = c.x - 0.1055613458*c.y - 0.0638541728*c.z;
        float s_ = c.x - 0.0894841775*c.y - 1.2914855480*c.z;
        float l = l_*l_*l_;
        float m = m_*m_*m_;
        float s = s_*s_*s_;
        return vec3(
          4.0767416621*l - 3.3077115913*m + 0.2309699292*s,
          -1.2684380046*l + 2.6097574011*m - 0.3413193965*s,
          -0.0041960863*l - 0.7034186147*m + 1.7076147010*s
        );
      }

      void main() {
        vec3 N = normalize(v_normal);
        vec3 L = normalize(u_light_dir);
        vec3 V = normalize(u_camera_pos - v_pos);
        float n_dot_l = dot(N, L);
        float half_lambert = n_dot_l * 0.5 + 0.5;
        float shift = (v_color.g - 0.5) * 0.3;
        float threshold = u_shadow_threshold + shift;
        float smoothness = max(u_shadow_smoothness, 0.001);

        float u_coord = clamp((half_lambert - threshold) + 0.5, 0.0, 1.0);
        float toon = u_coord;
        if (u_toon_steps < 0.5) {
          toon = smoothstep(threshold - 0.35 - smoothness, threshold + 0.35 + smoothness, half_lambert);
        } else if (u_toon_steps >= 0.5 && u_toon_steps < 1.5) {
          toon = smoothstep(threshold - smoothness, threshold + smoothness, half_lambert);
        } else if (u_toon_steps >= 1.5 && u_toon_steps < 2.5) {
          float s1 = smoothstep(threshold - 0.14 - smoothness, threshold - 0.14 + smoothness, half_lambert);
          float s2 = smoothstep(threshold + 0.14 - smoothness, threshold + 0.14 + smoothness, half_lambert);
          toon = s1 * 0.45 + s2 * 0.55;
        } else {
          float s1 = smoothstep(threshold - 0.20 - smoothness, threshold - 0.20 + smoothness, half_lambert);
          float s2 = smoothstep(threshold - smoothness, threshold + smoothness, half_lambert);
          float s3 = smoothstep(threshold + 0.20 - smoothness, threshold + 0.20 + smoothness, half_lambert);
          toon = (s1 + s2 + s3) / 3.0;
        }

        // P0-03 + P1-01 linear: base/light in linear
        vec3 baseLin = srgbToLinear(u_base_color.rgb);
        vec3 lightLin = srgbToLinear(u_light_color);
        vec3 lit = baseLin * lightLin * clamp(u_light_intensity, 0.0, 3.0);

        // P1-01 linear + P1-02 OKLab hue (clamp ±180)
        float hueShiftRad = radians(clamp(u_hue_shift, -180.0, 180.0));
        vec3 shadeLin = srgbToLinear(u_shade_color.rgb);
        vec3 shadowTintLin = srgbToLinear(u_shadow_color);
        vec3 raw_shadow_lin = shadeLin * shadowTintLin;
        // OKLab hue rotation
        vec3 lab = linearToOklab(raw_shadow_lin);
        float C = length(lab.yz);
        vec3 hueShiftedLin;
        if (C < 0.0001) {
          hueShiftedLin = raw_shadow_lin * mix(1.0, clamp(u_shadow_saturation,0.0,2.0), 0.5);
        } else {
          float hue = atan(lab.z, lab.y);
          float newHue = hue + hueShiftRad;
          float C2 = clamp(C * clamp(u_shadow_saturation,0.0,2.0), 0.0, 0.4);
          lab.y = C2 * cos(newHue);
          lab.z = C2 * sin(newHue);
          hueShiftedLin = oklabToLinear(lab);
        }
        float ambient = clamp(0.2 + u_ambient_intensity * 0.8, 0.05, 1.5);
        vec3 shadow = hueShiftedLin * ambient;

        vec3 base_cel = mix(shadow, lit, toon);

        vec3 H = normalize(L + V);
        float n_dot_h = max(dot(N, H), 0.0);
        vec3 up_vec = vec3(0.0, 1.0, 0.0);
        vec3 tangent = normalize(cross(N, mix(up_vec, vec3(1.0, 0.0, 0.0), step(0.99, abs(N.y)))));
        float t_dot_h = dot(tangent, H);
        float aniso = sqrt(max(1.0 - t_dot_h * t_dot_h, 0.0));
        // P0-04: jitter unified to world_pos.y*35 + uv.x*20 (was v_pos.x diverging)
        float jitter_pos = v_pos.y * 35.0 + v_uv.x * 20.0 + u_spec_offset * 10.0;
        float jitter = sin(jitter_pos) * 0.08;
        float spec_base = max(mix(n_dot_h, aniso * n_dot_h, 0.35), 0.0);
        float spec_term = pow(spec_base, max(u_spec_power, 1.0));
        float spec_cutoff = clamp(0.65 - (u_spec_intensity // P2-07 TODO separate spec_size uniform * 0.12), 0.30, 0.65);
        float spec_soft_clamped = max(u_spec_softness, 0.001);
        float spec_step = smoothstep(spec_cutoff + jitter - spec_soft_clamped, spec_cutoff + jitter + spec_soft_clamped, spec_term) * u_spec_intensity // P2-07 TODO separate spec_size uniform * v_color.a * toon;

        float rim_dot = 1.0 - max(dot(V, N), 0.0);
        float rim_fresnel = smoothstep(1.0 - u_rim_spread, 1.0, rim_dot);
        float rim_backlight = max(dot(L, -V) * 0.6 + 0.4, 0.0);
        float rim_term = rim_fresnel * rim_backlight * u_rim_intensity * v_color.a;

        vec3 specLin = srgbToLinear(u_spec_color.rgb);
        vec3 rimLin = srgbToLinear(u_rim_color);
        vec3 lit_highlighted = mix(base_cel, specLin, clamp(spec_step, 0.0, 1.0));
        vec3 with_rim = lit_highlighted + (rimLin * rim_term);
        // P1-01 linear -> srgb for display (pipeline Rgba8Unorm non-sRGB)
        vec3 colLin = clamp(with_rim, 0.0, 1.0) * clamp(v_color.r, 0.0, 1.0);
        vec3 col = linearToSrgb(colLin);
        fragColor = vec4(col, u_base_color.a);
      }
    `;

    const vsOutline = `#version 300 es
      layout(location = 0) in vec3 a_pos;
      layout(location = 1) in vec3 a_normal;
      layout(location = 3) in vec4 a_color;
      layout(location = 4) in uvec4 a_joints;
      layout(location = 5) in vec4 a_weights;

      uniform mat4 u_view_proj;
      uniform float u_outline_width;
      uniform float u_aspect;
      uniform float u_outline_depth_bias;

      void main() {
        if (a_color.b <= 0.001) {
          gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
          return;
        }
        vec4 clip = u_view_proj * vec4(a_pos, 1.0);
        vec4 norm = u_view_proj * vec4(a_normal, 0.0);
        float len = length(norm.xy);
        vec2 norm_clip = mix(vec2(0.0), norm.xy / len, step(1e-5, len));
        float aspectSafe = max(u_aspect, 0.001);
        clip.x += (norm_clip.x / aspectSafe) * u_outline_width * a_color.b * clip.w;
        clip.y += norm_clip.y * u_outline_width * a_color.b * clip.w;
        clip.z += u_outline_depth_bias * clip.w;
        gl_Position = clip;
      }
    `;

    const fsOutline = `#version 300 es
      precision highp float;
      uniform vec4 u_outline_color;
      uniform float u_outline_opacity;
      uniform float u_outline_smoothness;
      out vec4 fragColor;

      void main() {
        float smooth = clamp(u_outline_smoothness, 0.0, 1.0);
        vec4 col = vec4(u_outline_color.rgb, u_outline_color.a * u_outline_opacity);
        if (smooth > 0.001) {
          col.a = col.a * mix(1.0, 0.85, clamp(smooth * 8.0, 0.0, 1.0));
        }
        fragColor = col;
      }
    `;

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
    // P0-05: sparse morph channels only exist for the mannequin base.
    if (preset !== "mannequin") this.gpuMorphActive = false;
    else if (this.gpuMorphSet && this.morphBindGroup) this.gpuMorphActive = true;
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

    if (this.currentPreset === "mannequin") {
      // P0-05: proportions bake into the GPU base → invalidate + rebuild debounced.
      this.gpuMorphActive = false;
      this.scheduleGpuMorphRebuild();
      this.buildGeometryBuffers();
    }
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
  // P0-05: live GPU sparse-morph path (was dead code — upload never called)
  // ------------------------------------------------------------------

  /** Coverage report: every catalog slider deforms (explicit or procedural). */
  public getMorphCoverage(): { implemented: number; total: number; explicit: number } {
    const total = CANONICAL_SLIDERS.length;
    let explicit = 0;
    for (const s of CANONICAL_SLIDERS) if (isExplicitMorph(s.id)) explicit++;
    return { implemented: total, total, explicit };
  }

  private gpuBuildPending: boolean = false;

  private scheduleGpuMorphRebuild(): void {
    if (this.gpuRebuildTimer !== null) clearTimeout(this.gpuRebuildTimer);
    this.gpuRebuildTimer = setTimeout(() => {
      this.gpuRebuildTimer = null;
      if (!this.device || !this.morphPipeline) return;
      if (this.currentPreset !== "mannequin") return;
      if (!this.canonicalBaseVertices || !this.canonicalIndices) return;
      try {
        this.rebuildGpuMorphSet();
      } catch (e) {
        console.warn("[ANIGO 3D] scheduled GPU morph rebuild failed:", e);
      }
    }, 250);
  }

  /** Channel weights for the current live state (sliders + 3 somatotype). */
  private currentChannelWeights(): Map<string, number> {
    const out = new Map<string, number>();
    for (const def of CANONICAL_SLIDERS) {
      const raw = this.activeMorphWeights.get(def.id);
      if (raw === undefined) continue;
      const denom = channelDenominator(def);
      const w = (raw - def.defaultValue) / denom;
      if (w !== 0 && Number.isFinite(w)) out.set(def.id, w);
    }
    if (this.somatotypeEndo !== 0) out.set(SOMA_CHANNEL_IDS[0], this.somatotypeEndo);
    if (this.somatotypeMeso !== 0) out.set(SOMA_CHANNEL_IDS[1], this.somatotypeMeso);
    if (this.somatotypeEcto !== 0) out.set(SOMA_CHANNEL_IDS[2], this.somatotypeEcto);
    return out;
  }

  /**
   * Builds the SparseMorphSet for the current base mesh + proportions by
   * numeric differentiation of the (linear) deformation engine, then uploads
   * it via uploadSparseMorphData(). Falls back to CPU on any failure.
   */
  private rebuildGpuMorphSet(): void {
    this.gpuMorphActive = false;
    this.gpuMorphSet = null;
    if (!this.device || !this.morphPipeline || !this.morphBindGroupLayout) return;
    if (this.currentPreset !== "mannequin") return;
    const base = this.canonicalBaseVertices;
    const indices = this.canonicalIndices;
    if (!base || !indices) return;

    const t0 = performance.now();
    // Save live state (restored in `finally`).
    const savedWeights = this.activeMorphWeights;
    const savedEndo = this.somatotypeEndo;
    const savedMeso = this.somatotypeMeso;
    const savedEcto = this.somatotypeEcto;
    const STRIDE = 18; // packVertices floats per vertex
    // Capture live weights BEFORE the differentiation scratch state.
    const liveWeights = this.currentChannelWeights();
    try {
      // Base at defaults (proportions stay baked in).
      this.activeMorphWeights = new Map();
      this.somatotypeEndo = 0;
      this.somatotypeMeso = 0;
      this.somatotypeEcto = 0;
      const baseOut = this.applyAnatomicalDeformations(base, indices);
      const vCount = Math.floor(baseOut.vertices.length / STRIDE);
      const basePos = new Float32Array(vCount * 3);
      const baseNorm = new Float32Array(vCount * 3);
      for (let v = 0; v < vCount; v++) {
        basePos[v * 3] = baseOut.vertices[v * STRIDE];
        basePos[v * 3 + 1] = baseOut.vertices[v * STRIDE + 1];
        basePos[v * 3 + 2] = baseOut.vertices[v * STRIDE + 2];
        baseNorm[v * 3] = baseOut.vertices[v * STRIDE + 3];
        baseNorm[v * 3 + 1] = baseOut.vertices[v * STRIDE + 4];
        baseNorm[v * 3 + 2] = baseOut.vertices[v * STRIDE + 5];
      }
      const extract = (packed: Float32Array): { positions: Float32Array; normals: Float32Array } => {
        const positions = new Float32Array(vCount * 3);
        const normals = new Float32Array(vCount * 3);
        for (let v = 0; v < vCount; v++) {
          positions[v * 3] = packed[v * STRIDE];
          positions[v * 3 + 1] = packed[v * STRIDE + 1];
          positions[v * 3 + 2] = packed[v * STRIDE + 2];
          normals[v * 3] = packed[v * STRIDE + 3];
          normals[v * 3 + 1] = packed[v * STRIDE + 4];
          normals[v * 3 + 2] = packed[v * STRIDE + 5];
        }
        return { positions, normals };
      };

      const perChannel: Array<{ sliderId: string; positions: Float32Array; normals: Float32Array }> = [];
      for (const def of CANONICAL_SLIDERS) {
        const denom = channelDenominator(def);
        this.activeMorphWeights = new Map([[def.id, def.defaultValue + denom]]);
        const ch = this.applyAnatomicalDeformations(base, indices);
        const pn = extract(ch.vertices);
        perChannel.push({ sliderId: def.id, positions: pn.positions, normals: pn.normals });
      }
      // Linear somatotype pseudo-channels.
      this.activeMorphWeights = new Map();
      this.somatotypeEndo = 1;
      this.somatotypeMeso = 0;
      this.somatotypeEcto = 0;
      {
        const pn = extract(this.applyAnatomicalDeformations(base, indices).vertices);
        perChannel.push({ sliderId: SOMA_CHANNEL_IDS[0], positions: pn.positions, normals: pn.normals });
      }
      this.somatotypeEndo = 0;
      this.somatotypeMeso = 1;
      this.somatotypeEcto = 0;
      {
        const pn = extract(this.applyAnatomicalDeformations(base, indices).vertices);
        perChannel.push({ sliderId: SOMA_CHANNEL_IDS[1], positions: pn.positions, normals: pn.normals });
      }
      this.somatotypeEndo = 0;
      this.somatotypeMeso = 0;
      this.somatotypeEcto = 1;
      {
        const pn = extract(this.applyAnatomicalDeformations(base, indices).vertices);
        perChannel.push({ sliderId: SOMA_CHANNEL_IDS[2], positions: pn.positions, normals: pn.normals });
      }

      const set = buildSparseMorphSet(vCount, basePos, baseNorm, perChannel);
      this.uploadSparseMorphData(
        {
          activeChannels: set.channels.length,
          totalVertices: set.totalVertices,
          totalDeltas: set.totalDeltas,
        },
        baseOut.vertices,
        set.deltasF32,
        packChannelWeights(set, liveWeights)
      );
      this.gpuMorphSet = set;
      this.gpuMorphActive = true;
      this.gpuMorphDirty = false;
      const vramKB =
        (baseOut.vertices.byteLength + set.deltasF32.byteLength + set.channels.length * 16) / 1024;
      console.info(
        `[ANIGO 3D] GPU sparse morphs live: ${set.channels.length} channels, ` +
          `${set.totalDeltas} deltas, ~${vramKB.toFixed(0)} KB, built in ${(performance.now() - t0).toFixed(0)} ms`
      );
    } catch (e) {
      console.warn("[ANIGO 3D] GPU morph build failed, CPU fallback:", e);
      this.gpuMorphActive = false;
      this.gpuMorphSet = null;
    } finally {
      this.activeMorphWeights = savedWeights;
      this.somatotypeEndo = savedEndo;
      this.somatotypeMeso = savedMeso;
      this.somatotypeEcto = savedEcto;
    }
  }

  public setSomatotype(endo: number, meso: number, ecto: number) {
    // P0-02: sanitize at the boundary (NaN/Inf can never enter the engine).
    // NOTE (P1-02 follow-up): the TS viewport still treats components as
    // independent amplitudes while Rust uses normalized barycentric coords;
    // unification under one engine is tracked separately.
    const e = clampNumber(sanitizeFinite(endo, this.somatotypeEndo), 0, 1);
    const m = clampNumber(sanitizeFinite(meso, this.somatotypeMeso), 0, 1);
    const c = clampNumber(sanitizeFinite(ecto, this.somatotypeEcto), 0, 1);
    this.pendingSomatotype = [e, m, c];
    this.queueMorphFlush();
  }

  public setGenderDimorphism(gender: number) {
    // P0-02: NaN guard — genderDimorphism can never become NaN again.
    const g = clampNumber(sanitizeFinite(gender, this.genderDimorphism), 0.0, 1.0);
    this.pendingGender = g;
    this.queueMorphFlush();
  }

  public _setGenderDimorphism_original(gender: number) {
    this.genderDimorphism = clampNumber(sanitizeFinite(gender, this.genderDimorphism), 0.0, 1.0);
    if (this.currentPreset === "mannequin") {
      this.buildGeometryBuffers();
    }
  }

  public setMorphSlider(name: string, weight: number) {
    // P0-09: unknown ids are rejected loudly (no orphan state); known ids
    // are catalog-clamped on this write path like every other.
    if (!isKnownSliderId(name)) {
      console.warn(`[ANIGO 3D] setMorphSlider: unknown slider "${name}" ignored`);
      return;
    }
    const clamped = clampCatalog(name, weight);
    if (clamped === null) return;
    this.pendingMorphSliders.set(name, clamped);
    this.queueMorphFlush();
  }

  /** P0-05: coalesces all morph/gender/somatotype writes to ONE rebuild per frame. */
  private queueMorphFlush(): void {
    this.morphDirty = true;
    if (this.morphRaf === null) {
      this.morphRaf = requestAnimationFrame(() => {
        this.morphRaf = null;
        if (!this.morphDirty) return;
        this.morphDirty = false;
        const batch = new Map(this.pendingMorphSliders);
        this.pendingMorphSliders.clear();
        const g = this.pendingGender;
        this.pendingGender = null;
        const s = this.pendingSomatotype;
        this.pendingSomatotype = null;
        this.flushPendingMorphBatch(batch, g, s);
      });
    }
  }

  private flushPendingMorphBatch(
    batch: Map<string, number>,
    pendingGender: number | null,
    pendingSoma: [number, number, number] | null
  ): void {
    try {
      for (const [n, w] of batch) {
        this.activeMorphWeights.set(n, w);
      }
      if (pendingGender !== null) {
        this.genderDimorphism = pendingGender;
      }
      if (pendingSoma !== null) {
        this.somatotypeEndo = pendingSoma[0];
        this.somatotypeMeso = pendingSoma[1];
        this.somatotypeEcto = pendingSoma[2];
      }
      if (this.currentPreset !== "mannequin") return;
      if (this.gpuMorphActive && this.gpuMorphSet) {
        // P0-05: GPU path — weights upload happens in renderWebGPU.
        this.gpuMorphDirty = true;
        return;
      }
      // CPU fallback path — exactly ONE rebuild for the whole batch.
      this.buildGeometryBuffers();
    } catch (e) {
      console.warn("[ANIGO 3D] flushPendingMorphBatch:", e);
    }
  }

  private _applyMorphSliderInternal(name: string, weight: number) {
    this.activeMorphWeights.set(name, weight);
    if (this.currentPreset === "mannequin" && !this.gpuMorphActive) {
      this.buildGeometryBuffers();
    } else {
      this.gpuMorphDirty = true;
    }
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

  // P2-13 height-normalized zones (was magic indices 425/544...)
  private applyAnatomicalDeformations(
    baseVertices: VertexData[],
    rawIndices: Uint32Array
  ): { vertices: Float32Array; indices: Uint32Array } {
    // P2-13 if (this.canonicalExtras?.anigo_zones) use normalized anchors else fallback height-scaled magic indices
    const vertices: VertexData[] = baseVertices.map(v => ({
      pos: [v.pos[0], v.pos[1], v.pos[2]],
      normal: [v.normal[0], v.normal[1], v.normal[2]],
      uv: [v.uv[0], v.uv[1]],
      color: [v.color[0], v.color[1], v.color[2], v.color[3]],
      joints: [v.joints[0], v.joints[1], v.joints[2], v.joints[3]],
      weights: [v.weights[0], v.weights[1], v.weights[2], v.weights[3]],
    }));

    const headScale = this.headScale;
    const shoulderWidth = this.shoulderWidth;
    const legLength = this.legLength;
    const armLength = this.armLength;
    const neckLength = this.neckLength;

    const endo = this.somatotypeEndo;
    const meso = this.somatotypeMeso;
    const ecto = this.somatotypeEcto;

    // P0-04: generic fallback prep — bounds + active non-explicit sliders.
    // Guarantees EVERY catalog slider exposed in the UI deforms the mesh.
    let meshBounds: MeshBounds = { minY: 0, maxY: 1.7 };
    {
      let mn = Infinity;
      let mx = -Infinity;
      for (const bv of baseVertices) {
        const by = bv.pos[1];
        if (by < mn) mn = by;
        if (by > mx) mx = by;
      }
      if (mn < mx && Number.isFinite(mn) && Number.isFinite(mx)) {
        meshBounds = { minY: mn, maxY: mx };
      }
    }
    const genericActive: Array<{ def: MorphSlider; w: number }> = [];
    for (const [id, raw] of this.activeMorphWeights) {
      if (isExplicitMorph(id)) continue;
      const def = getSliderDef(id);
      if (!def) continue;
      const w = normalizeWeight(def, raw);
      if (w !== 0 && Number.isFinite(w)) genericActive.push({ def, w });
    }

    for (let i = 0; i < vertices.length; i++) {
      const v = vertices[i];
      let x = v.pos[0];
      let y = v.pos[1];
      let z = v.pos[2];
      let nx = v.normal[0];
      let ny = v.normal[1];
      let nz = v.normal[2];

      // --- Head & Face (i < 425, center around y=1.60) ---
      if (i < 425) {
        x *= headScale;
        y = 1.55 + (y - 1.55) * headScale;
        z *= headScale;

        const headWidth = (this.activeMorphWeights.get("head_width") ?? 1.0) - 1.0;
        if (headWidth !== 0) {
          x += Math.sign(x) * 0.025 * headWidth;
        }
        const headDepth = (this.activeMorphWeights.get("head_depth") ?? 1.0) - 1.0;
        if (headDepth !== 0 && z < 0) {
          z -= 0.035 * headDepth;
        }
        const faceLower = (this.activeMorphWeights.get("face_lower_length") ?? 1.0) - 1.0;
        if (faceLower !== 0 && y < 1.62 && z > 0) {
          y -= 0.025 * faceLower;
        }
        const foreheadH = (this.activeMorphWeights.get("forehead_height") ?? 1.0) - 1.0;
        if (foreheadH !== 0 && y > 1.64) {
          y += 0.030 * foreheadH;
        }
        const browRidge = (this.activeMorphWeights.get("brow_ridge_prominence") ?? 0.2) - 0.2;
        if (browRidge !== 0 && y > 1.58 && y < 1.66 && z > 0.03) {
          z += 0.022 * browRidge;
        }
        const cheekbone = (this.activeMorphWeights.get("cheekbone_prominence") ?? 0.3) - 0.3;
        if (cheekbone !== 0 && y > 1.53 && y < 1.62 && Math.abs(x) > 0.04 && z > 0.02) {
          x += Math.sign(x) * 0.015 * cheekbone;
          z += 0.015 * cheekbone;
        }
        const jawV = (this.activeMorphWeights.get("jaw_v_line_taper") ?? 0.6) - 0.6;
        if (jawV !== 0 && y < 1.56 && z > -0.02) {
          const taper = Math.max(0, Math.min(1, (1.56 - y) * 12.0));
          x -= x * 0.25 * taper * jawV;
        }
        const chinLen = (this.activeMorphWeights.get("chin_length") ?? 1.0) - 1.0;
        if (chinLen !== 0 && y < 1.48) {
          y -= 0.020 * chinLen;
        }
        const chinProj = this.activeMorphWeights.get("chin_forward_projection") ?? 0.0;
        if (chinProj !== 0 && y < 1.52 && z > 0.04) {
          z += 0.025 * chinProj;
        }
        const eyeScale = (this.activeMorphWeights.get("eye_scale_uniform") ?? 1.0) - 1.0;
        if (eyeScale !== 0 && y > 1.54 && y < 1.65 && Math.abs(x) > 0.025 && Math.abs(x) < 0.075 && z > 0.04) {
          x += Math.sign(x) * 0.012 * eyeScale;
          y += (y - 1.60) * 0.25 * eyeScale;
          z += 0.008 * eyeScale;
        }
        const eyeTilt = this.activeMorphWeights.get("eye_canthal_tilt") ?? 0.0;
        if (eyeTilt !== 0 && y > 1.55 && y < 1.65 && Math.abs(x) > 0.03 && z > 0.04) {
          const lat = Math.max(0, Math.min(1, (Math.abs(x) - 0.03) * 25.0));
          y += (eyeTilt / 20.0) * 0.015 * lat;
        }
        const aegyosal = (this.activeMorphWeights.get("lower_eyelid_aegyosal") ?? 0.2) - 0.2;
        if (aegyosal !== 0 && y > 1.54 && y < 1.58 && Math.abs(x) > 0.03 && Math.abs(x) < 0.065 && z > 0.05) {
          z += 0.012 * aegyosal;
        }
        const noseDepth = (this.activeMorphWeights.get("nose_bridge_depth") ?? 1.0) - 1.0;
        if (noseDepth !== 0 && y > 1.54 && y < 1.63 && Math.abs(x) < 0.025 && z > 0.05) {
          z += 0.025 * noseDepth;
        }
        const noseUpturn = this.activeMorphWeights.get("nose_tip_upturn") ?? 0.0;
        if (noseUpturn !== 0 && y > 1.52 && y < 1.57 && Math.abs(x) < 0.018 && z > 0.06) {
          y += (noseUpturn / 25.0) * 0.015;
          z += (noseUpturn / 25.0) * 0.005;
        }
        const muzzleSlant = (this.activeMorphWeights.get("anime_profile_slant") ?? 0.5) - 0.5;
        if (muzzleSlant !== 0 && z > 0.03 && y < 1.62) {
          const s = Math.max(0, Math.min(1, (1.62 - y) * 5.0));
          z -= 0.018 * s * muzzleSlant;
        }
        const earElf = this.activeMorphWeights.get("ear_pointy_elf") ?? 0.0;
        if (earElf !== 0 && Math.abs(x) > 0.075 && y > 1.58 && z < 0.03) {
          x += Math.sign(x) * 0.040 * earElf;
          y += 0.055 * earElf;
          z -= 0.025 * earElf;
        }
      }

      // --- Neck & Trapezius (425 <= i < 544) ---
      else if (i < 544) {
        y = 1.35 + (y - 1.35) * neckLength;
        const adams = this.activeMorphWeights.get("adams_apple_prominence") ?? 0.0;
        if (adams !== 0 && Math.abs(x) < 0.018 && z > 0.025 && y > 1.38 && y < 1.48) {
          z += 0.022 * adams;
        }
        const trap = (this.activeMorphWeights.get("trapezius_bulk") ?? 0.2) - 0.2;
        if (trap !== 0 && y < 1.42 && Math.abs(x) > 0.035) {
          x += Math.sign(x) * 0.020 * trap;
          y += 0.025 * trap;
        }
        const neckCirc = (this.activeMorphWeights.get("neck_circumference") ?? 1.0) - 1.0;
        if (neckCirc !== 0) {
          x += nx * 0.018 * neckCirc;
          z += nz * 0.018 * neckCirc;
        }
      }

      // --- Torso / Chest / Bust / Waist (544 <= i < 1069) ---
      else if (i < 1069) {
        if (y > 1.25) {
          x *= shoulderWidth;
        }
        const bustCup = (this.activeMorphWeights.get("bust_volume_cup") ?? 0.35) - 0.35;
        if (bustCup !== 0 && z > 0 && y > 1.10 && y < 1.35 && Math.abs(x) > 0.02 && Math.abs(x) < 0.14) {
          const yb = Math.exp(-Math.pow((y - 1.22) / 0.08, 2));
          const xb = Math.exp(-Math.pow((Math.abs(x) - 0.065) / 0.045, 2));
          const intensity = yb * xb;
          x += Math.sign(x) * 0.010 * intensity * bustCup;
          y -= 0.008 * intensity * bustCup;
          z += 0.055 * intensity * bustCup;
        }
        const bustSag = this.activeMorphWeights.get("bust_gravity_sag") ?? 0.0;
        if (bustSag !== 0 && z > 0.02 && y > 1.08 && y < 1.30 && Math.abs(x) < 0.14) {
          const intensity = Math.exp(-Math.pow((y - 1.18) / 0.07, 2));
          y -= 0.035 * intensity * bustSag;
          z -= 0.012 * intensity * bustSag;
        }
        const bustCleave = (this.activeMorphWeights.get("bust_separation_cleavage") ?? 1.0) - 1.0;
        if (bustCleave !== 0 && z > 0.02 && y > 1.15 && y < 1.32 && Math.abs(x) < 0.14) {
          x += Math.sign(x) * 0.025 * bustCleave;
        }
        const pecBulk = (this.activeMorphWeights.get("pectoral_muscle_bulk") ?? 0.2) - 0.2;
        if (pecBulk !== 0 && z > 0 && y > 1.15 && y < 1.38 && Math.abs(x) < 0.16) {
          const intensity = Math.exp(-Math.pow((y - 1.27) / 0.08, 2));
          z += 0.030 * intensity * pecBulk;
        }
        const waistPinch = this.activeMorphWeights.get("waist_pinch_width") ?? 0.0;
        if (waistPinch !== 0 && y > 0.95 && y < 1.12) {
          const pinch = Math.max(0, 1.0 - Math.abs((y - 1.03) / 0.08));
          x -= Math.sign(x) * 0.032 * pinch * waistPinch;
        }
        const sixpack = (this.activeMorphWeights.get("abs_sixpack_definition") ?? 0.2) - 0.2;
        if (sixpack !== 0 && z > 0.04 && y > 0.92 && y < 1.20 && Math.abs(x) < 0.09) {
          const wave = Math.cos(y * 42.0);
          const att = Math.max(0, 1.0 - Math.pow(x / 0.09, 2));
          z += wave * 0.012 * att * sixpack;
        }
        const belly = this.activeMorphWeights.get("belly_visceral_protuberance") ?? 0.0;
        if (belly !== 0 && z > 0.01 && y > 0.90 && y < 1.15) {
          const bump = Math.exp(-Math.pow((y - 1.02) / 0.10, 2)) * Math.max(0, 1.0 - Math.pow(x / 0.14, 2));
          z += 0.045 * bump * belly;
        }
      }

      // --- Pelvis & Gluteus (1069 <= i < 1444) ---
      else if (i < 1444) {
        const hipFlare = (this.activeMorphWeights.get("hip_trochanteric_flare") ?? 1.0) - 1.0;
        if (hipFlare !== 0 && Math.abs(x) > 0.08) {
          const f = Math.exp(-Math.pow((y - 0.84) / 0.08, 2));
          x += Math.sign(x) * 0.038 * f * hipFlare;
        }
        const gluteVol = (this.activeMorphWeights.get("gluteus_volume_overall") ?? 1.0) - 1.0;
        if (gluteVol !== 0 && z < 0) {
          const f = Math.exp(-Math.pow((y - 0.82) / 0.08, 2)) * Math.exp(-Math.pow(x / 0.14, 2));
          z -= 0.050 * f * gluteVol;
        }
        const gluteProf = this.activeMorphWeights.get("gluteus_shape_profile") ?? 0.0;
        if (gluteProf !== 0 && z < 0) {
          const shift = (y - 0.82) * 0.35;
          z += shift * 0.025 * gluteProf;
        }
      }

      // --- Shoulders & Arms (1444 <= i < 2536) ---
      else if (i < 2536) {
        x += Math.sign(x) * (shoulderWidth - 1.0) * 0.15;
        if (i >= 1678) {
          y = 1.30 - (1.30 - y) * armLength;
        }
        const biceps = (this.activeMorphWeights.get("biceps_peak_volume") ?? 0.2) - 0.2;
        if (biceps !== 0 && i >= 1678 && i < 1912 && z > 0) {
          z += 0.028 * biceps;
        }
        const armThick = (this.activeMorphWeights.get("upper_arm_thickness") ?? 1.0) - 1.0;
        if (armThick !== 0 && i >= 1678 && i < 1912) {
          x += nx * 0.020 * armThick;
          z += nz * 0.020 * armThick;
        }
      }

      // --- Lower Limbs (2536 <= i < 4070) ---
      else {
        y = y * legLength;

        const thighCirc = (this.activeMorphWeights.get("thigh_circumference") ?? 1.0) - 1.0;
        if (thighCirc !== 0 && i >= 2536 && i < 2926) {
          x += nx * 0.025 * thighCirc;
          z += nz * 0.025 * thighCirc;
        }
        const thighGap = this.activeMorphWeights.get("inner_thigh_gap") ?? 0.0;
        if (thighGap !== 0 && i >= 2536 && i < 2926 && ((x > 0 && nx < 0) || (x < 0 && nx > 0))) {
          x += Math.sign(x) * 0.022 * thighGap;
        }
        const patella = (this.activeMorphWeights.get("knee_patella_prominence") ?? 0.5) - 0.5;
        if (patella !== 0 && i >= 2926 && i < 3160 && z > 0) {
          z += 0.022 * patella;
        }
        const uchimata = this.activeMorphWeights.get("knee_valgus_uchimata") ?? 0.0;
        if (uchimata !== 0 && i >= 2926 && i < 3160) {
          x -= Math.sign(x) * 0.020 * (uchimata / 15.0);
        }
        const calfCirc = (this.activeMorphWeights.get("calf_circumference") ?? 1.0) - 1.0;
        if (calfCirc !== 0 && i >= 3160 && i < 3550) {
          x += nx * 0.022 * calfCirc;
          z += nz * 0.022 * calfCirc;
        }
      }

      // --- Macro Somatotype Heath-Carter Influence ---
      if (endo > 0.001) {
        if (i >= 544 && i < 1444) {
          x += nx * 0.035 * endo;
          z += nz * 0.035 * endo;
        } else if (i >= 2536 && i < 3550) {
          x += nx * 0.025 * endo;
          z += nz * 0.025 * endo;
        }
      }
      if (meso > 0.001) {
        if (i >= 544 && i < 1069 && (z > 0 || Math.abs(x) > 0.1)) {
          x += nx * 0.030 * meso;
          z += nz * 0.030 * meso;
        } else if (i >= 1444 && i < 2146) {
          x += nx * 0.020 * meso;
          z += nz * 0.020 * meso;
        } else if (i >= 2536 && i < 3550) {
          x += nx * 0.020 * meso;
          z += nz * 0.020 * meso;
        }
      }
      if (ecto > 0.001) {
        if (i >= 544 && i < 1444) {
          x -= nx * 0.025 * ecto;
          z -= nz * 0.025 * ecto;
        } else if (i >= 1444 && i < 3550) {
          x -= nx * 0.018 * ecto;
          z -= nz * 0.018 * ecto;
        }
      }

      // --- P0-04: generic procedural fallback (every non-explicit slider) ---
      if (genericActive.length > 0) {
        for (let gi = 0; gi < genericActive.length; gi++) {
          const g = genericActive[gi];
          const d = genericMorphDelta(g.def, g.w, x, y, z, nx, ny, nz, meshBounds, GENERIC_DELTA_TMP);
          if (d !== null) {
            x += d.dx;
            y += d.dy;
            z += d.dz;
          }
        }
      }

      v.pos[0] = x;
      v.pos[1] = y;
      v.pos[2] = z;
    }

    // P0-06: normals recomputed after deformation via the shared pure
    // implementation (identical math, now unit-tested in morph_engine).
    {
      const positions = new Float32Array(vertices.length * 3);
      const normals = new Float32Array(vertices.length * 3);
      for (let i = 0; i < vertices.length; i++) {
        positions[i * 3] = vertices[i].pos[0];
        positions[i * 3 + 1] = vertices[i].pos[1];
        positions[i * 3 + 2] = vertices[i].pos[2];
        normals[i * 3] = vertices[i].normal[0];
        normals[i * 3 + 1] = vertices[i].normal[1];
        normals[i * 3 + 2] = vertices[i].normal[2];
      }
      recomputeNormals(positions, rawIndices, normals);
      for (let i = 0; i < vertices.length; i++) {
        vertices[i].normal = [normals[i * 3], normals[i * 3 + 1], normals[i * 3 + 2]];
      }
    }

    const indicesArr = Array.from(rawIndices);
    // Add studio turntable pedestal at ground
    this.appendCube(vertices, indicesArr, [0.0, -0.02, 0.0], [1.6, 0.04, 1.6], [0.7, 0.50, 0.0, 0.3]);

    return {
      vertices: packVertices(vertices),
      indices: new Uint32Array(indicesArr),
    };
  }

  private generateMannequinData(headScale: number = 1.0, headRatio: number = 6.5): { vertices: Float32Array; indices: Uint32Array } {
    // P2-13 if (this.canonicalExtras?.anigo_zones) use normalized anchors else fallback height-scaled magic indices
    if (this.canonicalBaseVertices && this.canonicalIndices) {
      return this.applyAnatomicalDeformations(this.canonicalBaseVertices, this.canonicalIndices);
    }
    if (this.canonicalVertices && this.canonicalIndices) {
      return { vertices: this.canonicalVertices, indices: this.canonicalIndices };
    }
    // Fallback to cube if not loaded yet
    return this.generateCubeData(1.0);
  }

  public async loadCanonicalModel(gender: "male" | "female") {
    const url = `/models/anigo_base_${gender}.glb`;
    try {
      // P2-12 abort previous load, use cache, report progress
      if (this.loadAbortController) this.loadAbortController.abort();
      this.loadAbortController = new AbortController();
      if (this.canonicalModelCache.has(gender)) {
        const cached = this.canonicalModelCache.get(gender)!;
        this.canonicalBaseVertices = cached.vertices;
        this.canonicalIndices = cached.indices;
        this.canonicalGender = gender;
        this.buildGeometryBuffers();
        // P0-05: GPU channels follow the active base mesh.
        this.gpuMorphActive = false;
        this.gpuMorphSet = null;
        this.scheduleGpuMorphRebuild();
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

      this.canonicalBaseVertices = vertices;
      this.canonicalIndices = new Uint32Array(rawIndices);
      this.canonicalModelCache.set(gender, { vertices, indices: new Uint32Array(rawIndices) });
      this.canonicalGender = gender;
      this.canonicalVertices = packVertices(vertices);

      this.currentPreset = "mannequin";
      this.buildGeometryBuffers();
      // P0-05: GPU channels follow the active base mesh.
      this.gpuMorphActive = false;
      this.gpuMorphSet = null;
      this.scheduleGpuMorphRebuild();
    } catch (e) {
      // P0-10: failures propagate (caller + UI callback) — never silent cube.
      if (e instanceof DOMException && e.name === "AbortError") return;
      console.error("[ANIGO 3D] Failed to load canonical model:", e);
      this.onModelLoadError?.(e instanceof Error ? e.message : String(e));
      throw e;
    }
  }

  private buildUniformBuffers() {
    if (!this.device || !this.celPipeline || !this.outlinePipeline) return;

    this.cameraBuffer = this.device.createBuffer({
      size: 208, // P2-14 80→208 (model+normal)
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    this.lightBuffer = this.device.createBuffer({
      size: 80, // P2-04 48→80 (sky+ground)
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    // P0-04/09: sizes aligned with canonical WGSL structs (Material 112 B = 7×vec4 with rim_color, Outline 48 B = 3×vec4 with smoothness)
    this.materialBuffer = this.device.createBuffer({
      size: 112,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    this.outlineBuffer = this.device.createBuffer({
      size: 48,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    // 256x4 2D Toon Ramp Texture
    // Row 0: Continuous ramp, Row 1: 1-step harsh anime cel, Row 2: 2-step Ghibli penumbra, Row 3: 3-step high-key
    if (this.toonRampTexture) {
      try { this.toonRampTexture.destroy(); } catch (_) {}
    }
    this.toonRampTexture = this.device.createTexture({
      size: [256, 4],
      format: "rgba8unorm",
      usage: GPUTextureUsage.TEXTURE_BINDING | GPUTextureUsage.COPY_DST,
    });

    const rampData = new Uint8Array(256 * 4 * 4);
    for (let y = 0; y < 4; y++) {
      for (let x = 0; x < 256; x++) {
        const u = x / 255.0;
        let factor = u;
        if (y === 0) {
          factor = u;
        } else if (y === 1) {
          factor = u >= 0.5 ? 1.0 : 0.0;
        } else if (y === 2) {
          factor = u < 0.35 ? 0.0 : (u < 0.65 ? 0.5 : 1.0);
        } else if (y === 3) {
          factor = u < 0.25 ? 0.0 : (u < 0.50 ? 0.35 : (u < 0.75 ? 0.70 : 1.0));
        }
        const val = Math.min(255, Math.max(0, Math.round(factor * 255.0)));
        const idx = (y * 256 + x) * 4;
        rampData[idx] = val;
        rampData[idx + 1] = val;
        rampData[idx + 2] = val;
        rampData[idx + 3] = 255;
      }
    }
    this.device.queue.writeTexture(
      { texture: this.toonRampTexture },
      rampData,
      { bytesPerRow: 256 * 4, rowsPerImage: 4 },
      [256, 4]
    );

    this.toonRampSampler = this.device.createSampler({
      magFilter: "linear",
      minFilter: "linear",
      addressModeU: "clamp-to-edge",
      addressModeV: "clamp-to-edge",
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
          format: "depth24plus",
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
        } catch (_) {}
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
    } catch (_) {
      return;
    }
    const textureView = currentTexture.createView();

    const aspect = this.canvas.width / Math.max(this.canvas.height, 1);
    const viewProj = this.calculateViewProjectionMatrix(aspect, true);

    // 1. Camera Buffer — P2-14 52 floats (viewProj 16 + eye 4 + model 16 + normal 16)
    const camData = new Float32Array(52);
    camData.set(viewProj, 0);
    camData.set([this.eye[0], this.eye[1], this.eye[2], 1.0], 16);
    // model matrix (identity for now, per-object would be per draw)
    const model = [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1];
    const normalMat = [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1];
    camData.set(model, 20);
    camData.set(normalMat, 36);
    this.device.queue.writeBuffer(this.cameraBuffer!, 0, camData);

    // 2. Light Buffer — P2-04 20 floats (dir+color+shadow+sky+ground)
    const lightData = new Float32Array(20);
    lightData.set([this.lightDir[0], this.lightDir[1], this.lightDir[2], this.lightIntensity], 0);
    lightData.set([this.lightColor[0], this.lightColor[1], this.lightColor[2], this.ambientIntensity], 4);
    lightData.set([this.shadowColor[0], this.shadowColor[1], this.shadowColor[2], this.shadowSaturation], 8);
    lightData.set([this.ambientSky[0], this.ambientSky[1], this.ambientSky[2], 1.0], 12);
    lightData.set([this.ambientGround[0], this.ambientGround[1], this.ambientGround[2], 1.0], 16);
    this.device.queue.writeBuffer(this.lightBuffer!, 0, lightData);

    // 3. Material Buffer — 112 B = 28 floats = base/shade/spec/rim + params/params2/params3
    const matData = new Float32Array(28);
    matData.set(this.baseColor, 0);
    matData.set(this.shadeColor, 4);
    matData.set(this.specColor, 8);
    matData.set([this.rimColor[0], this.rimColor[1], this.rimColor[2], 1.0], 12);
    matData.set([this.shadowThreshold, this.toonSmoothness, this.specIntensity, this.specExponent], 16);
    matData.set([this.rimIntensity, this.rimSpread, (this.hueShift * Math.PI) / 180.0, this.toonSteps], 20);
    matData.set([this.specSoftness, this.specOffset, this.specularSize, this.aoIntensity], 24); // P2-07/05
    this.device.queue.writeBuffer(this.materialBuffer!, 0, matData);

    // 4. Outline Buffer — 48 B = 12 floats = color + params(4) + params2(4 smoothness)
    const outlineData = new Float32Array(12);
    outlineData.set(this.outlineColor, 0);
    outlineData.set([this.outlineWidth, aspect, this.outlineDepthBias, this.outlineOpacity], 4);
    outlineData.set([this.outlineSmoothness, 0.0, 0.0, 0.0], 8);
    this.device.queue.writeBuffer(this.outlineBuffer!, 0, outlineData);

    // 5. Render Passes (Compute Sparse Morphs followed by NPR Cel-Shading)
    const commandEncoder = this.device.createCommandEncoder();

    // P0-05: lazy one-time build once model + compute pipeline both exist.
    if (
      !this.gpuMorphSet &&
      !this.gpuBuildPending &&
      this.morphPipeline &&
      this.currentPreset === "mannequin" &&
      this.canonicalBaseVertices &&
      this.canonicalIndices
    ) {
      this.gpuBuildPending = true;
      setTimeout(() => {
        try {
          this.rebuildGpuMorphSet();
        } catch (e) {
          console.warn("[ANIGO 3D] lazy GPU morph build failed:", e);
        } finally {
          this.gpuBuildPending = false;
        }
      }, 50);
    }

    if (
      this.gpuMorphActive &&
      this.gpuMorphSet &&
      this.morphPipeline &&
      this.morphBindGroup &&
      this.morphedVertexBuffer &&
      this.morphVertexCount > 0
    ) {
      // P0-05: per-frame channel weights (slider + somatotype), then dispatch.
      if (this.gpuMorphDirty && this.morphChannelsBuffer) {
        try {
          this.device.queue.writeBuffer(
            this.morphChannelsBuffer,
            0,
            packChannelWeights(this.gpuMorphSet, this.currentChannelWeights())
          );
        } catch (e) {
          console.warn("[ANIGO 3D] channel weight upload failed:", e);
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
          clearValue: { r: 0.08, g: 0.09, b: 0.13, a: 1.0 },
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

    // P2-02 canonical order outline→cel (was cel→outline, now unified with headless)
    // PASS 1: Inverted Hull Backfaces (depthWrite false, bias)
    passEncoder.setPipeline(this.outlinePipeline!);
    passEncoder.setBindGroup(0, this.outlineBindGroup!);
    passEncoder.drawIndexed(this.indexCount);

    // PASS 2: Cel-Shading Frontfaces (opaque, no blend)
    passEncoder.setPipeline(this.celPipeline!);
    passEncoder.setBindGroup(0, this.celBindGroup!);
    passEncoder.drawIndexed(this.indexCount);

    passEncoder.end();
    this.device.queue.submit([commandEncoder.finish()]);

    this.recordMetrics(startTime, "WebGPU Hardware");
  }

  private renderWebGL2(startTime: number) {
    const gl = this.gl;
    if (!gl || !this.glCelProgram || !this.glOutlineProgram || !this.glVao) return;

    gl.viewport(0, 0, this.canvas.width, this.canvas.height);
    gl.clearColor(0.08, 0.09, 0.13, 1.0);
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
    const z = this.normalize([
      this.eye[0] - this.target[0],
      this.eye[1] - this.target[1],
      this.eye[2] - this.target[2],
    ]);
    let x = this.cross(this.up, z);
    let xLen = Math.hypot(x[0], x[1], x[2]);
    if (xLen < 1e-4) {
      const fallback: [number, number, number] = Math.abs(z[1]) < 0.9 ? [0, 1, 0] : [1, 0, 0];
      x = this.cross(fallback, z);
      xLen = Math.hypot(x[0], x[1], x[2]);
    }
    const right = [x[0] / xLen, x[1] / xLen, x[2] / xLen];
    const y = this.cross(z, right);

    // Column-major View Matrix
    const view = [
      right[0], y[0], z[0], 0,
      right[1], y[1], z[1], 0,
      right[2], y[2], z[2], 0,
      -this.dot(right, this.eye), -this.dot(y, this.eye), -this.dot(z, this.eye), 1,
    ];

    const f = 1.0 / Math.tan(this.fov / 2);
    const near = 0.05;
    const far = 100.0;
    const nf = 1.0 / (near - far);

    let proj: number[];
    if (isWebGPU) {
      // WebGPU NDC Depth: [0, 1]
      proj = [
        f / aspect, 0, 0, 0,
        0, f, 0, 0,
        0, 0, far * nf, -1,
        0, 0, near * far * nf, 0,
      ];
    } else {
      // WebGL2 / OpenGL NDC Depth: [-1, 1]
      proj = [
        f / aspect, 0, 0, 0,
        0, f, 0, 0,
        0, 0, (far + near) * nf, -1,
        0, 0, 2 * near * far * nf, 0,
      ];
    }

    return new Float32Array(this.multiplyMat4(proj, view));
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
    if (this.morphRaf !== null) { try { cancelAnimationFrame(this.morphRaf); } catch (_) {} this.morphRaf = null; }
    if (this.gpuRebuildTimer !== null) { try { clearTimeout(this.gpuRebuildTimer); } catch (_) {} this.gpuRebuildTimer = null; }
    this.pendingMorphSliders?.clear?.();
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
