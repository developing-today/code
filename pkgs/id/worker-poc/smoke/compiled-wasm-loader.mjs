// Let Node import the worker's wrangler CompiledWasm binding in the focused
// WorldObject smoke. The worker still receives a WebAssembly.Module, just as
// it does under workerd.
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

export async function load(url, context, nextLoad) {
  if (!url.endsWith(".wasm")) return nextLoad(url, context);

  const bytes = await readFile(fileURLToPath(url));
  const base64 = bytes.toString("base64");
  return {
    format: "module",
    shortCircuit: true,
    source: `const bytes = Uint8Array.from(Buffer.from(${JSON.stringify(base64)}, "base64"));\nexport default new WebAssembly.Module(bytes);\n`,
  };
}
