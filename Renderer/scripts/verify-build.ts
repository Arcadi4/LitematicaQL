import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";

const rendererDirectory = join(import.meta.dirname, "..");
const outputDirectory = join(rendererDirectory, "..", "Resources", "Renderer");
const html = await readFile(join(outputDirectory, "index.html"), "utf8");

const externalAssetPatterns: RegExp[] = [
  /<script\b[^>]*\bsrc=/iu,
  /<link\b[^>]*\brel=["'](?:modulepreload|stylesheet)["']/iu,
];

if (externalAssetPatterns.some((pattern) => pattern.test(html))) {
  throw new Error("Renderer build still references external JavaScript or CSS assets.");
}

if (!html.includes("litematicaQL") || !html.includes("meshReady")) {
  throw new Error("Renderer build is missing the native message bridge or mesh handoff.");
}

if (html.includes("data:application/wasm;base64,") || html.includes("nucleation")) {
  throw new Error("Renderer build still bundles the WebAssembly mesher.");
}

const requiredLicenses = ["three-LICENSE"];
for (const license of requiredLicenses) {
  const contents = await readFile(join(rendererDirectory, "third-party", license), "utf8");
  if (contents.trim().length === 0) {
    throw new Error(`Third-party license is empty: ${license}`);
  }
}

const resourcePack = await stat(join(outputDirectory, "pack.zip"));
if (resourcePack.size === 0) {
  throw new Error("Bundled resource pack is empty.");
}

console.log("Renderer build is self-contained and displays the native bridge's mesh.");
