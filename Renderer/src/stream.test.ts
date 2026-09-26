import { afterEach, describe, expect, it } from "vite-plus/test";
import { Texture } from "three";
import { decodeBatch, StreamUploader, uploadStream, type BatchDecoder } from "./stream";

const originalFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = originalFetch;
});

function batch(index: number, textureIndex = 0, texturePNG?: Uint8Array): ArrayBuffer {
  const vertices = 3;
  const indices = 3;
  const textureLength = texturePNG?.byteLength ?? 0;
  const buffer = new ArrayBuffer((64 + 24 + vertices * 48 + indices * 4 + textureLength + 3) & ~3);
  const bytes = new Uint8Array(buffer);
  const view = new DataView(buffer);
  bytes.set([0x4c, 0x51, 0x4d, 0x42], 0);
  view.setUint16(4, 1, true);
  view.setUint16(6, 64, true);
  view.setUint32(8, index, true);
  view.setUint32(12, 1, true);
  view.setUint32(16, vertices, true);
  view.setUint32(20, indices, true);
  view.setUint32(24, 1, true);
  view.setFloat32(28, -1, true);
  view.setFloat32(32, -2, true);
  view.setFloat32(36, -3, true);
  view.setFloat32(40, 4, true);
  view.setFloat32(44, 5, true);
  view.setFloat32(48, 6, true);
  let offset = 64;
  view.setUint32(offset, textureIndex, true);
  view.setUint32(offset + 4, textureIndex === 0 ? 0 : 0x100, true);
  view.setUint32(offset + 8, vertices, true);
  view.setUint32(offset + 12, indices, true);
  view.setUint32(offset + 16, textureLength, true);
  offset += 24;
  const values = new Float32Array(buffer, offset, vertices * 3 + vertices * 3 + vertices * 2 + vertices * 4);
  values[0] = 0;
  values[1] = 1;
  values[2] = 2;
  const indexArray = new Uint32Array(buffer, offset + vertices * 48, indices);
  indexArray.set([0, 1, 2]);
  if (texturePNG) bytes.set(texturePNG, offset + vertices * 48 + indices * 4);
  return buffer;
}

describe("native mesh batches", () => {
  it("decodes ordered typed attributes and bounds", () => {
    const payload = decodeBatch(batch(4));
    expect(payload.index).toBe(4);
    expect(payload.parts[0].positions[1]).toBe(1);
    expect(payload.parts[0].indices).toEqual(new Uint32Array([0, 1, 2]));
    expect(payload.parts[0].texturePNG).toBeUndefined();
    expect(payload.boundsMin.x).toBe(-1);
    expect(payload.boundsMax.z).toBe(6);
  });

  it("accepts a first-use texture before a following part", () => {
    const vertices = 3;
    const indices = 3;
    const textureLength = 3;
    const buffer = new ArrayBuffer(64 + 24 * 2 + vertices * 48 * 2 + indices * 4 * 2 + textureLength + 1);
    const bytes = new Uint8Array(buffer);
    const view = new DataView(buffer);
    bytes.set([0x4c, 0x51, 0x4d, 0x42], 0);
    view.setUint16(4, 1, true);
    view.setUint16(6, 64, true);
    view.setUint32(8, 0, true);
    view.setUint32(12, 2, true);
    view.setUint32(16, vertices * 2, true);
    view.setUint32(20, indices * 2, true);
    view.setUint32(24, 2, true);
    let offset = 64;
    for (let part = 0; part < 2; part += 1) {
      view.setUint32(offset, part === 0 ? 1 : 0, true);
      view.setUint32(offset + 4, part === 0 ? 0x100 : 0, true);
      view.setUint32(offset + 8, vertices, true);
      view.setUint32(offset + 12, indices, true);
      view.setUint32(offset + 16, part === 0 ? textureLength : 0, true);
      offset += 24 + vertices * 48 + indices * 4;
      if (part === 0) {
        bytes.set([1, 2, 3], offset);
        offset = (offset + textureLength + 3) & ~3;
      }
    }
    expect(offset).toBe(buffer.byteLength);
    const payload = decodeBatch(buffer);
    expect(payload.parts[0].texturePNG).toHaveLength(textureLength);
    expect(payload.parts[1].textureIndex).toBe(0);
  });

  it("reuses the atlas and first-use greedy textures", async () => {
    let decodes = 0;
    const decoder: BatchDecoder = { decode: async () => {
      decodes += 1;
      return new Texture();
    } };
    globalThis.fetch = async () => new Response(new Uint8Array([1]));
    const uploader = new StreamUploader(decoder);
    await uploader.loadAtlas("atlas");
    await uploader.stage(decodeBatch(batch(0)));
    await uploader.stage(decodeBatch(batch(1)));
    await uploader.stage(decodeBatch(batch(2, 1, new Uint8Array([2]))));
    await uploader.stage(decodeBatch(batch(3, 1)));
    const result = uploader.commit();
    expect(decodes).toBe(2);
    expect(result.batchCount).toBe(4);
    expect(result.root.children).toHaveLength(4);
    uploader.dispose();
  });

  it("stops requesting stale batches after cancellation", async () => {
    const decoder: BatchDecoder = { decode: async () => {
      const texture = new Texture();
      texture.image = { width: 1, height: 1 };
      return texture;
    } };
    globalThis.fetch = async () => new Response(new Uint8Array([1]));
    const abort = new AbortController();
    let requests = 0;
    await expect(uploadStream({
      atlasURL: "atlas",
      atlasWidth: 1,
      atlasHeight: 1,
      batchCount: 3,
      decoder,
      signal: abort.signal,
      fetchBatch: async () => {
        requests += 1;
        abort.abort();
        return new Response(batch(requests - 1));
      },
      onProgress: (progress) => {
        if (progress.phase === "upload") abort.abort();
      },
    })).rejects.toMatchObject({ name: "AbortError" });
    expect(requests).toBe(1);
  });
});
