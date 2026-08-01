import { describe, expect, it } from "vite-plus/test";
import { decodeBase64, formatDimensions } from "./bridge";

describe("decodeBase64", () => {
  it("recreates arbitrary bytes", () => {
    const bytes = new Uint8Array([0, 31, 139, 127, 128, 255]);
    const encoded = "AB+Lf4D/";

    expect(new Uint8Array(decodeBase64(encoded))).toEqual(bytes);
  });
});

describe("formatDimensions", () => {
  it("normalizes typed arrays", () => {
    expect(formatDimensions(new Int32Array([12, 7, 19]))).toEqual([12, 7, 19]);
  });

  it("rejects incomplete dimensions", () => {
    expect(formatDimensions([12, 7])).toBeUndefined();
  });
});
