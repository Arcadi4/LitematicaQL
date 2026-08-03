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

import { describe, expect, it } from "vite-plus/test";
import {
  quickLookPostProcessingOptions,
  quickLookSafeMeshBuildingMode,
} from "./renderer-configuration";

describe("quickLookSafeMeshBuildingMode", () => {
  it("avoids the animation-frame-dependent batched pipeline", () => {
    expect(quickLookSafeMeshBuildingMode).toBe("incremental");
    expect(quickLookSafeMeshBuildingMode).not.toBe("batched");
  });
});

describe("quickLookPostProcessingOptions", () => {
  it("enables geometry-aware shading without changing texture gamma", () => {
    expect(quickLookPostProcessingOptions).toMatchObject({
      enabled: true,
      enableSSAO: true,
      enableSMAA: true,
      enableGamma: false,
      ssaoPresets: {
        isometric: {
          aoRadius: 0.3,
          distanceFalloff: 0.1,
          intensity: 0.8,
        },
      },
    });
  });
});
