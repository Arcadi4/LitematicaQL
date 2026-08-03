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

// The batched pipeline preserves each block's palette index while yielding with
// timers, so hidden Quick Look previews do not depend on animation frames.
export const quickLookSafeMeshBuildingMode = "batched" as const;

// schematic-renderer@1.6.1 hard-codes requestAnimationFrame as the yield
// mechanism in its batched mesh builder. Quick Look may suspend that callback
// while the preview is hidden, so scope a timer-backed scheduler to mesh loading.
interface QuickLookAnimationFrameHost {
  requestAnimationFrame: (callback: FrameRequestCallback) => number;
  cancelAnimationFrame: (handle: number) => void;
  setTimeout: (callback: () => void, delay: number) => number;
  clearTimeout: (handle: number) => void;
}

export async function withQuickLookTimerBackedAnimationFrames<T>(
  operation: () => Promise<T>,
  host: QuickLookAnimationFrameHost = window,
): Promise<T> {
  const originalRequestAnimationFrame = host.requestAnimationFrame;
  const originalCancelAnimationFrame = host.cancelAnimationFrame;
  const pendingTimerHandles = new Set<number>();

  host.requestAnimationFrame = (callback) => {
    let handle = 0;
    handle = host.setTimeout(() => {
      pendingTimerHandles.delete(handle);
      callback(performance.now());
    }, 0);
    pendingTimerHandles.add(handle);
    return handle;
  };
  host.cancelAnimationFrame = (handle) => {
    if (!pendingTimerHandles.delete(handle)) {
      originalCancelAnimationFrame.call(host, handle);
      return;
    }

    host.clearTimeout(handle);
  };

  try {
    return await operation();
  } finally {
    for (const handle of pendingTimerHandles) {
      host.clearTimeout(handle);
    }
    host.requestAnimationFrame = originalRequestAnimationFrame;
    host.cancelAnimationFrame = originalCancelAnimationFrame;
  }
}

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
