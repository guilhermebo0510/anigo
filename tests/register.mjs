/** Registered via `node --import ./tests/register.mjs` before the test runner. */
import { register } from "node:module";
import { pathToFileURL } from "node:url";

register("./tests/resolve-ts.mjs", pathToFileURL("./"));
