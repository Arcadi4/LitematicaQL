// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import { describe, expect, it, vi } from "vite-plus/test";
import { loadBundledResourcePack } from "./resource-pack";

describe("loadBundledResourcePack", () => {
  it("decodes the native base64 reply into the ZIP bytes", async () => {
    const handler = {
      postMessage: vi.fn().mockResolvedValue("UEsDBA=="),
    };

    const pack = await loadBundledResourcePack(handler);

    expect(handler.postMessage).toHaveBeenCalledWith({ type: "resourcePack" });
    expect(pack).toEqual(new Uint8Array([80, 75, 3, 4]));
  });

  it("fails clearly when the native bridge is unavailable", async () => {
    await expect(loadBundledResourcePack(undefined)).rejects.toThrow(
      "native resource-pack bridge is unavailable",
    );
  });

  it("rejects malformed native replies", async () => {
    const handler = { postMessage: vi.fn().mockResolvedValue(null) };

    await expect(loadBundledResourcePack(handler)).rejects.toThrow(
      "bundled block resources could not be read",
    );
  });
});
