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

import { copyFile, mkdir } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const projectRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const rendererEntry = require.resolve("schematic-renderer");
const rendererDist = dirname(rendererEntry);
const rendererRoot = join(rendererDist, "..");
const rendererRequire = createRequire(join(rendererRoot, "package.json"));

await mkdir(join(projectRoot, "public"), { recursive: true });
await mkdir(join(projectRoot, "third-party"), { recursive: true });

await copyFile(join(rendererDist, "pack.zip"), join(projectRoot, "public", "pack.zip"));
await copyFile(
  join(rendererRoot, "LICENSE"),
  join(projectRoot, "third-party", "schematic-renderer-LICENSE"),
);

const threeEntry = rendererRequire.resolve("three");
const threeRoot = join(dirname(threeEntry), "..");
await copyFile(join(threeRoot, "LICENSE"), join(projectRoot, "third-party", "three-LICENSE"));

const nucleationPackage = rendererRequire.resolve("nucleation/package.json");
const nucleationRoot = dirname(nucleationPackage);
await copyFile(
  join(nucleationRoot, "LICENSE"),
  join(projectRoot, "third-party", "nucleation-LICENSE"),
);
