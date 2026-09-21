/**
 * Node ESM resolution hook for ANIGO tests: resolves relative imports inside
 * src/ the way Vite/TypeScript do, so `node --test` can run the pure-TS unit
 * tests without the source files losing their extensionless imports.
 *
 * A specifier counts as "already resolved" only when it ends in a real script
 * extension. `path.extname` alone is not enough: `../contracts/core_snapshot.v1`
 * reports `.1`, which is a versioned module name, not an extension.
 */
import { existsSync, statSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

const SCRIPT_EXTENSIONS = new Set([".ts", ".tsx", ".mts", ".cts", ".js", ".mjs", ".cjs", ".json"]);

function isFile(p) {
  try {
    return statSync(p).isFile();
  } catch {
    return false;
  }
}

export async function resolve(specifier, context, nextResolve) {
  if (specifier.startsWith(".") && context.parentURL?.startsWith("file:")) {
    const parentDir = path.dirname(fileURLToPath(context.parentURL));
    const abs = path.resolve(parentDir, specifier);
    if (!SCRIPT_EXTENSIONS.has(path.extname(abs))) {
      for (const candidate of [`${abs}.ts`, `${abs}.tsx`, path.join(abs, "index.ts")]) {
        if (isFile(candidate)) {
          return { url: pathToFileURL(candidate).href, shortCircuit: true };
        }
      }
    }
  }
  return nextResolve(specifier, context);
}

/** Exported for the harness self-test. */
export { SCRIPT_EXTENSIONS, existsSync };
