// ANIGO Native WebGPU Real-Time Renderer
// Directly runs WGSL Cel-Shading & Inverted Hull pipelines on GPU canvas at 120+ FPS

export interface ViewportMetrics {
  fps: number;
  frameTimeMs: number;
  triangles: number;
  drawCalls: number;
  adapterName: string;
}

export class WebGpuViewportRenderer {
  private canvas: HTMLCanvasElement;
  private adapter: GPUAdapter | null = null;
  private device: GPUDevice | null = null;
  private context: GPUCanvasContext | null = null;
  private format: GPUTextureFormat = "bgra8unorm";

  private depthTexture: GPUTexture | null = null;
  private depthView: GPUTextureView | null = null;

  private celPipeline: GPURenderPipeline | null = null;
  private outlinePipeline: GPURenderPipeline | null = null;

  private vertexBuffer: GPUBuffer | null = null;
  private indexBuffer: GPUBuffer | null = null;
  private indexCount: number = 0;

  private cameraBuffer: GPUBuffer | null = null;
  private lightBuffer: GPUBuffer | null = null;
  private materialBuffer: GPUBuffer | null = null;
  private outlineBuffer: GPUBuffer | null = null;

  private celBindGroup: GPUBindGroup | null = null;
  private outlineBindGroup: GPUBindGroup | null = null;

  // Camera State
  public eye: [number, number, number] = [0.0, 1.5, 3.5];
  public target: [number, number, number] = [0.0, 1.0, 0.0];
  public up: [number, number, number] = [0.0, 1.0, 0.0];
  public fov: number = (45.0 * Math.PI) / 180.0;

  // Light State
  public lightDir: [number, number, number] = [0.577, 0.577, 0.577];
  public lightColor: [number, number, number] = [1.0, 0.98, 0.95];
  public lightIntensity: number = 1.0;
  public shadowColor: [number, number, number] = [0.65, 0.68, 0.85];

  // Material State
  public baseColor: [number, number, number, number] = [0.98, 0.92, 0.85, 1.0];
  public shadeColor: [number, number, number, number] = [0.82, 0.73, 0.78, 1.0];
  public outlineColor: [number, number, number, number] = [0.25, 0.15, 0.20, 1.0];
  public outlineWidth: number = 0.0035;
  public shadowThreshold: number = 0.50;

  private animationFrameId: number | null = null;
  private lastTime: number = performance.now();
  private frameCounter: number = 0;
  private fpsTimer: number = 0;
  public onMetricsUpdate?: (metrics: ViewportMetrics) => void;

  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
  }

  public async initialize(): Promise<boolean> {
    if (!navigator.gpu) {
      console.warn("[WebGPU] navigator.gpu not available in this webview context.");
      return false;
    }

    try {
      this.adapter = await navigator.gpu.requestAdapter({
        powerPreference: "high-performance",
      });
      if (!this.adapter) return false;

      this.device = await this.adapter.requestDevice();
      this.context = this.canvas.getContext("webgpu");
      if (!this.context) return false;

      this.format = navigator.gpu.getPreferredCanvasFormat();
      this.context.configure({
        device: this.device,
        format: this.format,
        alphaMode: "premultiplied",
      });

      this.buildShadersAndPipelines();
      this.buildGeometryBuffers();
      this.buildUniformBuffers();
      this.resize(this.canvas.clientWidth, this.canvas.clientHeight);
      this.startRenderLoop();

      return true;
    } catch (e) {
      console.error("[WebGPU] Initialization error:", e);
      return false;
    }
  }

  private buildShadersAndPipelines() {
    if (!this.device) return;

    const celShaderCode = `
      struct CameraUniform {
          view_proj: mat4x4<f32>,
          camera_pos: vec4<f32>,
      };
      struct LightUniform {
          direction: vec4<f32>,
          color: vec4<f32>,
          shadow_color: vec4<f32>,
      };
      struct MaterialUniform {
          base_color: vec4<f32>,
          shade_color: vec4<f32>,
          params: vec4<f32>,
      };

      @group(0) @binding(0) var<uniform> camera: CameraUniform;
      @group(0) @binding(1) var<uniform> light: LightUniform;
      @group(0) @binding(2) var<uniform> material: MaterialUniform;

      struct VertexInput {
          @location(0) position: vec3<f32>,
          @location(1) normal: vec3<f32>,
          @location(2) uv: vec2<f32>,
          @location(3) color: vec4<f32>,
      };
      struct VertexOutput {
          @builtin(position) clip_position: vec4<f32>,
          @location(0) world_normal: vec3<f32>,
          @location(1) world_position: vec3<f32>,
          @location(2) uv: vec2<f32>,
          @location(3) anime_attr: vec4<f32>,
      };

      @vertex
      fn vs_main(in: VertexInput) -> VertexOutput {
          var out: VertexOutput;
          let world_pos = vec4<f32>(in.position, 1.0);
          out.clip_position = camera.view_proj * world_pos;
          out.world_position = in.position;
          out.world_normal = normalize(in.normal);
          out.uv = in.uv;
          out.anime_attr = in.color;
          return out;
      }

      @fragment
      fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
          let N = normalize(in.world_normal);
          let L = normalize(light.direction.xyz);
          let n_dot_l = dot(N, L);
          let half_lambert = n_dot_l * 0.5 + 0.5;

          let shadow_shift = (in.anime_attr.g - 0.5) * 0.3;
          let threshold = material.params.x + shadow_shift;
          let smoothness = max(material.params.y, 0.001);

          let toon_factor = smoothstep(threshold - smoothness, threshold + smoothness, half_lambert);
          let ao = in.anime_attr.r;

          let lit_color = material.base_color.rgb * light.color.rgb;
          let shadow_tint = material.shade_color.rgb * light.shadow_color.rgb;
          let ambient_light = material.shade_color.rgb * light.color.w * ao;

          let final_rgb = mix(shadow_tint + ambient_light, lit_color, toon_factor) * ao;
          return vec4<f32>(final_rgb, material.base_color.a);
      }
    `;

    const outlineShaderCode = `
      struct CameraUniform {
          view_proj: mat4x4<f32>,
          camera_pos: vec4<f32>,
      };
      struct OutlineUniform {
          color: vec4<f32>,
          params: vec4<f32>,
      };

      @group(0) @binding(0) var<uniform> camera: CameraUniform;
      @group(0) @binding(1) var<uniform> outline: OutlineUniform;

      struct VertexInput {
          @location(0) position: vec3<f32>,
          @location(1) normal: vec3<f32>,
          @location(2) uv: vec2<f32>,
          @location(3) color: vec4<f32>,
      };
      struct VertexOutput {
          @builtin(position) clip_position: vec4<f32>,
      };

      @vertex
      fn vs_main(in: VertexInput) -> VertexOutput {
          var out: VertexOutput;
          let world_pos = vec4<f32>(in.position, 1.0);
          var clip_pos = camera.view_proj * world_pos;

          let normal_vec4 = camera.view_proj * vec4<f32>(in.normal, 0.0);
          let normal_clip = normalize(normal_vec4.xy);

          let thickness = outline.params.x * in.color.b;
          clip_pos.x += normal_clip.x * thickness * clip_pos.w;
          clip_pos.y += normal_clip.y * thickness * clip_pos.w;

          out.clip_position = clip_pos;
          return out;
      }

      @fragment
      fn fs_main() -> @location(0) vec4<f32> {
          return outline.color;
      }
    `;

    const celModule = this.device.createShaderModule({ code: celShaderCode });
    const outlineModule = this.device.createShaderModule({ code: outlineShaderCode });

    const vertexBufferLayout: GPUVertexBufferLayout = {
      arrayStride: 48, // 12 floats * 4 bytes
      attributes: [
        { shaderLocation: 0, offset: 0, format: "float32x3" },  // position
        { shaderLocation: 1, offset: 12, format: "float32x3" }, // normal
        { shaderLocation: 2, offset: 24, format: "float32x2" }, // uv
        { shaderLocation: 3, offset: 32, format: "float32x4" }, // anime attributes (AO, shadow, outline, spec)
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
        targets: [{ format: this.format }],
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
        targets: [{ format: this.format }],
      },
      primitive: {
        topology: "triangle-list",
        cullMode: "front", // Inverted Hull: cull front, draw back
      },
      depthStencil: {
        format: "depth24plus",
        depthWriteEnabled: true,
        depthCompare: "less-equal",
      },
    });
  }

  private buildGeometryBuffers() {
    if (!this.device) return;

    // Procedural Anime Mannequin
    const { vertices, indices } = this.generateMannequinData();

    this.vertexBuffer = this.device.createBuffer({
      size: vertices.byteLength,
      usage: GPUBufferUsage.VERTEX | GPUBufferUsage.COPY_DST,
    });
    this.device.queue.writeBuffer(this.vertexBuffer, 0, vertices);

    this.indexBuffer = this.device.createBuffer({
      size: indices.byteLength,
      usage: GPUBufferUsage.INDEX | GPUBufferUsage.COPY_DST,
    });
    this.device.queue.writeBuffer(this.indexBuffer, 0, indices);
    this.indexCount = indices.length;
  }

  private generateMannequinData(): { vertices: Float32Array; indices: Uint32Array } {
    const rawVerts: number[] = [];
    const rawIndices: number[] = [];

    const segments = [
      { center: [0.0, 1.65, 0.0], dims: [0.28, 0.32, 0.28], color: [1.0, 0.48, 1.2, 1.0] },
      { center: [0.0, 1.45, 0.0], dims: [0.10, 0.12, 0.10], color: [0.9, 0.50, 1.0, 1.0] },
      { center: [0.0, 1.25, 0.0], dims: [0.38, 0.30, 0.24], color: [1.0, 0.50, 1.0, 1.0] },
      { center: [0.0, 0.95, 0.0], dims: [0.34, 0.25, 0.22], color: [1.0, 0.50, 1.0, 1.0] },
      { center: [-0.28, 1.20, 0.0], dims: [0.10, 0.32, 0.10], color: [0.95, 0.50, 1.0, 0.8] },
      { center: [0.28, 1.20, 0.0], dims: [0.10, 0.32, 0.10], color: [0.95, 0.50, 1.0, 0.8] },
      { center: [-0.28, 0.82, 0.0], dims: [0.08, 0.30, 0.08], color: [0.95, 0.50, 1.0, 0.8] },
      { center: [0.28, 0.82, 0.0], dims: [0.08, 0.30, 0.08], color: [0.95, 0.50, 1.0, 0.8] },
      { center: [-0.12, 0.65, 0.0], dims: [0.14, 0.38, 0.14], color: [1.0, 0.50, 1.0, 0.9] },
      { center: [0.12, 0.65, 0.0], dims: [0.14, 0.38, 0.14], color: [1.0, 0.50, 1.0, 0.9] },
      { center: [-0.12, 0.25, 0.0], dims: [0.11, 0.40, 0.11], color: [1.0, 0.50, 1.0, 0.9] },
      { center: [0.12, 0.25, 0.0], dims: [0.11, 0.40, 0.11], color: [1.0, 0.50, 1.0, 0.9] },
      { center: [0.0, -0.02, 0.0], dims: [1.2, 0.04, 1.2], color: [0.7, 0.50, 0.0, 0.3] },
    ];

    const cubeFaces = [
      { normal: [0, 0, 1], corners: [[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]] },
      { normal: [0, 0, -1], corners: [[0.5, -0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5]] },
      { normal: [0, 1, 0], corners: [[-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5]] },
      { normal: [0, -1, 0], corners: [[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [-0.5, -0.5, 0.5]] },
      { normal: [1, 0, 0], corners: [[0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5]] },
      { normal: [-1, 0, 0], corners: [[-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, -0.5]] },
    ];

    for (const seg of segments) {
      for (const face of cubeFaces) {
        const baseIdx = rawVerts.length / 12;
        for (const pt of face.corners) {
          // Pos
          rawVerts.push(
            seg.center[0] + pt[0] * seg.dims[0],
            seg.center[1] + pt[1] * seg.dims[1],
            seg.center[2] + pt[2] * seg.dims[2]
          );
          // Normal
          rawVerts.push(face.normal[0], face.normal[1], face.normal[2]);
          // UV
          rawVerts.push(0.0, 0.0);
          // Color / Anime attributes
          rawVerts.push(seg.color[0], seg.color[1], seg.color[2], seg.color[3]);
        }
        rawIndices.push(baseIdx, baseIdx + 1, baseIdx + 2, baseIdx, baseIdx + 2, baseIdx + 3);
      }
    }

    return {
      vertices: new Float32Array(rawVerts),
      indices: new Uint32Array(rawIndices),
    };
  }

  private buildUniformBuffers() {
    if (!this.device || !this.celPipeline || !this.outlinePipeline) return;

    this.cameraBuffer = this.device.createBuffer({
      size: 80, // mat4 (64) + vec4 (16)
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    this.lightBuffer = this.device.createBuffer({
      size: 48,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    this.materialBuffer = this.device.createBuffer({
      size: 48,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    this.outlineBuffer = this.device.createBuffer({
      size: 32,
      usage: GPUBufferUsage.UNIFORM | GPUBufferUsage.COPY_DST,
    });

    this.celBindGroup = this.device.createBindGroup({
      layout: this.celPipeline.getBindGroupLayout(0),
      entries: [
        { binding: 0, resource: { buffer: this.cameraBuffer } },
        { binding: 1, resource: { buffer: this.lightBuffer } },
        { binding: 2, resource: { buffer: this.materialBuffer } },
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

  public resize(width: number, height: number) {
    if (!this.device || width <= 0 || height <= 0) return;
    this.canvas.width = width;
    this.canvas.height = height;

    if (this.depthTexture) this.depthTexture.destroy();

    this.depthTexture = this.device.createTexture({
      size: [width, height],
      format: "depth24plus",
      usage: GPUTextureUsage.RENDER_ATTACHMENT,
    });
    this.depthView = this.depthTexture.createView();
  }

  // Camera Orbit, Zoom, Pan
  public orbit(deltaAzimuth: number, deltaElevation: number) {
    const dx = this.eye[0] - this.target[0];
    const dy = this.eye[1] - this.target[1];
    const dz = this.eye[2] - this.target[2];
    const radius = Math.hypot(dx, dy, dz);
    if (radius < 0.001) return;

    let azimuth = Math.atan2(dz, dx);
    let elevation = Math.asin(Math.max(-1, Math.min(1, dy / radius)));

    azimuth += deltaAzimuth;
    elevation = Math.max(-1.5, Math.min(1.5, elevation + deltaElevation));

    this.eye[0] = this.target[0] + radius * Math.cos(elevation) * Math.cos(azimuth);
    this.eye[1] = this.target[1] + radius * Math.sin(elevation);
    this.eye[2] = this.target[2] + radius * Math.cos(elevation) * Math.sin(azimuth);
  }

  public zoom(factor: number) {
    const dx = this.eye[0] - this.target[0];
    const dy = this.eye[1] - this.target[1];
    const dz = this.eye[2] - this.target[2];
    const radius = Math.hypot(dx, dy, dz);
    const newRadius = Math.max(0.3, Math.min(30.0, radius * factor));
    const norm = newRadius / radius;
    this.eye[0] = this.target[0] + dx * norm;
    this.eye[1] = this.target[1] + dy * norm;
    this.eye[2] = this.target[2] + dz * norm;
  }

  public pan(dx: number, dy: number) {
    const fx = this.target[0] - this.eye[0];
    const fy = this.target[1] - this.eye[1];
    const fz = this.target[2] - this.eye[2];
    const fLen = Math.hypot(fx, fy, fz);
    const fNorm = [fx / fLen, fy / fLen, fz / fLen];

    // Right = forward x up
    const rx = fNorm[1] * this.up[2] - fNorm[2] * this.up[1];
    const ry = fNorm[2] * this.up[0] - fNorm[0] * this.up[2];
    const rz = fNorm[0] * this.up[1] - fNorm[1] * this.up[0];

    const ux = ry * fNorm[2] - rz * fNorm[1];
    const uy = rz * fNorm[0] - rx * fNorm[2];
    const uz = rx * fNorm[1] - ry * fNorm[0];

    const shiftX = rx * dx + ux * dy;
    const shiftY = ry * dx + uy * dy;
    const shiftZ = rz * dx + uz * dy;

    this.eye[0] += shiftX;
    this.eye[1] += shiftY;
    this.eye[2] += shiftZ;
    this.target[0] += shiftX;
    this.target[1] += shiftY;
    this.target[2] += shiftZ;
  }

  private startRenderLoop() {
    const frame = () => {
      this.render();
      this.animationFrameId = requestAnimationFrame(frame);
    };
    this.animationFrameId = requestAnimationFrame(frame);
  }

  public render() {
    if (!this.device || !this.context || !this.depthView) return;
    const startTime = performance.now();

    // 1. Update Uniforms
    const aspect = this.canvas.width / Math.max(this.canvas.height, 1);
    const viewProj = this.calculateViewProjectionMatrix(aspect);

    // Camera buffer
    const camData = new Float32Array(20);
    camData.set(viewProj, 0);
    camData.set([this.eye[0], this.eye[1], this.eye[2], 1.0], 16);
    this.device.queue.writeBuffer(this.cameraBuffer!, 0, camData);

    // Light buffer
    const lightData = new Float32Array(12);
    lightData.set([this.lightDir[0], this.lightDir[1], this.lightDir[2], this.lightIntensity], 0);
    lightData.set([this.lightColor[0], this.lightColor[1], this.lightColor[2], 0.35], 4);
    lightData.set([this.shadowColor[0], this.shadowColor[1], this.shadowColor[2], 1.0], 8);
    this.device.queue.writeBuffer(this.lightBuffer!, 0, lightData);

    // Material buffer
    const matData = new Float32Array(12);
    matData.set(this.baseColor, 0);
    matData.set(this.shadeColor, 4);
    matData.set([this.shadowThreshold, 0.04, 0.0, 0.0], 8);
    this.device.queue.writeBuffer(this.materialBuffer!, 0, matData);

    // Outline buffer
    const outlineData = new Float32Array(8);
    outlineData.set(this.outlineColor, 0);
    outlineData.set([this.outlineWidth, 1.0, 0.0, 0.0], 4);
    this.device.queue.writeBuffer(this.outlineBuffer!, 0, outlineData);

    // 2. Command Encoding
    const commandEncoder = this.device.createCommandEncoder();
    const textureView = this.context.getCurrentTexture().createView();

    const passEncoder = commandEncoder.beginRenderPass({
      colorAttachments: [
        {
          view: textureView,
          clearValue: { r: 0.12, g: 0.13, b: 0.16, a: 1.0 },
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

    passEncoder.setVertexBuffer(0, this.vertexBuffer!);
    passEncoder.setIndexBuffer(this.indexBuffer!, "uint32");

    // Pass 1: Inverted Hull (backface lineart)
    passEncoder.setPipeline(this.outlinePipeline!);
    passEncoder.setBindGroup(0, this.outlineBindGroup!);
    passEncoder.drawIndexed(this.indexCount);

    // Pass 2: Cel-Shading NPR Surfaces
    passEncoder.setPipeline(this.celPipeline!);
    passEncoder.setBindGroup(0, this.celBindGroup!);
    passEncoder.drawIndexed(this.indexCount);

    passEncoder.end();
    this.device.queue.submit([commandEncoder.finish()]);

    // Metrics
    const elapsed = performance.now() - startTime;
    this.frameCounter++;
    if (performance.now() - this.fpsTimer >= 500) {
      const currentFps = Math.round((this.frameCounter * 1000) / (performance.now() - this.fpsTimer));
      this.frameCounter = 0;
      this.fpsTimer = performance.now();
      if (this.onMetricsUpdate) {
        this.onMetricsUpdate({
          fps: currentFps,
          frameTimeMs: elapsed,
          triangles: this.indexCount / 3,
          drawCalls: 2,
          adapterName: this.adapter?.info.device || "WebGPU Hardware (Dawn/Vulkan)",
        });
      }
    }
  }

  private calculateViewProjectionMatrix(aspect: number): Float32Array {
    // Standard LookAt Matrix
    const z = this.normalize([
      this.eye[0] - this.target[0],
      this.eye[1] - this.target[1],
      this.eye[2] - this.target[2],
    ]);
    const x = this.normalize(this.cross(this.up, z));
    const y = this.cross(z, x);

    const view = [
      x[0], y[0], z[0], 0,
      x[1], y[1], z[1], 0,
      x[2], y[2], z[2], 0,
      -this.dot(x, this.eye), -this.dot(y, this.eye), -this.dot(z, this.eye), 1,
    ];

    // Perspective Matrix
    const f = 1.0 / Math.tan(this.fov / 2);
    const near = 0.05;
    const far = 100.0;
    const nf = 1.0 / (near - far);

    const proj = [
      f / aspect, 0, 0, 0,
      0, f, 0, 0,
      0, 0, far * nf, -1,
      0, 0, near * far * nf, 0,
    ];

    return new Float32Array(this.multiplyMat4(proj, view));
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

  private multiplyMat4(a: number[], b: number[]): number[] {
    const out = new Array(16).fill(0);
    for (let r = 0; r < 4; r++) {
      for (let c = 0; c < 4; c++) {
        out[r * 4 + c] =
          a[r * 4 + 0] * b[0 * 4 + c] +
          a[r * 4 + 1] * b[1 * 4 + c] +
          a[r * 4 + 2] * b[2 * 4 + c] +
          a[r * 4 + 3] * b[3 * 4 + c];
      }
    }
    return out;
  }

  public destroy() {
    if (this.animationFrameId !== null) {
      cancelAnimationFrame(this.animationFrameId);
    }
    if (this.depthTexture) this.depthTexture.destroy();
  }
}
