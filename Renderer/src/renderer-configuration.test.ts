import { describe, expect, it } from "vite-plus/test";
import { quickLookSafeMeshBuildingMode } from "./renderer-configuration";

describe("quickLookSafeMeshBuildingMode", () => {
  it("avoids the animation-frame-dependent batched pipeline", () => {
    expect(quickLookSafeMeshBuildingMode).toBe("incremental");
    expect(quickLookSafeMeshBuildingMode).not.toBe("batched");
  });
});
