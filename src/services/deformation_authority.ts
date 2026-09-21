/**
 * ANIGO — Deformation authority gate.
 *
 * ARQUITETURA_CANONICA_ANIGO §2.2: the canonical mesh is deformed **only** by
 * the Rust core. Since P0 §7.4 the TypeScript engine was removed from production
 * entirely — it lives in `tests/reference/morph_engine.ts` as the oracle the
 * contract tests compare against.
 *
 * This module keeps the single place that decides who may deform a session, so
 * that a future reference mode (dev preview) can never be reached implicitly:
 * `requireReferenceDeformation()` is the only door, and the contract tests
 * assert that no `src/**` module imports the reference engine at all.
 */

import {
  authorityFor,
  coverageIsComplete,
  type DeformationAuthority as WireAuthority,
  type DeformationCoverage,
} from "../contracts/core_snapshot.v1";

/** Authority resolved for the current session. */
export type RuntimeDeformationAuthority = "core" | "reference_ts" | "unavailable";

export interface AuthorityContext {
  /** A Rust core answered (Tauri session with the core commands registered). */
  coreAvailable: boolean;
  /** Coverage reported by the last core snapshot, when any. */
  coverage: DeformationCoverage | null;
  /** `true` for a production build (`import.meta.env.PROD`). */
  production: boolean;
  /** Explicit opt-in used by the desktop settings during the convergence sprint. */
  allowReferenceFallback?: boolean;
}

export interface AuthorityDecision {
  authority: RuntimeDeformationAuthority;
  /** Authority implied by the core model (independent of availability). */
  coreAuthority: WireAuthority | null;
  /** `true` when the session does not run on the canonical core model. */
  degraded: boolean;
  /** Human-readable reason, shown in the status bar and telemetry. */
  reason: string;
}

/**
 * The reference engine is allowed in development builds only. In a production
 * build the viewport either uses core-authored geometry or shows the
 * undeformed base mesh — it never silently becomes a second deformation engine.
 */
export function referenceFallbackAllowed(context: {
  production: boolean;
  allowReferenceFallback?: boolean;
}): boolean {
  if (context.allowReferenceFallback === true) return true;
  return !context.production;
}

/** Decides who deforms the canonical mesh for this session. */
export function decideAuthority(context: AuthorityContext): AuthorityDecision {
  const { coreAvailable, coverage } = context;
  const coreAuthority = coverage ? authorityFor(coverage) : null;
  const fallbackAllowed = referenceFallbackAllowed(context);

  if (!coreAvailable) {
    return fallbackAllowed
      ? {
          authority: "reference_ts",
          coreAuthority,
          degraded: true,
          reason:
            "Núcleo Rust indisponível (preview no navegador): deformação de referência em uso — não é autoridade de produção.",
        }
      : {
          authority: "unavailable",
          coreAuthority,
          degraded: true,
          reason:
            "Núcleo Rust indisponível e deformação de referência desativada em produção: exibindo a malha base canônica.",
        };
  }

  if (!coverage) {
    return {
      authority: fallbackAllowed ? "reference_ts" : "unavailable",
      coreAuthority,
      degraded: true,
      reason: "Aguardando o primeiro snapshot canônico do núcleo.",
    };
  }

  if (coverageIsComplete(coverage)) {
    return {
      authority: "core",
      coreAuthority: coreAuthority ?? "core",
      degraded: false,
      reason: `Núcleo Rust é a autoridade (${coverage.sliders_with_geometry}/${coverage.total_sliders} sliders, políticas de proporção v${coverage.proportion_policy_version} e somatotipo v${coverage.somatotype_policy_version}).`,
    };
  }

  return {
    authority: fallbackAllowed ? "reference_ts" : "unavailable",
    coreAuthority,
    degraded: true,
    reason: `Modelo canônico incompleto no núcleo (${coverage.sliders_with_geometry}/${coverage.total_sliders} sliders com geometria).`,
  };
}

/** Raised when a second deformation implementation is (mis)used in production. */
export class DeformationAuthorityError extends Error {
  readonly code = "DEFORMATION_AUTHORITY_VIOLATION";
  constructor(detail: string) {
    super(`[ANIGO deformation authority] ${detail}`);
    this.name = "DeformationAuthorityError";
  }
}

/**
 * Guards every entry point of the TypeScript reference deformation engine.
 * Throws when the reference engine is not the authorized authority, so a
 * production session can never silently deform twice.
 */
export function requireReferenceDeformation(authority: RuntimeDeformationAuthority, entryPoint: string): void {
  if (authority === "reference_ts") return;
  if (authority === "core") {
    throw new DeformationAuthorityError(
      `${entryPoint} must not run: the Rust core is the deformation authority for this session`
    );
  }
  throw new DeformationAuthorityError(
    `${entryPoint} must not run: no deformation authority is available (base geometry only)`
  );
}

/** Diagnostics shared with the UI + telemetry. */
export interface AuthorityDiagnostics {
  authority: RuntimeDeformationAuthority;
  degraded: boolean;
  reason: string;
  coreRevision: number | null;
  coverage: DeformationCoverage | null;
}

export function describeAuthority(
  decision: AuthorityDecision,
  coreRevision: number | null,
  coverage: DeformationCoverage | null
): AuthorityDiagnostics {
  return {
    authority: decision.authority,
    degraded: decision.degraded,
    reason: decision.reason,
    coreRevision,
    coverage,
  };
}
