// LitematicaQL: macOS Quick Look plugin for Litematica schematics.
// Copyright (C) 2026 4rcadia
// SPDX-License-Identifier: AGPL-3.0-or-later

import { describe, expect, it } from "vite-plus/test";
import { presentRendererFrame } from "./render-presentation";

describe("presentRendererFrame", () => {
  it("renders immediately and waits until that frame can be presented", async () => {
    const calls: string[] = [];
    const pendingFrames: FrameRequestCallback[] = [];
    let settled = false;

    const presentation = presentRendererFrame(
      {
        invalidate: () => calls.push("invalidate"),
        renderManager: {
          render: () => calls.push("render"),
        },
      },
      (callback) => {
        pendingFrames.push(callback);
        return pendingFrames.length;
      },
    ).then(() => {
      settled = true;
    });

    expect(calls).toEqual(["invalidate", "render"]);
    expect(pendingFrames).toHaveLength(1);

    pendingFrames.shift()?.(0);
    await Promise.resolve();
    expect(settled).toBe(false);
    expect(pendingFrames).toHaveLength(1);

    pendingFrames.shift()?.(16);
    await presentation;
    expect(settled).toBe(true);
  });
});
