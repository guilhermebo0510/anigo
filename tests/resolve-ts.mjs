/**
 * Node ESM resolution hook for ANIGO tests: resolves extensionless relative
 * imports inside src/ (*.ts) the way Vite/TS do, so `node --test` can run
 * pure-TS service unit tests without touching source imports.
 */
import { existsSync, statSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

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
    if (!path.extname(abs)) {
      for (const candidate of [`${abs}.ts`, `${abs}.tsx`, path.join(abs, "index.ts")]) {
        if (isFile(candidate)) {
          return { url: pathToFileURL(candidate).href, shortCircuit: true };
        }
      }
    }
  }
  return nextResolve(specifier, context);
}
