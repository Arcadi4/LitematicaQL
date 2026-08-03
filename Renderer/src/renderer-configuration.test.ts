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
  withQuickLookTimerBackedAnimationFrames,
} from "./renderer-configuration";

describe("quickLookSafeMeshBuildingMode", () => {
  it("uses the state-preserving batched mesh pipeline", () => {
    expect(quickLookSafeMeshBuildingMode).toBe("batched");
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

describe("withQuickLookTimerBackedAnimationFrames", () => {
  it("uses timer callbacks during the operation and restores the host", async () => {
    let nextHandle = 0;
    const pendingTimers = new Map<number, ReturnType<typeof setTimeout>>();
    const originalRequestAnimationFrame = (_callback: FrameRequestCallback) => 1;
    const originalCancelAnimationFrame = (_handle: number) => undefined;
    const host = {
      requestAnimationFrame: originalRequestAnimationFrame,
      cancelAnimationFrame: originalCancelAnimationFrame,
      setTimeout(callback: () => void, delay: number) {
        const handle = ++nextHandle;
        pendingTimers.set(
          handle,
          setTimeout(() => {
            pendingTimers.delete(handle);
            callback();
          }, delay),
        );
        return handle;
      },
      clearTimeout(handle: number) {
        const timer = pendingTimers.get(handle);
        if (timer) {
          clearTimeout(timer);
        }
        pendingTimers.delete(handle);
      },
    };
    let timestamp: number | undefined;

    await withQuickLookTimerBackedAnimationFrames(async () => {
      await new Promise<void>((resolve) => {
        host.requestAnimationFrame((nextTimestamp) => {
          timestamp = nextTimestamp;
          resolve();
        });
      });
    }, host);

    expect(timestamp).toEqual(expect.any(Number));
    expect(host.requestAnimationFrame).toBe(originalRequestAnimationFrame);
    expect(host.cancelAnimationFrame).toBe(originalCancelAnimationFrame);
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
