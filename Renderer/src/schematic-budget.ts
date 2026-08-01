import * as pako from "pako";

export const maximumInflatedBytes = 64 * 1_024 * 1_024;
export const maximumRegionCount = 64;
export const maximumAxisLength = 4_096;
export const maximumSchematicVolume = 16_777_216;
export const maximumRenderedBlocks = 8_388_608;

const compressionChunkSize = 64 * 1_024;
const maximumPaletteEntries = 4_096;
const maximumEntityEntries = 100_000;
const maximumNestedDepth = 64;
const maximumParsedTags = 500_000;
const maximumGenericListEntries = 1_000_000;

const tag = {
  end: 0,
  byte: 1,
  short: 2,
  int: 3,
  long: 4,
  float: 5,
  double: 6,
  byteArray: 7,
  string: 8,
  list: 9,
  compound: 10,
  intArray: 11,
  longArray: 12,
} as const;

type RegionInspection = {
  blockStatesLength?: number;
  dimensions?: [number, number, number];
  paletteLength?: number;
  position?: [number, number, number];
};

export type SchematicInspection = {
  inflatedBytes: number;
  regionCount: number;
  totalVolume: number;
};

export type ParsedSchematicStats = {
  blockCount: number;
  dimensions: ArrayLike<number>;
  volume: number;
};

export class SchematicBudgetError extends Error {
  override name = "SchematicBudgetError";
}

export function inspectCompressedLitematic(compressed: Uint8Array): SchematicInspection {
  const inflatedBytes = measureInflatedSize(compressed);
  const inflated = inflateForInspection(compressed, inflatedBytes);
  const inspection = new LitematicNbtInspector(inflated).inspect();
  return { inflatedBytes, ...inspection };
}

export function measureInflatedSize(
  compressed: Uint8Array,
  maximumBytes = maximumInflatedBytes,
): number {
  let inflatedBytes = 0;
  runInflater(compressed, (chunk) => {
    inflatedBytes += chunk.byteLength;
    if (inflatedBytes > maximumBytes) {
      throw new SchematicBudgetError(
        `The schematic expands beyond the ${formatMebibytes(maximumBytes)} preview limit.`,
      );
    }
  });
  return inflatedBytes;
}

export function assertParsedSchematicWithinBudget(stats: ParsedSchematicStats): void {
  const dimensions = Array.from(stats.dimensions);
  if (dimensions.length !== 3) {
    throw invalidStructure("The parsed schematic does not have three dimensions.");
  }

  let dimensionProduct = 1n;
  for (const dimension of dimensions) {
    if (!Number.isSafeInteger(dimension) || dimension <= 0 || dimension > maximumAxisLength) {
      throw new SchematicBudgetError(
        `The schematic exceeds the ${maximumAxisLength}-block per-axis preview limit.`,
      );
    }
    dimensionProduct *= BigInt(dimension);
  }

  if (dimensionProduct > BigInt(maximumSchematicVolume)) {
    throw volumeLimitError();
  }
  if (!isSafeCount(stats.volume) || stats.volume > maximumSchematicVolume) {
    throw volumeLimitError();
  }
  if (!isSafeCount(stats.blockCount) || stats.blockCount > maximumRenderedBlocks) {
    throw new SchematicBudgetError(
      `The schematic exceeds the ${maximumRenderedBlocks.toLocaleString()}-block preview limit.`,
    );
  }
}

function inflateForInspection(compressed: Uint8Array, inflatedBytes: number): Uint8Array {
  const output = new Uint8Array(inflatedBytes);
  let offset = 0;
  runInflater(compressed, (chunk) => {
    if (offset + chunk.byteLength > output.byteLength) {
      throw invalidStructure("The gzip stream changed while it was being inspected.");
    }
    output.set(chunk, offset);
    offset += chunk.byteLength;
  });

  if (offset !== output.byteLength) {
    throw invalidStructure("The gzip stream ended at an unexpected size.");
  }
  return output;
}

function runInflater(compressed: Uint8Array, onData: (chunk: Uint8Array) => void): void {
  const inflater = new pako.Inflate({ chunkSize: compressionChunkSize });
  inflater.onData = (chunk) => {
    onData(chunk instanceof Uint8Array ? chunk : new Uint8Array(chunk));
  };

  try {
    for (let offset = 0; offset < compressed.byteLength; offset += compressionChunkSize) {
      const end = Math.min(offset + compressionChunkSize, compressed.byteLength);
      const isLast = end === compressed.byteLength;
      if (!inflater.push(compressed.subarray(offset, end), isLast)) {
        break;
      }
    }
  } catch (error) {
    if (error instanceof SchematicBudgetError) {
      throw error;
    }
    throw invalidStructure(normalizeMessage(error));
  }

  if (inflater.err !== 0) {
    throw invalidStructure(inflater.msg || "The gzip stream is incomplete or corrupt.");
  }
}

class LitematicNbtInspector {
  private offset = 0;
  private parsedTags = 0;
  private regionCount = 0;
  private totalVolume = 0n;
  private readonly globalMinimum: Array<bigint | undefined> = [undefined, undefined, undefined];
  private readonly globalMaximum: Array<bigint | undefined> = [undefined, undefined, undefined];
  private readonly decoder = new TextDecoder("utf-8", { fatal: true });

  constructor(private readonly bytes: Uint8Array) {}

  inspect(): Omit<SchematicInspection, "inflatedBytes"> {
    if (this.readUnsignedByte() !== tag.compound) {
      throw invalidStructure("The NBT root is not a compound tag.");
    }
    this.readString();
    this.readRootCompound(0);
    if (this.offset !== this.bytes.byteLength) {
      throw invalidStructure("The NBT document contains trailing data.");
    }
    if (this.regionCount === 0) {
      throw invalidStructure("The schematic does not contain any regions.");
    }
    return {
      regionCount: this.regionCount,
      totalVolume: Number(this.totalVolume),
    };
  }

  private readRootCompound(depth: number): void {
    this.readNamedTags(depth, (type, name, childDepth) => {
      if (name === "Regions") {
        this.requireType(type, tag.compound, "Regions");
        this.readRegionsCompound(childDepth);
      } else {
        this.skipPayload(type, childDepth);
      }
    });
  }

  private readRegionsCompound(depth: number): void {
    this.readNamedTags(depth, (type, _name, childDepth) => {
      this.requireType(type, tag.compound, "region");
      this.regionCount += 1;
      if (this.regionCount > maximumRegionCount) {
        throw new SchematicBudgetError(
          `The schematic exceeds the ${maximumRegionCount}-region preview limit.`,
        );
      }
      this.readRegionCompound(childDepth);
    });
  }

  private readRegionCompound(depth: number): void {
    const region: RegionInspection = {};
    this.readNamedTags(depth, (type, name, childDepth) => {
      switch (name) {
        case "Size":
          this.requireType(type, tag.compound, "region Size");
          region.dimensions = this.readVectorCompound("Size", childDepth);
          break;
        case "Position":
          this.requireType(type, tag.compound, "region Position");
          region.position = this.readVectorCompound("Position", childDepth);
          break;
        case "BlockStatePalette":
          this.requireType(type, tag.list, "BlockStatePalette");
          region.paletteLength = this.readPaletteList(childDepth);
          break;
        case "BlockStates":
          this.requireType(type, tag.longArray, "BlockStates");
          region.blockStatesLength = this.readArrayLength(8, "BlockStates");
          break;
        case "Entities":
        case "TileEntities":
          this.requireType(type, tag.list, name);
          this.readEntityList(name, childDepth);
          break;
        default:
          this.skipPayload(type, childDepth);
      }
    });
    this.validateRegion(region);
  }

  private readVectorCompound(label: string, depth: number): [number, number, number] {
    const axes: Partial<Record<"x" | "y" | "z", number>> = {};
    this.readNamedTags(depth, (type, tagName, childDepth) => {
      if (tagName === "x" || tagName === "y" || tagName === "z") {
        this.requireType(type, tag.int, `${label}.${tagName}`);
        axes[tagName] = this.readInt();
      } else {
        this.skipPayload(type, childDepth);
      }
    });
    if (axes.x === undefined || axes.y === undefined || axes.z === undefined) {
      throw invalidStructure(`A region ${label} is missing an axis.`);
    }
    return [axes.x, axes.y, axes.z];
  }

  private readPaletteList(depth: number): number {
    const elementType = this.readUnsignedByte();
    const length = this.readLength("BlockStatePalette");
    if (elementType !== tag.compound || length === 0 || length > maximumPaletteEntries) {
      throw invalidStructure("A region has an invalid block-state palette.");
    }
    for (let index = 0; index < length; index += 1) {
      this.skipPayload(elementType, depth);
    }
    return length;
  }

  private readEntityList(name: string, depth: number): void {
    const elementType = this.readUnsignedByte();
    const length = this.readLength(name);
    if (elementType !== tag.compound || length > maximumEntityEntries) {
      throw invalidStructure(`${name} exceeds the supported preview structure.`);
    }
    for (let index = 0; index < length; index += 1) {
      this.skipPayload(elementType, depth);
    }
  }

  private validateRegion(region: RegionInspection): void {
    const { dimensions, position, paletteLength, blockStatesLength } = region;
    if (
      !dimensions ||
      !position ||
      paletteLength === undefined ||
      blockStatesLength === undefined
    ) {
      throw invalidStructure(
        "A region is missing Position, Size, BlockStatePalette, or BlockStates.",
      );
    }

    let volume = 1n;
    for (let axis = 0; axis < dimensions.length; axis += 1) {
      const signedDimension = dimensions[axis]!;
      const dimension = Math.abs(signedDimension);
      if (!Number.isSafeInteger(dimension) || dimension === 0 || dimension > maximumAxisLength) {
        throw new SchematicBudgetError(
          `The schematic exceeds the ${maximumAxisLength}-block per-axis preview limit.`,
        );
      }
      volume *= BigInt(dimension);
      this.extendGlobalBounds(axis, position[axis]!, signedDimension);
    }

    this.totalVolume += volume;
    if (this.totalVolume > BigInt(maximumSchematicVolume)) {
      throw volumeLimitError();
    }
    this.validateGlobalBounds();

    const bitsPerBlock = Math.max(2, Math.ceil(Math.log2(paletteLength)));
    const expectedLongs = (volume * BigInt(bitsPerBlock) + 63n) / 64n;
    if (BigInt(blockStatesLength) !== expectedLongs) {
      throw invalidStructure("A region BlockStates array does not match its declared size.");
    }
  }

  private extendGlobalBounds(axis: number, position: number, signedSize: number): void {
    const start = BigInt(position);
    const end = start + BigInt(signedSize > 0 ? signedSize - 1 : signedSize + 1);
    const coordinateMinimum = -(1n << 31n);
    const coordinateMaximum = (1n << 31n) - 1n;
    if (end < coordinateMinimum || end > coordinateMaximum) {
      throw invalidStructure("A region extends beyond the supported coordinate range.");
    }

    const regionMinimum = start < end ? start : end;
    const regionMaximum = start > end ? start : end;
    const currentMinimum = this.globalMinimum[axis];
    const currentMaximum = this.globalMaximum[axis];
    this.globalMinimum[axis] =
      currentMinimum === undefined || regionMinimum < currentMinimum
        ? regionMinimum
        : currentMinimum;
    this.globalMaximum[axis] =
      currentMaximum === undefined || regionMaximum > currentMaximum
        ? regionMaximum
        : currentMaximum;
  }

  private validateGlobalBounds(): void {
    let boundingVolume = 1n;
    for (let axis = 0; axis < 3; axis += 1) {
      const minimum = this.globalMinimum[axis];
      const maximum = this.globalMaximum[axis];
      if (minimum === undefined || maximum === undefined) {
        throw invalidStructure("The schematic bounding box is incomplete.");
      }
      const span = maximum - minimum + 1n;
      if (span > BigInt(maximumAxisLength)) {
        throw new SchematicBudgetError(
          `The schematic exceeds the ${maximumAxisLength}-block per-axis preview limit.`,
        );
      }
      boundingVolume *= span;
    }
    if (boundingVolume > BigInt(maximumSchematicVolume)) {
      throw volumeLimitError();
    }
  }

  private readNamedTags(
    depth: number,
    readTag: (type: number, name: string, childDepth: number) => void,
  ): void {
    this.checkDepth(depth);
    while (true) {
      const type = this.readUnsignedByte();
      if (type === tag.end) {
        return;
      }
      this.countTag();
      const name = this.readString();
      readTag(type, name, depth + 1);
    }
  }

  private skipPayload(type: number, depth: number): void {
    this.checkDepth(depth);
    switch (type) {
      case tag.byte:
        this.skip(1);
        return;
      case tag.short:
        this.skip(2);
        return;
      case tag.int:
      case tag.float:
        this.skip(4);
        return;
      case tag.long:
      case tag.double:
        this.skip(8);
        return;
      case tag.byteArray:
        this.readArrayLength(1, "byte array");
        return;
      case tag.string:
        this.skip(this.readUnsignedShort());
        return;
      case tag.list:
        this.skipList(depth + 1);
        return;
      case tag.compound:
        this.readNamedTags(depth + 1, (childType, _name, childDepth) => {
          this.skipPayload(childType, childDepth);
        });
        return;
      case tag.intArray:
        this.readArrayLength(4, "int array");
        return;
      case tag.longArray:
        this.readArrayLength(8, "long array");
        return;
      default:
        throw invalidStructure(`Unknown NBT tag type ${type}.`);
    }
  }

  private skipList(depth: number): void {
    const elementType = this.readUnsignedByte();
    const length = this.readLength("list");
    if (length > maximumGenericListEntries || (elementType === tag.end && length !== 0)) {
      throw invalidStructure("An NBT list has an invalid length or element type.");
    }

    const fixedWidth = fixedPayloadWidth(elementType);
    if (fixedWidth !== undefined) {
      this.skip(length * fixedWidth);
      return;
    }
    for (let index = 0; index < length; index += 1) {
      this.countTag();
      this.skipPayload(elementType, depth);
    }
  }

  private readArrayLength(elementWidth: number, name: string): number {
    const length = this.readLength(name);
    this.skip(length * elementWidth);
    return length;
  }

  private readLength(name: string): number {
    const length = this.readInt();
    if (length < 0) {
      throw invalidStructure(`${name} has a negative length.`);
    }
    return length;
  }

  private readString(): string {
    const length = this.readUnsignedShort();
    this.ensure(length);
    const start = this.offset;
    this.offset += length;
    try {
      return this.decoder.decode(this.bytes.subarray(start, this.offset));
    } catch {
      throw invalidStructure("The NBT document contains invalid UTF-8.");
    }
  }

  private readUnsignedByte(): number {
    this.ensure(1);
    return this.bytes[this.offset++]!;
  }

  private readUnsignedShort(): number {
    this.ensure(2);
    const value = (this.bytes[this.offset]! << 8) | this.bytes[this.offset + 1]!;
    this.offset += 2;
    return value;
  }

  private readInt(): number {
    this.ensure(4);
    const view = new DataView(this.bytes.buffer, this.bytes.byteOffset + this.offset, 4);
    const value = view.getInt32(0, false);
    this.offset += 4;
    return value;
  }

  private skip(length: number): void {
    if (!Number.isSafeInteger(length) || length < 0) {
      throw invalidStructure("An NBT payload length is invalid.");
    }
    this.ensure(length);
    this.offset += length;
  }

  private ensure(length: number): void {
    if (this.offset + length > this.bytes.byteLength) {
      throw invalidStructure("The NBT document ended unexpectedly.");
    }
  }

  private countTag(): void {
    this.parsedTags += 1;
    if (this.parsedTags > maximumParsedTags) {
      throw invalidStructure("The NBT document contains too many tags.");
    }
  }

  private checkDepth(depth: number): void {
    if (depth > maximumNestedDepth) {
      throw invalidStructure("The NBT document is nested too deeply.");
    }
  }

  private requireType(actual: number, expected: number, name: string): void {
    if (actual !== expected) {
      throw invalidStructure(`${name} has an unexpected NBT type.`);
    }
  }
}

function fixedPayloadWidth(type: number): number | undefined {
  switch (type) {
    case tag.byte:
      return 1;
    case tag.short:
      return 2;
    case tag.int:
    case tag.float:
      return 4;
    case tag.long:
    case tag.double:
      return 8;
    default:
      return undefined;
  }
}

function isSafeCount(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function invalidStructure(detail: string): SchematicBudgetError {
  return new SchematicBudgetError(`The schematic structure is invalid. ${detail}`);
}

function volumeLimitError(): SchematicBudgetError {
  return new SchematicBudgetError(
    `The schematic exceeds the ${maximumSchematicVolume.toLocaleString()}-cell preview limit.`,
  );
}

function formatMebibytes(bytes: number): string {
  return `${bytes / (1_024 * 1_024)} MiB`;
}

function normalizeMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
