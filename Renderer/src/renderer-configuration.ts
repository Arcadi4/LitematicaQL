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
// The incremental pipeline yields with timers and does not wait for animation frames.
export const quickLookSafeMeshBuildingMode = "incremental" as const;

// Screen-space ambient occlusion restores contact shading between adjacent
// blocks. The tighter radius keeps the orthographic preview from looking dirty.
export const quickLookPostProcessingOptions = {
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
} as const;
