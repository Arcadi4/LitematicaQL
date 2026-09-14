// LitematicaQL: macOS Quick Look plugin for Litematica schematics.
// Copyright (C) 2026 4rcadia
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
//
// See the LICENSE file for the full license text.

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

const requiredLicenses = ["nucleation-LICENSE", "schematic-renderer-LICENSE", "three-LICENSE"];
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
