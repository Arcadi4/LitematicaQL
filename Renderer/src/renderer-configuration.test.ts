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
  configureQuickLookLighting,
  quickLookPostProcessingOptions,
  quickLookSafeMeshBuildingMode,
} from "./renderer-configuration";

describe("quickLookSafeMeshBuildingMode", () => {
  it("uses animation-frame-independent instanced mesh building", () => {
    expect(quickLookSafeMeshBuildingMode).toBe("instanced");
  });
});

describe("quickLookPostProcessingOptions", () => {
  it("keeps WKWebView on the direct WebGL presentation path", () => {
    expect(quickLookPostProcessingOptions).toEqual({
      enabled: false,
      enableSSAO: false,
      enableSMAA: false,
      enableGamma: false,
    });
  });
});

describe("configureQuickLookLighting", () => {
  it("adds face contrast without removing the ambient readability floor", () => {
    const intensities = new Map([
      ["ambientLight", 2.2],
      ["directionalLight", 1],
    ]);

    configureQuickLookLighting({
      sceneManager: {
        updateLight(name, { intensity }) {
          intensities.set(name, intensity);
        },
      },
    });

    expect(intensities.get("ambientLight")).toBeGreaterThanOrEqual(1.5);
    expect(intensities.get("ambientLight")).toBeLessThan(2.2);
    expect(intensities.get("directionalLight")).toBeGreaterThan(1);
  });
});
