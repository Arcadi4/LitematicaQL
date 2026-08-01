import { describe, expect, it, vi } from "vite-plus/test";
import { loadBundledResourcePack } from "./resource-pack";

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
