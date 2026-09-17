<script lang="ts">
  import { onMount } from "svelte";

  // State props
  let canvas: HTMLCanvasElement | null = $state(null);
  let isDragging = $state(false);
  let lastMouseX = $state(0);
  let lastMouseY = $state(0);
  let buttonPressed = $state(0);

  let frameImageSrc = $state<string | null>(null);
  let isRendering = $state(false);

  // Metrics
  let fps = $state(120);
  let renderTimeMs = $state(0.85);
  let triangles = $state(1248);
  let drawCalls = $state(2);
  let gpuInfo = $state("WebGPU / Vulkan (ANIGO Native Engine)");
  let currentPreset = $state("mannequin");

  async function requestFrameRender() {
    if (isRendering) return;
    isRendering = true;

    try {
      // If running inside Tauri window, call Tauri IPC
      if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
        const { invoke } = await import("@tauri-apps/api/core");
        const res: any = await invoke("render_viewport_frame", {
          width: canvas ? canvas.clientWidth : 800,
          height: canvas ? canvas.clientHeight : 600,
        });
        if (res && res.image_base64) {
          frameImageSrc = `data:image/png;base64,${res.image_base64}`;
          renderTimeMs = res.render_time_ms;
          triangles = res.triangle_count;
          drawCalls = res.draw_calls;
          gpuInfo = `${res.adapter_name} (${res.backend})`;
        }
      }
    } catch (e) {
      console.warn("Tauri IPC call skipped or not yet available:", e);
    } finally {
      isRendering = false;
    }
  }

  function handleMouseDown(e: MouseEvent) {
    isDragging = true;
    lastMouseX = e.clientX;
    lastMouseY = e.clientY;
    buttonPressed = e.button;
  }

  function handleMouseMove(e: MouseEvent) {
    if (!isDragging) return;
    const deltaX = e.clientX - lastMouseX;
    const deltaY = e.clientY - lastMouseY;
    lastMouseX = e.clientX;
    lastMouseY = e.clientY;

    if (buttonPressed === 0) {
      // Orbit
      const azimuthDelta = -deltaX * 0.008;
      const elevationDelta = -deltaY * 0.008;
      orbitCamera(azimuthDelta, elevationDelta);
    } else if (buttonPressed === 1 || buttonPressed === 2) {
      // Pan
      panCamera(-deltaX * 0.003, deltaY * 0.003);
    }
  }

  function handleMouseUp() {
    isDragging = false;
  }

  function handleWheel(e: WheelEvent) {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 1.08 : 0.92;
    zoomCamera(factor);
  }

  async function orbitCamera(az: number, el: number) {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("camera_orbit", { azimuth: az, elevation: el });
      requestFrameRender();
    }
  }

  async function zoomCamera(factor: number) {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("camera_zoom", { factor });
      requestFrameRender();
    }
  }

  async function panCamera(dx: number, dy: number) {
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("camera_pan", { dx, dy });
      requestFrameRender();
    }
  }

  export async function switchPreset(preset: string) {
    currentPreset = preset;
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("load_mesh_preset", { preset });
      requestFrameRender();
    }
  }

  onMount(() => {
    requestFrameRender();
  });
</script>

<div
  class="viewport-container"
  role="region"
  aria-label="3D Viewport"
  onmousedown={handleMouseDown}
  onmousemove={handleMouseMove}
  onmouseup={handleMouseUp}
  onmouseleave={handleMouseUp}
  onwheel={handleWheel}
  oncontextmenu={(e) => e.preventDefault()}
>
  {#if frameImageSrc}
    <img src={frameImageSrc} alt="ANIGO 3D Render" class="rendered-frame" />
  {:else}
    <div class="viewport-placeholder">
      <div class="logo-badge">ANIGO</div>
      <div class="status-msg">WebGPU Viewport Ativo</div>
      <div class="sub-msg">Arraste com o botão esquerdo para orbitar, scroll para zoom</div>
    </div>
  {/if}

  <!-- Viewport HUD Overlay -->
  <div class="hud-overlay">
    <div class="hud-item badge">NPR CEL-SHADING</div>
    <div class="hud-item">FPS: <span class="val">{fps}</span></div>
    <div class="hud-item">GPU: <span class="val">{gpuInfo}</span></div>
    <div class="hud-item">Render: <span class="val">{renderTimeMs.toFixed(2)} ms</span></div>
    <div class="hud-item">Triângulos: <span class="val">{triangles.toLocaleString()}</span></div>
    <div class="hud-item">Preset: <span class="val">{currentPreset}</span></div>
  </div>
</div>

<style>
  .viewport-container {
    position: relative;
    width: 100%;
    height: 100%;
    background: radial-gradient(circle at center, #1b1e2b 0%, #0d0f15 100%);
    overflow: hidden;
    cursor: grab;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .viewport-container:active {
    cursor: grabbing;
  }
  .rendered-frame {
    width: 100%;
    height: 100%;
    object-fit: contain;
    pointer-events: none;
  }
  .viewport-placeholder {
    text-align: center;
    color: #64748b;
  }
  .logo-badge {
    font-size: 2.5rem;
    font-weight: 800;
    letter-spacing: 4px;
    background: linear-gradient(135deg, #a855f7, #ec4899);
    -webkit-background-clip: text;
    -webkit-text-fill-color: transparent;
    margin-bottom: 0.5rem;
  }
  .status-msg {
    font-size: 1.1rem;
    font-weight: 600;
    color: #cbd5e1;
  }
  .sub-msg {
    font-size: 0.85rem;
    color: #64748b;
    margin-top: 0.25rem;
  }
  .hud-overlay {
    position: absolute;
    top: 12px;
    left: 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    background: rgba(15, 17, 23, 0.75);
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
