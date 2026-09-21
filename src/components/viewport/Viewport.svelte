<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { devicePixelRatioSafe } from "../../services/settings_persist";
  import { WebGpuViewportRenderer, type ViewportMetrics, type MeshPreset } from "./webgpu_renderer";
  import type { CoreSnapshotDelivery } from "../../services/core_bridge";
  import {
    RendererDiagnostics,
    type DiagnosticCode,
    type DiagnosticsSummary,
    type RenderDiagnostic,
  } from "../../services/render_diagnostics";
  import type { AnatomicalSegment } from "./tactile";

  let {
    onResize = undefined,
    onMetrics = undefined,
    onProjectionChange = undefined,
    onRenderGraphChange = undefined,
    onTactileDrag = undefined,
    onTactileDragStart = undefined,
    onTactileDragEnd = undefined,
    onModelLoadError = undefined,
    tactileEnabled = false,
  }: {
    onResize?: (w: number, h: number) => void;
    onMetrics?: (m: ViewportMetrics) => void;
    /** Issue #13: notifica a troca de projeção (perspectiva/ortográfica). */
    onProjectionChange?: (mode: "perspective" | "orthographic") => void;
    /** Issue #14: notifica a mudança do render graph (depth pre-pass). */
    onRenderGraphChange?: (state: { depthPrepass: boolean }) => void;
    onTactileDrag?: (
      primarySlider: string,
      primaryDelta: number,
      secondarySlider?: string,
      secondaryDelta?: number
    ) => void;
    onTactileDragStart?: (segment: AnatomicalSegment) => void;
    onTactileDragEnd?: () => void;
    onModelLoadError?: (message: string) => void;
    /** P1-02: diagnóstico estruturado do renderer (status bar/telemetria). */
    onDiagnostic?: (diagnostic: RenderDiagnostic) => void;
    /** P0-09: tactile manipulation is gated by the parent (personagem + body/face only). */
    tactileEnabled?: boolean;
    /**
     * P0 §7.5 — busca o snapshot canônico no núcleo. Quem fala com o núcleo é o
     * shell (`App.svelte`); o viewport só consome a geometria autorizada.
     */
    coreSnapshotProvider?: (force?: boolean) => Promise<CoreSnapshotDelivery | null>;
    /** Whether the renderer should still load the raw GLB (degraded, no core). */
    allowDegradedBaseMesh?: boolean;
  } = $props();

  let canvas: HTMLCanvasElement | null = $state(null);
  let containerEl: HTMLElement | null = $state(null);
  let renderer: WebGpuViewportRenderer | null = null;
  // Issue #11: notificação informativa de recuperação do device (rodapé).
  let deviceRecoveryNotice: string | null = $state(null);
  let deviceRecoveryTimer: ReturnType<typeof setTimeout> | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let coreSnapshotInFlight: Promise<unknown> | null = null;
  let coreAuthority: string = "unavailable";

  // Pointer Interaction State (DCC standard: LMB/Alt+LMB Orbit, MMB/Shift+LMB Pan, Wheel/Alt+RMB Zoom)
  let isDragging = $state(false);
  let lastMouseX = $state(0);
  let lastMouseY = $state(0);
  let buttonPressed = $state(0);
  let activeTactileSegment: AnatomicalSegment | null = $state(null);

  function handlePointerDown(e: PointerEvent) {
    try {
      containerEl?.setPointerCapture(e.pointerId);
    } catch (_) {}
    isDragging = true;
    lastMouseX = e.clientX;
    lastMouseY = e.clientY;
    buttonPressed = e.button;

    // Prevent middle-click autoscroll and context actions
    if (e.button === 1 || e.button === 2) {
      e.preventDefault();
      activeTactileSegment = null;
    } else if (e.button === 0 && !e.altKey && !e.ctrlKey && !e.shiftKey && renderer && canvas) {
      // P0-09: tactile raycast ONLY when explicitly enabled by the parent
      // (personagem workspace + body/face tool). Otherwise plain orbit.
      if (tactileEnabled) {
        const rect = canvas.getBoundingClientRect();
        const screenX = e.clientX - rect.left;
        const screenY = e.clientY - rect.top;
        const hit = renderer.raycastTactile(screenX, screenY);
        activeTactileSegment = hit ? hit.segment : null;
        if (activeTactileSegment) onTactileDragStart?.(activeTactileSegment);
      } else {
        activeTactileSegment = null;
      }
    } else {
      activeTactileSegment = null;
    }
  }

  function handlePointerMove(e: PointerEvent) {
    if (!isDragging || !renderer) return;
    const deltaX = e.clientX - lastMouseX;
    const deltaY = e.clientY - lastMouseY;
    lastMouseX = e.clientX;
    lastMouseY = e.clientY;

    if (activeTactileSegment) {
      const proj = renderer.projectTactileDelta(activeTactileSegment, deltaX, deltaY);
      if (onTactileDrag) {
        onTactileDrag(proj.primarySlider, proj.primaryDelta, proj.secondarySlider, proj.secondaryDelta);
      }
      return;
    }

    const isZoom =
      (buttonPressed === 2 && e.altKey) ||
      (buttonPressed === 0 && e.ctrlKey);

    const isPan =
      buttonPressed === 1 ||
      (buttonPressed === 0 && e.shiftKey) ||
      (buttonPressed === 2 && e.shiftKey) ||
      (buttonPressed === 2 && !e.altKey && !e.ctrlKey);

    if (isZoom) {
      // Progressive distance zoom
      const zoomFactor = Math.exp(deltaY * 0.005);
      renderer.zoom(zoomFactor);
      import("@tauri-apps/api/core").then(({ invoke }) => invoke("camera_zoom", { factor: zoomFactor }).catch(()=>{}));
    } else if (isPan) {
      // 1:1 calibrated screen-space pan
      renderer.pan(deltaX, deltaY);
      import("@tauri-apps/api/core").then(({ invoke }) => invoke("camera_pan", { dx: deltaX, dy: deltaY }).catch(()=>{}));
    } else if (buttonPressed === 0) {
      // Orbit (LMB or Alt+LMB)
      const azimuthDelta = -deltaX * 0.008;
      const elevationDelta = -deltaY * 0.008;
      renderer.orbit(azimuthDelta, elevationDelta);
      import("@tauri-apps/api/core").then(({ invoke }) => invoke("camera_orbit", { azimuth: azimuthDelta, elevation: elevationDelta }).catch(()=>{}));
    }
  }

  function handlePointerUp(e: PointerEvent) {
    if (isDragging) {
      try {
        containerEl?.releasePointerCapture(e.pointerId);
      } catch (_) {}
      isDragging = false;
      if (activeTactileSegment) onTactileDragEnd?.();
      activeTactileSegment = null;
    }
  }

  function handleWheel(e: WheelEvent) {
    e.preventDefault();
    if (!renderer) return;
    const targetEl = canvas || containerEl;
    let ndcX = 0;
    let ndcY = 0;
    if (targetEl) {
      const rect = targetEl.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) {
        ndcX = ((e.clientX - rect.left) / rect.width) * 2 - 1;
        ndcY = 1 - ((e.clientY - rect.top) / rect.height) * 2;
      }
    }
    const factor = e.deltaY > 0 ? 1.08 : 0.92;
    renderer.zoom(factor, ndcX, ndcY);
  }

  function handleKeyDown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
      return;
    }

    if (e.key === "f" || e.key === "F" || e.key === "Home") {
      e.preventDefault();
      recenterCamera();
    }
    // Issue #13: alternância perspectiva ⇄ ortográfica por atalho (mesma ação
    // do botão em LightingControls). O enquadramento é preservado pelo renderer.
    if (e.key === "o" || e.key === "O") {
      e.preventDefault();
      toggleProjectionMode();
    }
  }

  // Exported API for parent components & automated bridge
  export async function loadCanonicalModel(gender: "male" | "female") {
    if (!renderer) return;
    try {
      return await renderer.loadCanonicalModel(gender);
    } catch (e) {
      // P0-10: load failures propagate to the UI (never silent cube).
      const msg = e instanceof Error ? e.message : String(e);
      onModelLoadError?.(msg);
      throw e;
    }
  }

  /**
   * P0 §8: geometria canônica do núcleo que está na GPU + revisão estática.
   * O App usa isso para o teste de paridade da exportação (nunca para deformar).
   */
  export function getCoreGeometry(): {
    geometry: ReturnType<WebGpuViewportRenderer["getCoreGeometry"]>;
    staticRevision: number;
  } {
    return {
      geometry: renderer?.getCoreGeometry?.() ?? null,
      staticRevision: renderer?.getCoreStaticRevision?.() ?? 0,
    };
  }

  export function getMorphCoverage(): { implemented: number; total: number; explicit: number } | null {
    try {
      return (renderer as any)?.getMorphCoverage?.() ?? null;
    } catch {
      return null;
    }
  }

  export function switchPreset(preset: MeshPreset, headScale?: number, headRatio?: number) {
    if (renderer) renderer.switchPreset(preset, headScale, headRatio);
  }

  export function setHeadProportions(headScale: number, headRatio: number) {
    if (renderer) renderer.setHeadProportions(headScale, headRatio);
  }

  export function setProportions(params: {
    headScale?: number;
    headRatio?: number;
    shoulderWidth?: number;
    legLength?: number;
    armLength?: number;
    neckLength?: number;
    torsoLength?: number;
    heightOverall?: number;
  }) {
    if (renderer) renderer.setProportions(params);
  }

  export function setSomatotype(endo: number, meso: number, ecto: number) {
    if (renderer) renderer.setSomatotype(endo, meso, ecto);
  }

  export function setGenderDimorphism(gender: number) {
    if (renderer) renderer.setGenderDimorphism(gender);
  }

  export function setMorphSlider(name: string, weight: number) {
    if (renderer) renderer.setMorphSlider(name, weight);
  }

  export function orbit(azimuth: number, elevation: number) {
    if (renderer) renderer.orbit(azimuth, elevation);
    // P0-05: sync backend camera (was fixed)
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      import("@tauri-apps/api/core").then(({ invoke }) => invoke("camera_orbit", { azimuth, elevation }).catch(()=>{}));
    }
  }

  export function zoom(factor: number, mouseNdcX: number = 0, mouseNdcY: number = 0) {
    if (renderer) renderer.zoom(factor, mouseNdcX, mouseNdcY);
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      import("@tauri-apps/api/core").then(({ invoke }) => invoke("camera_zoom", { factor }).catch(()=>{}));
    }
  }

  export function pan(dx: number, dy: number) {
    if (renderer) renderer.pan(dx, dy);
    if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
      import("@tauri-apps/api/core").then(({ invoke }) => invoke("camera_pan", { dx, dy }).catch(()=>{}));
    }
  }

  export function setLight(
    dir: [number, number, number],
    intensity: number,
    shadowColor?: [number, number, number],
    lightColor?: [number, number, number],
    ambientIntensity?: number,
    shadowSaturation?: number
  ) {
    if (renderer) {
      renderer.setLight(dir, intensity, shadowColor, lightColor, ambientIntensity, shadowSaturation);
    }
  }

  export function setMaterialParams(params: {
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
  }) {
    if (renderer) renderer.setMaterialParams(params);
  }

  export function setOutlineWidth(width: number) {
    if (renderer) renderer.setOutlineWidth(width);
  }

  export function setOutlineColor(color: [number, number, number, number]) {
    if (renderer) renderer.setOutlineColor(color);
  }

  export function setShadowThreshold(threshold: number) {
    if (renderer) renderer.setShadowThreshold(threshold);
  }

  export function setToonSmoothness(smoothness: number) {
    if (renderer) renderer.setToonSmoothness(smoothness);
  }

  export function setSpecular(intensity: number, exponent: number) {
    if (renderer) renderer.setSpecular(intensity, exponent);
  }

  export function setRimLight(intensity: number, spread: number, color?: [number, number, number]) {
    if (renderer) renderer.setRimLight(intensity, spread, color as any);
    if (color && renderer) (renderer as any).setRimColor?.(color);
  }

  export function setHueShift(degrees: number) {
    if (renderer) renderer.setHueShift(degrees);
  }

  export function setToonSteps(steps: number) {
    if (renderer) renderer.setToonSteps(steps);
  }

  export function recenterCamera(duration: number = 300) {
    if (renderer) renderer.recenterCamera(duration);
  }

  /**
   * Issue #13: alterna perspectiva ⇄ ortográfica sem salto visual (o renderer
   * casa o enquadramento na distância do alvo).
   */
  export function toggleProjectionMode(): "perspective" | "orthographic" {
    const resolved = (renderer?.toggleProjectionMode()?.mode ??
      "perspective") as "perspective" | "orthographic";
    syncProjectionToCore(resolved);
    onProjectionChange?.(resolved);
    return resolved;
  }

  /** Define o modo de projeção explicitamente (botão da UI). */
  export function setProjectionMode(mode: "perspective" | "orthographic"): "perspective" | "orthographic" {
    const applied = renderer?.setProjectionMode(mode);
    const resolved = (applied?.mode ?? mode) as "perspective" | "orthographic";
    syncProjectionToCore(resolved);
    onProjectionChange?.(resolved);
    return resolved;
  }

  /**
   * Issue #13: o núcleo é a autoridade dos dados (undo/redo, persistência e o
   * render headless/exportação), então a troca de projeção vai para ele como
   * comando canônico — o mesmo caminho de `camera_orbit`/`camera_zoom`.
   */
  function syncProjectionToCore(mode: "perspective" | "orthographic") {
    if (typeof window === "undefined" || !(window as any).__TAURI_INTERNALS__) return;
    import("@tauri-apps/api/core")
      .then(({ invoke }) => invoke("camera_projection", { orthographic: mode === "orthographic", height: null }))
      .catch(() => {
        // Fora do host Tauri (preview no navegador) o núcleo não está presente.
      });
  }

  /** `true` quando a câmera está em projeção ortográfica. */
  export function isProjectionOrthographic(): boolean {
    return renderer?.isOrthographic() ?? false;
  }

  /**
   * Issue #14: o depth pre-pass é decisão do documento, não um botão local — a
   * UI aplica no renderer e manda o comando canônico para o núcleo, que devolve
   * o mesmo estado no próximo snapshot (viewport e headless concordam).
   */
  export function setDepthPrepass(enabled: boolean): boolean {
    const applied = renderer?.setDepthPrepass(enabled) ?? false;
    syncRenderGraphToCore({ depth_prepass: enabled });
    onRenderGraphChange?.({ depthPrepass: applied });
    return applied;
  }

  /** `true` quando o plano do frame inclui o depth pre-pass. */
  export function isDepthPrepassEnabled(): boolean {
    return renderer?.depthPrepassEnabled() ?? false;
  }

  /** Passes executados neste frame, na ordem do render graph. */
  export function passesExecuted(): string[] {
    return renderer?.passesExecuted() ?? [];
  }

  /**
   * Issue #14: caminho canônico (`set_render_settings`) para o núcleo. O mesmo
   * comando aparece no log/undo, então a mudança é revertível como qualquer
   * outra.
   */
  function syncRenderGraphToCore(patch: { depth_prepass?: boolean; disabled_passes?: string[]; pass_order?: string[] }) {
    if (typeof window === "undefined" || !(window as any).__TAURI_INTERNALS__) return;
    import("@tauri-apps/api/core")
      .then(({ invoke }) => invoke("core_apply_command", { command: { kind: "set_render_settings", ...patch } }))
      .catch(() => {
        // Fora do host Tauri (preview no navegador) o núcleo não está presente.
      });
  }

  export function setFpsCap(fps: number) {
    if (renderer) renderer.setFpsCap(fps);
  }

  export function setDpiScale(multiplier: number) {
    if (renderer) renderer.setDpiScale(multiplier);
  }

  export function setVsync(enabled: boolean) {
    if (renderer) renderer.setVsync(enabled);
  }

  /**
   * Aplica um snapshot canônico já buscado pelo shell (parte estática e/ou
   * dinâmica). Devolve o relatório de autoridade para telemetria/status bar.
   */
  export function applyCoreDelivery(delivery: CoreSnapshotDelivery) {
    if (!renderer) return null;
    const report = renderer.applyCoreSnapshot(delivery);
    coreAuthority = report.authority;
    return report;
  }

  /** Pede (uma vez) um snapshot novo ao núcleo e o aplica. */
  export async function requestCoreSnapshot(force: boolean = false) {
    if (!coreSnapshotProvider) return null;
    if (coreSnapshotInFlight) return coreSnapshotInFlight;
    coreSnapshotInFlight = (async () => {
      try {
        const delivery = await coreSnapshotProvider(force);
        if (delivery) return applyCoreDelivery(delivery);
        return null;
      } catch (error) {
        console.warn("[ANIGO 3D] core snapshot request failed:", error);
        return null;
      } finally {
        coreSnapshotInFlight = null;
      }
    })();
    return coreSnapshotInFlight;
  }

  /** Resumo dos diagnósticos do renderer (item 2 do P1). */
  export function getDiagnostics(): { summary: DiagnosticsSummary; entries: readonly RenderDiagnostic[] } {
    return (
      renderer?.getDiagnostics?.() ?? {
        summary: { total: 0, errors: 0, warnings: 0, codes: [], dropped: 0, degraded: false },
        entries: [],
      }
    );
  }

  /**
   * Reporta um diagnóstico originado fora do renderer (ex.: snapshot recusado
   * pelo contrato). Passa pelo mesmo canal — um só lugar para a UI observar.
   */
  export function reportDiagnostic(
    code: DiagnosticCode,
    message: string,
    options?: { detail?: string; context?: Record<string, string | number | boolean> }
  ): void {
    if (renderer?.reportDiagnostic) {
      renderer.reportDiagnostic(code, message, options);
      return;
    }
    console.warn(RendererDiagnostics.format({
      code,
      severity: "error",
      message,
      detail: options?.detail ?? null,
      context: options?.context ?? null,
      count: 1,
      firstAt: 0,
      lastAt: 0,
    }));
  }

  /** Autoridade de deformação em vigor no viewport (degradação explícita). */
  export function getDeformationAuthority() {
    return renderer?.getDeformationAuthority?.() ?? {
      authority: coreAuthority,
      degraded: true,
      coreStaticRevision: 0,
      coreDynamicRevision: 0,
      channels: 0,
      vertexCount: 0,
    };
  }

  export function resize(w?: number, h?: number) {
    if (containerEl && renderer) {
      const targetW = w ?? containerEl.clientWidth;
      const targetH = h ?? containerEl.clientHeight;
      if (targetW > 0 && targetH > 0) {
        renderer.resize(targetW, targetH);
        onResize?.(targetW, targetH);
      }
    }
  }

  onMount(async () => {
    if (canvas && containerEl) {
      renderer = new WebGpuViewportRenderer(canvas);
      // P0-10: surface model/GLB failures to the parent UI.
      renderer.onModelLoadError = (msg: string) => onModelLoadError?.(msg);
      // P1-02: todo diagnóstico do renderer sobe para o shell.
      renderer.onDiagnostic = (diagnostic: RenderDiagnostic) => onDiagnostic?.(diagnostic);
      // Issue #11: recuperação de device → notificação no rodapé (evento + tempo).
      renderer.onDeviceRecovery = (info) => {
        deviceRecoveryNotice =
          `GPU recuperada: ${info.attempts} tentativa(s) em ${Math.round(info.elapsedMs)} ms`;
        if (deviceRecoveryTimer) clearTimeout(deviceRecoveryTimer);
        deviceRecoveryTimer = setTimeout(() => {
          deviceRecoveryNotice = null;
        }, 6000);
      };
      // P0 §7.5: quando o renderer precisa de geometria canônica, o shell busca
      // o snapshot no núcleo (nunca há deformação local como plano B).
      renderer.onCoreGeometryRequired = () => {
        void requestCoreSnapshot();
      };

      renderer.onMetricsUpdate = (m: ViewportMetrics) => {
        onMetrics?.(m);
        // Sync live telemetry back to Tauri's LiveWindowState in the background (no HUD overlay on canvas)
        if (typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__) {
          const diagnostics = getDiagnostics().summary;
          import("@tauri-apps/api/core").then(({ invoke }) => {
            invoke("report_live_telemetry", {
              telemetry: {
                fps: m.fps,
                frame_time_ms: m.frameTimeMs,
                draw_calls: m.drawCalls,
                triangle_count: m.triangles,
                adapter_name: m.adapterName,
                camera_eye: renderer?.eye || [0, 1.5, 3.5],
                camera_target: renderer?.target || [0, 1, 0],
                light_direction: renderer?.lightDir || [0.577, 0.577, 0.577],
                light_intensity: renderer?.lightIntensity ?? 1.0,
                shadow_color: renderer?.shadowColor || [1.0, 1.0, 1.0], // P1-08 neutral
                active_preset: renderer?.currentPreset || "mannequin",
                outline_width: (renderer?.outlineWidth || 0.0035) * 1000,
                shadow_threshold: renderer?.shadowThreshold || 0.5,
                head_scale: renderer?.headScale || 1.0,
                head_ratio: renderer?.headRatio || 6.5,
                webgpu_active: m.backend === "WebGPU",
                // P1-06: telemetria com dados reais do renderer/núcleo, não estimativas.
                diagnostics_errors: diagnostics.errors,
                diagnostics_warnings: diagnostics.warnings,
                diagnostic_codes: diagnostics.codes,
                deformation_authority: getDeformationAuthority().authority,
                core_static_revision: getDeformationAuthority().coreStaticRevision,
                core_dynamic_revision: getDeformationAuthority().coreDynamicRevision,
                morph_channels: getDeformationAuthority().channels,
                vertex_count: getDeformationAuthority().vertexCount,
                spec_intensity: renderer?.specIntensity ?? 0.4,
                spec_power: renderer?.specExponent ?? 32.0,
                rim_intensity: renderer?.rimIntensity ?? 0.8,
                hue_shift: renderer?.hueShift ?? -15.0,
                toon_steps: renderer?.toonSteps ?? 1.0,
                light_color: renderer?.lightColor || [1.0, 0.98, 0.95],
              },
            }).catch(() => {});
          });
        }
      };

      await renderer.initialize();
      if (typeof window !== "undefined") {
        (window as any).__ANIGO_VIEWPORT_RENDERER__ = renderer;
      }

      // Observe container resize
      if (containerEl) {
        resizeObserver = new ResizeObserver((entries) => {
          for (const entry of entries) {
            const { width, height } = entry.contentRect;
            if (width > 0 && height > 0) {
              const w = Math.floor(width);
              const h = Math.floor(height);
              if (renderer) {
                renderer.resize(w, h); // P1-11 DPR clamped inside renderer via devicePixelRatioSafe (max 2x)
              }
              onResize?.(w, h);
            }
          }
        });
        resizeObserver.observe(containerEl);

        // P1-10: pause when viewport hidden (IntersectionObserver + visibilitychange) — was rendering 120fps hidden
        const io = new IntersectionObserver((entries) => {
          for (const e of entries) {
            const hidden = !e.isIntersecting || (e.intersectionRatio as number) <= 0;
            (renderer as any)?.setPausedByVisibility?.(hidden);
          }
        }, { threshold: 0 });
        io.observe(containerEl);
        // store for cleanup (reuse resizeObserver variable for simplicity, add separate)
        (containerEl as any).__anigoIO = io;
        const onVis = () => {
          const hidden = document.hidden || document.visibilityState === 'hidden';
          // only pause if our container is also not intersecting? For now document hidden → pause
          if (hidden) (renderer as any)?.pause?.();
          else (renderer as any)?.resume?.();
        };
        document.addEventListener('visibilitychange', onVis);
        (containerEl as any).__anigoVisHandler = onVis;

        // Initial dimension report
        if (containerEl.clientWidth > 0 && containerEl.clientHeight > 0) {
          onResize?.(containerEl.clientWidth, containerEl.clientHeight);
        }
      }

      // Listen for Tauri live socket bridge events (MCP remote automation)
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
              renderer.zoom(
                event.payload.factor || 1.0,
                event.payload.ndcX ?? 0,
                event.payload.ndcY ?? 0
              );
            }
          });

          await listen("anigo://camera_pan", (event: any) => {
            if (renderer && event.payload) {
              renderer.pan(event.payload.dx || 0, event.payload.dy || 0);
            }
          });

          await listen("anigo://set_camera", (event: any) => {
            if (renderer && event.payload) {
              if (event.payload.eye) renderer.eye = event.payload.eye;
              if (event.payload.target) renderer.target = event.payload.target;
            }
          });

          // P1-07: single consumer is App/store — Viewport never handles shading/material directly (was duplicate snake→camel no-op)
          // set_light / load_preset / set_outline / set_material_toon / set_proportions now handled only by App.svelte via normalizeBridgePayload
          await listen("anigo://camera_recenter", () => {
            recenterCamera();
          });

          await listen("anigo://recenter_camera", () => {
            recenterCamera();
          });
        } catch (e) {
          console.warn("[Tauri Live Bridge] Event listeners init error:", e);
        }
      }

      window.addEventListener("keydown", handleKeyDown);
    }
  });

  onDestroy(() => {
    if (typeof window !== "undefined") {
      window.removeEventListener("keydown", handleKeyDown);
    }
    if (resizeObserver) {
      resizeObserver.disconnect();
      resizeObserver = null;
    }
    // P1-10 cleanup IntersectionObserver/visibility
    try {
      const io = (containerEl as any)?.__anigoIO as IntersectionObserver | undefined;
      io?.disconnect();
      const h = (containerEl as any)?.__anigoVisHandler as any;
      if (h) document.removeEventListener("visibilitychange", h);
    } catch (_) {}
    if (renderer) renderer.destroy();
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<div
  bind:this={containerEl}
  class="viewport-container"
  role="application"
  aria-label="ANIGO 3D Native Viewport"
  tabindex="0"
  onpointerdown={handlePointerDown}
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
  onpointercancel={handlePointerUp}
  onwheel={handleWheel}
  onkeydown={handleKeyDown}
  oncontextmenu={(e) => e.preventDefault()}
>
  <!-- Hardware 3D Canvas -->
  <!-- P2-16 loading/erro/vazio overlay (skeleton/spinner + diagnostics) -->
  {#if !renderer}<div class="viewport-loading">Carregando viewport…</div>{/if}
  <canvas
    bind:this={canvas}
    class="viewport-canvas"
    oncontextmenu={(e) => e.preventDefault()}
  ></canvas>
  <!-- Issue #11: notificação de recuperação de device (device lost → recreated) -->
  {#if deviceRecoveryNotice}
    <div class="device-recovery-toast" role="status">{deviceRecoveryNotice}</div>
  {/if}
</div>

<style>
  .viewport-container {
    position: relative;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    background: radial-gradient(circle at center, #1b202e 0%, #0a0d14 100%);
    overflow: hidden;
    cursor: grab;
    display: flex;
    align-items: center;
    justify-content: center;
    outline: none;
  }
  .viewport-container:active {
    cursor: grabbing;
  }
  .viewport-canvas {
    position: absolute;
    top: 0;
    left: 0;
    width: 100%;
    height: 100%;
    display: block;
    object-fit: contain;
  }
  .device-recovery-toast {
    position: absolute;
    bottom: 12px;
    left: 50%;
    transform: translateX(-50%);
    background: rgba(20, 26, 43, 0.92);
    color: #9ecbff;
    border: 1px solid rgba(158, 203, 255, 0.35);
    border-radius: 8px;
    padding: 8px 14px;
    font-size: 12px;
    font-family: sans-serif;
    pointer-events: none;
    z-index: 10000;
  }
</style>
