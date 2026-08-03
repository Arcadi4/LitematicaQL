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

import { describe, expect, it, vi } from "vite-plus/test";
import { loadBundledResourcePack, loadBundledResourcePackIntoCubane } from "./resource-pack";

describe("loadBundledResourcePack", () => {
  it("turns the native base64 reply into a ZIP blob", async () => {
    const handler = {
      postMessage: vi.fn().mockResolvedValue("UEsDBA=="),
    };

    const blob = await loadBundledResourcePack(handler);

    expect(handler.postMessage).toHaveBeenCalledWith({ type: "resourcePack" });
    expect(blob.type).toBe("application/zip");
    expect(new Uint8Array(await blob.arrayBuffer())).toEqual(new Uint8Array([80, 75, 3, 4]));
  });

  it("fails clearly when the native bridge is unavailable", async () => {
    await expect(loadBundledResourcePack(undefined)).rejects.toThrow(
      "native resource-pack bridge is unavailable",
    );
  });

  it("rejects malformed native replies", async () => {
    const handler = {
      postMessage: vi.fn().mockResolvedValue(null),
    };

    await expect(loadBundledResourcePack(handler)).rejects.toThrow(
      "bundled block resources could not be read",
    );
  });
});

describe("loadBundledResourcePackIntoCubane", () => {
  it("loads the bundled ZIP directly into the in-memory asset loader", async () => {
    const pack = new Blob([new Uint8Array([80, 75, 3, 4])], { type: "application/zip" });
    const loadResourcePack = vi.fn().mockResolvedValue(undefined);
    const buildTextureAtlas = vi.fn().mockResolvedValue(undefined);
    const cubane = {
      buildTextureAtlas,
      getAssetLoader: () => ({ loadResourcePack }),
    };

    await loadBundledResourcePackIntoCubane(cubane, async () => pack);

    expect(loadResourcePack).toHaveBeenCalledOnce();
    expect(loadResourcePack).toHaveBeenCalledWith(pack);
    expect(buildTextureAtlas).toHaveBeenCalledOnce();
  });
});
