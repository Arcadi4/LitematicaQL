// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import { describe, expect, it } from "vite-plus/test";
import { decodeBase64 } from "./bridge";

describe("decodeBase64", () => {
  it("recreates arbitrary bytes", () => {
    const bytes = new Uint8Array([0, 31, 139, 127, 128, 255]);

    expect(decodeBase64("AB+Lf4D/")).toEqual(bytes);
  });
});
