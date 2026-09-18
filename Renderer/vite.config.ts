import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { defineConfig } from "vite-plus";
import { viteSingleFile } from "vite-plugin-singlefile";

const require = createRequire(import.meta.url);
const nucleationDirectory = dirname(require.resolve("nucleation"));
const nucleationConfigPath = resolve(nucleationDirectory, "diplomat.config.mjs");
const nucleationConfigModuleId = "\0litematicaql-nucleation-config";

/**
 * Quick Look loads the renderer over `file://`, where `fetch` of a sibling
 * WebAssembly file fails and the offline content rules block every network
 * scheme. Nucleation's generated loader instantiates whatever module the
 * `diplomat.config.mjs` module exports as `wasm_path`, so replace that module
 * with a data URL holding the whole binary. This keeps `index.html`
 * self-contained, which `verify-build.mjs` enforces.
 */
function inlineNucleationWasm() {
  const encodedWasm = readFileSync(resolve(nucleationDirectory, "nucleation.wasm")).toString(
    "base64",
  );
  const wasmDataUrl = `data:application/wasm;base64,${encodedWasm}`;

  return {
    name: "litematicaql:inline-nucleation-wasm",
    enforce: "pre" as const,
    resolveId(source: string, importer: string | undefined) {
      if (!importer) {
        return null;
      }

      const candidate = resolve(dirname(importer.split("?", 1)[0]!), source);
      return candidate === nucleationConfigPath ? nucleationConfigModuleId : null;
    },
    load(id: string) {
      return id === nucleationConfigModuleId
        ? `export default { wasm_path: ${JSON.stringify(wasmDataUrl)} };`
        : null;
    },
  };
}

export default defineConfig({
  plugins: [inlineNucleationWasm(), viteSingleFile()],
  // `vendor/pack.zip` is the tracked Minecraft resource pack. Serving it as the
  // public directory copies it beside the bundle without a staging step.
  publicDir: "vendor",
  build: {
    emptyOutDir: true,
    modulePreload: false,
    outDir: "../Resources/Renderer",
    sourcemap: false,
    target: "safari16",
  },
  fmt: {},
  lint: {
    jsPlugins: [
      {
        name: "vite-plus",
        specifier: "vite-plus/oxlint-plugin",
      },
    ],
    options: {
      typeAware: true,
      typeCheck: true,
    },
    rules: {
      "vite-plus/prefer-vite-plus-imports": "error",
    },
  },
});
