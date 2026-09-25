import {
  BufferAttribute,
  BufferGeometry,
  DoubleSide,
  FrontSide,
  Group,
  NearestFilter,
  Mesh,
  MeshStandardMaterial,
  RepeatWrapping,
  Texture,
  SRGBColorSpace,
  Vector4,
} from "three";

const batchMagic = "LQMB";
const batchVersion = 1;
const batchHeaderBytes = 64;
const partHeaderBytes = 24;
const opaque = 0;
const mask = 1;
const blend = 2;
const repeatFlag = 0x100;

export interface BatchPart {
  textureIndex: number;
  alphaMode: number;
  repeat: boolean;
  vertexCount: number;
  indexCount: number;
  positions: Float32Array;
  normals: Float32Array;
  uvs: Float32Array;
  colors: Float32Array;
  indices: Uint32Array;
  texturePNG: Uint8Array | undefined;
}

export interface BatchPayload {
  index: number;
  partCount: number;
  vertexCount: number;
  indexCount: number;
  triangleCount: number;
  boundsMin: Vector4;
  boundsMax: Vector4;
  parts: BatchPart[];
  byteLength: number;
}

export interface BatchDecoder {
  decode(bytes: ArrayBuffer): Promise<Texture>;
}

export interface BatchProgress {
  phase: "atlas" | "upload";
  completed: number;
  total: number;
  bytes: number;
  batchBytes: number;
  triangles: number;
}

export interface StreamUploaderOptions {
  atlasURL: string;
  atlasWidth: number;
  atlasHeight: number;
  batchCount: number;
  fetchBatch(index: number): Promise<Response>;
  decoder?: BatchDecoder;
  signal?: AbortSignal;
  onProgress?(progress: BatchProgress): void;
  uploader?: StreamUploader;
  onStaged?(): void;
}

export interface StreamUploadResult {
  root: Group;
  triangles: number;
  bytes: number;
  batchCount: number;
  atlasBytes: number;
}

class PNGDecoder implements BatchDecoder {
  async decode(bytes: ArrayBuffer): Promise<Texture> {
    const bitmap = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
    const texture = new Texture(bitmap);
    configureTexture(texture);
    return texture;
  }
}

export class StreamUploader {
  readonly root = new Group();
  private readonly decoder: BatchDecoder;
  private readonly textures = new Map<number, Texture>();
  private readonly materials = new Map<string, MeshStandardMaterial>();
  private readonly geometries = new Set<BufferGeometry>();
  private atlasTexture: Texture | undefined;
  private atlasBytes = 0;
  private transferredBytes = 0;
  private triangles = 0;
  private nextBatch = 0;

  constructor(decoder: BatchDecoder = new PNGDecoder()) {
    this.decoder = decoder;
  }

  async loadAtlas(
    url: string,
    signal?: AbortSignal,
    expectedWidth?: number,
    expectedHeight?: number,
  ): Promise<number> {
    if (this.atlasTexture) {
      throw new Error("The shared atlas was uploaded more than once.");
    }
    const response = await fetch(url, { signal, cache: "no-store" });
    if (!response.ok) {
      throw new Error(`The shared atlas could not be read (HTTP ${response.status}).`);
    }
    const bytes = await response.arrayBuffer();
    const texture = await this.decoder.decode(bytes);
    configureTexture(texture);
    const image = texture.image as { width?: number; height?: number } | undefined;
    if (expectedWidth !== undefined && image?.width !== expectedWidth) {
      throw new Error("The shared atlas width does not match native metadata.");
    }
    if (expectedHeight !== undefined && image?.height !== expectedHeight) {
      throw new Error("The shared atlas height does not match native metadata.");
    }
    this.atlasTexture = texture;
    this.textures.set(0, texture);
    this.atlasBytes = bytes.byteLength;
    this.transferredBytes = bytes.byteLength;
    return bytes.byteLength;
  }

  async stage(payload: BatchPayload, transferredBytes = 0): Promise<void> {
    if (payload.index !== this.nextBatch) {
      throw new Error(`Mesh batch ${payload.index} arrived out of order.`);
    }
    for (const part of payload.parts) {
      const texture = await this.textureFor(part);
      const geometry = new BufferGeometry();
      geometry.setAttribute("position", new BufferAttribute(part.positions, 3));
      geometry.setAttribute("normal", new BufferAttribute(part.normals, 3));
      geometry.setAttribute("uv", new BufferAttribute(part.uvs, 2));
      geometry.setAttribute("color", new BufferAttribute(part.colors, 4));
      geometry.setIndex(new BufferAttribute(part.indices, 1));
      this.geometries.add(geometry);
      const material = this.materialFor(texture, part.alphaMode, part.repeat);
      const mesh = new Mesh(geometry, material);
      mesh.frustumCulled = false;
      this.root.add(mesh);
    }
    this.nextBatch = payload.index + 1;
    this.triangles += payload.triangleCount;
    this.transferredBytes += transferredBytes;
  }

  commit(): StreamUploadResult {
    for (const material of this.materials.values()) {
      material.colorWrite = true;
      material.depthWrite = material.userData.depthWrite as boolean;
      material.needsUpdate = true;
    }
    return {
      root: this.root,
      triangles: this.triangles,
      bytes: this.bytes(),
      batchCount: this.nextBatch,
      atlasBytes: this.atlasBytes,
    };
  }


  bytes(): number {
    return this.transferredBytes;
  }

  dispose(): void {
    for (const geometry of this.geometries) geometry.dispose();
    for (const material of this.materials.values()) material.dispose();
    for (const texture of this.textures.values()) {
      const image = texture.image;
      if (typeof ImageBitmap !== "undefined" && image instanceof ImageBitmap) image.close();
      texture.dispose();
    }
    this.geometries.clear();
    this.materials.clear();
    this.textures.clear();
    this.atlasTexture = undefined;
  }

  private async textureFor(part: BatchPart): Promise<Texture> {
    if (this.textures.has(part.textureIndex)) {
      if (part.texturePNG) {
        throw new Error(`Texture ${part.textureIndex} was uploaded more than once.`);
      }
      return this.textures.get(part.textureIndex)!;
    }
    if (part.textureIndex === 0 || !part.texturePNG) {
      throw new Error(`Texture ${part.textureIndex} has no first-use payload.`);
    }
    const source = part.texturePNG.slice().buffer;
    const texture = await this.decoder.decode(source);
    configureTexture(texture);
    this.textures.set(part.textureIndex, texture);
    return texture;
  }

  private materialFor(
    texture: Texture,
    alphaMode: number,
    repeat: boolean,
  ): MeshStandardMaterial {
    const key = `${texture.uuid}:${alphaMode}:${repeat ? 1 : 0}`;
    const existing = this.materials.get(key);
    if (existing) return existing;
    const transparent = alphaMode === blend;
    const material = new MeshStandardMaterial({
      map: texture,
      vertexColors: true,
      alphaTest: alphaMode === mask ? 0.5 : 0,
      transparent,
      depthWrite: !transparent,
      side: alphaMode === mask || transparent ? DoubleSide : FrontSide,
      roughness: 1,
      metalness: 0,
    });
    material.colorWrite = false;
    material.depthWrite = false;
    material.userData.depthWrite = !transparent;
    this.materials.set(key, material);
    return material;
  }
}

export async function uploadStream(options: StreamUploaderOptions): Promise<StreamUploadResult> {
  if (!Number.isSafeInteger(options.batchCount) || options.batchCount < 0) {
    throw new Error("The native mesh reported an invalid batch count.");
  }
  const uploader = options.uploader ?? new StreamUploader(options.decoder);
  const atlasBytes = await uploader.loadAtlas(
    options.atlasURL,
    options.signal,
    options.atlasWidth,
    options.atlasHeight,
  );
  options.onProgress?.({
    phase: "atlas",
    completed: 1,
    total: options.batchCount + 1,
    bytes: atlasBytes,
    batchBytes: atlasBytes,
    triangles: 0,
  });
  let bytes = atlasBytes;
  for (let index = 0; index < options.batchCount; index += 1) {
    if (options.signal?.aborted) throw new DOMException("Preview cancelled", "AbortError");
    const response = await options.fetchBatch(index);
    if (!response.ok) {
      throw new Error(`Mesh batch ${index + 1} could not be read (HTTP ${response.status}).`);
    }
    const payloadBytes = await response.arrayBuffer();
    const payload = decodeBatch(payloadBytes);
    await uploader.stage(payload, payloadBytes.byteLength);
    options.onStaged?.();
    bytes += payloadBytes.byteLength;
    options.onProgress?.({
      phase: "upload",
      completed: index + 2,
      total: options.batchCount + 1,
      bytes,
      batchBytes: payloadBytes.byteLength,
      triangles: payload.triangleCount,
    });
  }
  const result = uploader.commit();
  options.onProgress?.({
    phase: "upload",
    completed: options.batchCount + 1,
    total: options.batchCount + 1,
    bytes: result.bytes,
    batchBytes: 0,
    triangles: result.triangles,
  });
  return result;
}

export function decodeBatch(buffer: ArrayBuffer): BatchPayload {
  const view = new DataView(buffer);
  if (buffer.byteLength < batchHeaderBytes || readAscii(view, 0, 4) !== batchMagic) {
    throw new Error("The native mesh returned an invalid batch header.");
  }
  if (view.getUint16(4, true) !== batchVersion || view.getUint16(6, true) !== batchHeaderBytes) {
    throw new Error("The native mesh batch version is not supported.");
  }
  const index = view.getUint32(8, true);
  const partCount = view.getUint32(12, true);
  const vertexCount = view.getUint32(16, true);
  const indexCount = view.getUint32(20, true);
  const triangleCount = view.getUint32(24, true);
  const boundsMin = new Vector4(
    view.getFloat32(28, true),
    view.getFloat32(32, true),
    view.getFloat32(36, true),
    0,
  );
  const boundsMax = new Vector4(
    view.getFloat32(40, true),
    view.getFloat32(44, true),
    view.getFloat32(48, true),
    0,
  );
  if (triangleCount !== indexCount / 3 || partCount > 1_000_000) {
    throw new Error("The native mesh returned inconsistent batch counts.");
  }
  const parts: BatchPart[] = [];
  let offset = batchHeaderBytes;
  let vertices = 0;
  let indices = 0;
  for (let partIndex = 0; partIndex < partCount; partIndex += 1) {
    requireBytes(buffer, offset, partHeaderBytes);
    const textureIndex = view.getUint32(offset, true);
    const flags = view.getUint32(offset + 4, true);
    const partVertices = view.getUint32(offset + 8, true);
    const partIndices = view.getUint32(offset + 12, true);
    const textureLength = view.getUint32(offset + 16, true);
    const alphaMode = flags & 0xff;
    if (textureIndex === 0 && textureLength !== 0) throw new Error("The shared atlas was embedded in a geometry batch.");
    if (alphaMode > blend || partIndices % 3 !== 0) throw new Error("The native mesh returned an invalid batch part.");
    const attributeBytes = partVertices * 48 + partIndices * 4 + textureLength;
    requireBytes(buffer, offset + partHeaderBytes, attributeBytes);
    let dataOffset = offset + partHeaderBytes;
    const positions = new Float32Array(buffer, dataOffset, partVertices * 3);
    dataOffset += partVertices * 12;
    const normals = new Float32Array(buffer, dataOffset, partVertices * 3);
    dataOffset += partVertices * 12;
    const uvs = new Float32Array(buffer, dataOffset, partVertices * 2);
    dataOffset += partVertices * 8;
    const colors = new Float32Array(buffer, dataOffset, partVertices * 4);
    dataOffset += partVertices * 16;
    const indexArray = new Uint32Array(buffer, dataOffset, partIndices);
    for (const value of indexArray) {
      if (value >= partVertices) throw new Error("The native mesh returned an out-of-range index.");
    }
    dataOffset += partIndices * 4;
    const texturePNG = textureLength
      ? new Uint8Array(buffer, dataOffset, textureLength)
      : undefined;
    parts.push({
      textureIndex,
      alphaMode,
      repeat: (flags & repeatFlag) !== 0,
      vertexCount: partVertices,
      indexCount: partIndices,
      positions,
      normals,
      uvs,
      colors,
      indices: indexArray,
      texturePNG,
    });
    vertices += partVertices;
    indices += partIndices;
    offset = dataOffset + textureLength;
  }
  if (offset !== buffer.byteLength || vertices !== vertexCount || indices !== indexCount) {
    throw new Error("The native mesh returned truncated or oversized batch data.");
  }
  return { index, partCount, vertexCount, indexCount, triangleCount, boundsMin, boundsMax, parts, byteLength: buffer.byteLength };
}

function configureTexture(texture: Texture): void {
  texture.colorSpace = SRGBColorSpace;
  texture.magFilter = NearestFilter;
  texture.minFilter = NearestFilter;
  texture.wrapS = RepeatWrapping;
  texture.wrapT = RepeatWrapping;
  texture.generateMipmaps = false;
  texture.needsUpdate = true;
}

function requireBytes(buffer: ArrayBuffer, offset: number, length: number): void {
  if (offset < 0 || length < 0 || offset + length > buffer.byteLength) {
    throw new Error("The native mesh returned truncated batch data.");
  }
}

function readAscii(view: DataView, offset: number, length: number): string {
  let value = "";
  for (let index = 0; index < length; index += 1) value += String.fromCharCode(view.getUint8(offset + index));
  return value;
}
