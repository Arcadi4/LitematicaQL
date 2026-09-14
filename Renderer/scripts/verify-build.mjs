// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import { readFile, stat } from "node:fs/promises";
import { join } from "node:path";

const rendererDirectory = join(import.meta.dirname, "..");
const outputDirectory = join(rendererDirectory, "..", "Resources", "Renderer");
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

// Quick Look cannot fetch sibling files over `file://`, so Nucleation's
// WebAssembly binary has to travel inside the document.
if (!html.includes("data:application/wasm;base64,")) {
  throw new Error("Renderer build does not inline the Nucleation WebAssembly binary.");
}

const requiredLicenses = ["fflate-LICENSE", "nucleation-LICENSE", "three-LICENSE"];
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

console.log("Renderer build is self-contained and uses the native resource-pack bridge.");
