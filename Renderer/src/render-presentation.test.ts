// LitematicaQL: macOS Quick Look plugin for Litematica schematics.
// Copyright (C) 2026 4rcadia
// SPDX-License-Identifier: AGPL-3.0-or-later

import { describe, expect, it } from "vite-plus/test";
import { presentRendererFrame } from "./render-presentation";

describe("presentRendererFrame", () => {
  it("does not depend on animation-frame scheduling", () => {
    const renderer = {
      invalidate: () => undefined,
      renderManager: {
        render: () => undefined,
      },
    };

    expect(presentRendererFrame(renderer)).toBeUndefined();
  });

  it("renders the completed scene immediately", () => {
    const calls: string[] = [];

    presentRendererFrame({
      invalidate: () => calls.push("invalidate"),
      renderManager: {
        render: () => calls.push("render"),
      },
    });

    expect(calls).toEqual(["invalidate", "render"]);
  });
});
