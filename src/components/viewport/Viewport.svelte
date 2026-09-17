<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { WebGpuViewportRenderer, type ViewportMetrics } from "./webgpu_renderer";

  let canvas: HTMLCanvasElement | null = $state(null);
  let containerEl: HTMLElement | null = $state(null);
  let renderer: WebGpuViewportRenderer | null = null;

  // Viewport Metrics
  let fps = $state(120);
  let renderTimeMs = $state(0.45);
  let triangles = $state(156);
  let drawCalls = $state(2);
  let gpuInfo = $state("WebGPU / Vulkan Hardware");
  let webgpuActive = $state(false);

  // Mouse Interaction State
  let isDragging = $state(false);
  let lastMouseX = $state(0);
  let lastMouseY = $state(0);
  let buttonPressed = $state(0);

  function handleMouseDown(e: MouseEvent) {
    isDragging = true;
    lastMouseX = e.clientX;
    lastMouseY = e.clientY;
    buttonPressed = e.button;
  }

  function handleMouseMove(e: MouseEvent) {
    if (!isDragging || !renderer) return;
    const deltaX = e.clientX - lastMouseX;
    const deltaY = e.clientY - lastMouseY;
    lastMouseX = e.clientX;
    lastMouseY = e.clientY;

    if (buttonPressed === 0) {
      // Left Click: Orbit
      const azimuthDelta = -deltaX * 0.008;
      const elevationDelta = -deltaY * 0.008;
      renderer.orbit(azimuthDelta, elevationDelta);
    } else if (buttonPressed === 1 || buttonPressed === 2) {
      // Right or Middle Click: Pan
      renderer.pan(-deltaX * 0.003, deltaY * 0.003);
    }
  }

  function handleMouseUp() {
    isDragging = false;
  }

  function handleWheel(e: WheelEvent) {
    e.preventDefault();
    if (!renderer) return;
    const factor = e.deltaY > 0 ? 1.08 : 0.92;
    renderer.zoom(factor);
  }

  export function orbit(azimuth: number, elevation: number) {
    if (renderer) renderer.orbit(azimuth, elevation);
  }

  export function zoom(factor: number) {
    if (renderer) renderer.zoom(factor);
  }

  export function setLight(dir: [number, number, number], intensity: number) {
    if (renderer) {
      renderer.lightDir = dir;
      renderer.lightIntensity = intensity;
    }
  }

  export function setOutlineWidth(width: number) {
    if (renderer) renderer.outlineWidth = width * 0.001;
  }

  export function setShadowThreshold(threshold: number) {
    if (renderer) renderer.shadowThreshold = threshold;
  }

  onMount(async () => {
    if (canvas && containerEl) {
      renderer = new WebGpuViewportRenderer(canvas);
      renderer.onMetricsUpdate = (m: ViewportMetrics) => {
        fps = m.fps;
        renderTimeMs = m.frameTimeMs;
        triangles = m.triangles;
        drawCalls = m.drawCalls;
        gpuInfo = m.adapterName;
      };

      const success = await renderer.initialize();
      webgpuActive = success;

      // Handle window resizing
      const resizeObserver = new ResizeObserver((entries) => {
        for (const entry of entries) {
          const { width, height } = entry.contentRect;
          if (renderer && width > 0 && height > 0) {
            renderer.resize(Math.floor(width), Math.floor(height));
          }
        }
      });
      resizeObserver.observe(containerEl);

      // Listen for Tauri live bridge events (MCP live control)
      if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
        try {
          const { listen } = await import("@tauri-apps/api/event");
          await listen("anigo://camera_orbit", (event: any) => {
            if (renderer && event.payload) {
              renderer.orbit(event.payload.azimuth || 0, event.payload.elevation || 0);
            }
          });
          await listen("anigo://camera_zoom", (event: any) => {
            if (renderer && event.payload) {
              renderer.zoom(event.payload.factor || 1.0);
            }
          });
          await listen("anigo://set_light", (event: any) => {
            if (renderer && event.payload) {
              renderer.lightDir = event.payload.direction || renderer.lightDir;
              renderer.lightIntensity = event.payload.intensity ?? renderer.lightIntensity;
            }
          });
        } catch (e) {
          console.warn("[Tauri Live Bridge] Event listener initialization skipped:", e);
        }
      }
    }
  });

  onDestroy(() => {
    if (renderer) renderer.destroy();
  });
</script>

<div
  bind:this={containerEl}
  class="viewport-container"
  role="region"
  aria-label="ANIGO 3D Native WebGPU Viewport"
  onmousedown={handleMouseDown}
  onmousemove={handleMouseMove}
  onmouseup={handleMouseUp}
  onmouseleave={handleMouseUp}
  onwheel={handleWheel}
  oncontextmenu={(e) => e.preventDefault()}
>
  <!-- Real Hardware WebGPU Canvas -->
  <canvas bind:this={canvas} class="viewport-canvas"></canvas>

  <!-- Viewport HUD Overlay -->
  <div class="hud-overlay">
    <div class="hud-item badge">
      {webgpuActive ? "WEBGPU NATIVO (120+ FPS)" : "CARREGANDO MOTOR WEBGPU..."}
    </div>
    <div class="hud-item">FPS: <span class="val">{fps}</span></div>
    <div class="hud-item">GPU: <span class="val">{gpuInfo}</span></div>
    <div class="hud-item">Passe GPU: <span class="val">{renderTimeMs.toFixed(2)} ms</span></div>
    <div class="hud-item">Triângulos: <span class="val">{triangles.toLocaleString()}</span></div>
    <div class="hud-item">Draw Calls: <span class="val">{drawCalls} (Hull + Cel)</span></div>
  </div>
</div>

<style>
  .viewport-container {
    position: relative;
    width: 100%;
    height: 100%;
    background: radial-gradient(circle at center, #1e2230 0%, #0d0f15 100%);
    overflow: hidden;
    cursor: grab;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .viewport-container:active {
    cursor: grabbing;
  }
  .viewport-canvas {
    width: 100%;
    height: 100%;
    display: block;
  }
  .hud-overlay {
    position: absolute;
    top: 12px;
    left: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    background: rgba(15, 17, 23, 0.85);
    backdrop-filter: blur(8px);
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 6px;
    padding: 8px 12px;
    font-family: monospace;
    font-size: 0.78rem;
    color: #94a3b8;
    pointer-events: none;
  }
  .hud-item .val {
    color: #38bdf8;
    font-weight: bold;
  }
  .badge {
    color: #ec4899;
    font-weight: 700;
    letter-spacing: 1px;
  }
</style>
