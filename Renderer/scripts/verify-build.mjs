import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";

const outputDirectory = join(import.meta.dirname, "..", "..", "Resources", "Renderer");
const html = await readFile(join(outputDirectory, "index.html"), "utf8");

const externalAssetPatterns = [
  /<script\b[^>]*\bsrc=/iu,
  /<link\b[^>]*\brel=["'](?:modulepreload|stylesheet)["']/iu,
];

if (externalAssetPatterns.some((pattern) => pattern.test(html))) {
  throw new Error("Renderer build still references external JavaScript or CSS assets.");
}

if (!html.includes("window.webkit") || !html.includes("litematicaQLResourcePack")) {
  throw new Error("Renderer build is missing the native resource-pack bridge.");
}

const resourcePack = await stat(join(outputDirectory, "pack.zip"));
if (resourcePack.size === 0) {
  throw new Error("Bundled resource pack is empty.");
}

console.log("Renderer build is self-contained and uses the native resource-pack bridge.");
