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

// Quick Look can suspend requestAnimationFrame while preparing a hidden preview.
// The instanced pipeline does not depend on animation frames during mesh construction.
export const quickLookSafeMeshBuildingMode = "instanced" as const;

// Quick Look hosts this renderer in WKWebView. schematic-renderer's
// EffectComposer path does not present scene color there, so keep the direct
// WebGL path and create readable face shading with the renderer's scene lights.
export const quickLookPostProcessingOptions = {
  enabled: false,
  enableSSAO: false,
  enableSMAA: false,
  enableGamma: false,
} as const;

interface QuickLookLightingTarget {
  sceneManager: {
    updateLight(name: string, properties: { intensity: number }): void;
  };
}

const quickLookLightIntensity = {
  ambientLight: 1.5,
  directionalLight: 1.4,
} as const;

export function configureQuickLookLighting(renderer: QuickLookLightingTarget): void {
  for (const [name, intensity] of Object.entries(quickLookLightIntensity)) {
    renderer.sceneManager.updateLight(name, { intensity });
  }
}
