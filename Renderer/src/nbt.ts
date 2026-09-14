// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import {
  maximumNbtCollectionItems,
  maximumNbtDepth,
  maximumNbtNodes,
  SchematicFormatError,
} from "./schematic-limits";

/** NBT tag ids, as defined by the Java edition format. */
export type NbtTagId = 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12;

export interface NbtNumber {
  readonly tag: "byte" | "short" | "int" | "float" | "double";
  readonly value: number;
}

export interface NbtLong {
  readonly tag: "long";
  readonly value: bigint;
}

export interface NbtString {
  readonly tag: "string";
  readonly value: string;
}

export interface NbtByteArray {
  readonly tag: "byteArray";
  readonly value: Int8Array;
}

export interface NbtIntArray {
  readonly tag: "intArray";
  readonly value: Int32Array;
}

export interface NbtLongArray {
  readonly tag: "longArray";
  readonly value: BigInt64Array;
}

export interface NbtList {
  readonly tag: "list";
  /** `0` when the list is empty: NBT writes `TAG_End` as its element id. */
  readonly elementTag: NbtTagId | 0;
  readonly value: readonly NbtTag[];
}

export interface NbtCompound {
  readonly tag: "compound";
  readonly value: ReadonlyMap<string, NbtTag>;
}

export type NbtTag =
  | NbtNumber
  | NbtLong
  | NbtString
  | NbtByteArray
  | NbtIntArray
  | NbtLongArray
  | NbtList
  | NbtCompound;

const tagEnd = 0;
const tagCompound = 10;
const tagIntArray = 11;
const tagLongArray = 12;

/** `TAG_List` element ids are `TAG_End` when the list is empty. */
const tagByte = 1;

const truncated = "This .nbt file is truncated.";
const unknownTag = "This .nbt file uses an unknown NBT tag.";
const negativeLength = "This .nbt file declares a negative NBT length.";
const limitsExceeded = "This .nbt file exceeds the preview's NBT limits.";

/**
 * Reads the big-endian NBT dialect Java edition writes. Bedrock's
 * little-endian file format is `.mcstructure`, which never reaches this module.
 *
 * Every read is bounds-checked against the input rather than trusted, because a
 * declared length in the file is attacker-controlled and this parser runs on
 * files chosen by whoever is previewing them.
 */
class NbtReader {
  private readonly decoder = new TextDecoder();
  private readonly view: DataView;
  private nodes = 0;
  private offset = 0;

  constructor(bytes: Uint8Array) {
    this.view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  }

  readRoot(): NbtCompound {
    if (this.readByte() !== tagCompound) {
      throw new SchematicFormatError("This .nbt file is not an NBT compound.");
    }

    this.readName();
    const root = this.readPayload(tagCompound, 1);
    if (root.tag !== "compound") {
      throw new SchematicFormatError("This .nbt file is not an NBT compound.");
    }

    // Bytes past the root compound are ignored; gzip padding is legal.
    return root;
  }

  /** Reserves `count` bytes and returns the offset they start at. */
  private advance(count: number): number {
    const start = this.offset;
    if (count < 0 || start + count > this.view.byteLength) {
      throw new SchematicFormatError(truncated);
    }

    this.offset = start + count;
    return start;
  }

  private readByte(): number {
    return this.view.getUint8(this.advance(1));
  }

  private readInt16(): number {
    return this.view.getInt16(this.advance(2));
  }

  private readInt32(): number {
    return this.view.getInt32(this.advance(4));
  }

  /** NBT declares every field length as a signed 32-bit value. */
  private readCount(): number {
    const count = this.readInt32();
    if (count < 0) {
      throw new SchematicFormatError(negativeLength);
    }
    if (count > maximumNbtCollectionItems) {
      throw new SchematicFormatError(limitsExceeded);
    }

    return count;
  }

  /**
   * NBT declares string lengths as unsigned 16-bit values, so the widest a
   * single string can be is 65,535 bytes — short of `maximumNbtStringBytes`,
   * which bounds the same quantity for the file as a whole.
   */
  private readName(): string {
    const length = this.view.getUint16(this.advance(2));
    const start = this.advance(length);
    return this.decoder.decode(
      new Uint8Array(this.view.buffer, this.view.byteOffset + start, length),
    );
  }

  private readPayload(tagId: NbtTagId, depth: number): NbtTag {
    this.nodes += 1;
    if (depth > maximumNbtDepth || this.nodes > maximumNbtNodes) {
      throw new SchematicFormatError(limitsExceeded);
    }

    switch (tagId) {
      case 1:
        return { tag: "byte", value: this.view.getInt8(this.advance(1)) };
      case 2:
        return { tag: "short", value: this.readInt16() };
      case 3:
        return { tag: "int", value: this.readInt32() };
      case 4:
        return { tag: "long", value: this.view.getBigInt64(this.advance(8)) };
      case 5:
        return { tag: "float", value: this.view.getFloat32(this.advance(4)) };
      case 6:
        return { tag: "double", value: this.view.getFloat64(this.advance(8)) };
      case 7:
        return { tag: "byteArray", value: this.readByteArray() };
      case 8:
        return { tag: "string", value: this.readName() };
      case 9:
        return this.readList(depth);
      case 10:
        return this.readCompound(depth);
      case tagIntArray:
        return { tag: "intArray", value: this.readIntArray() };
      default:
        return { tag: "longArray", value: this.readLongArray() };
    }
  }

  /**
   * Refuses a declared element count that the remaining bytes cannot hold, so a
   * hostile header cannot drive an allocation larger than the input itself.
   */
  private readArrayCount(width: number): number {
    const count = this.readCount();
    if (count * width > this.view.byteLength - this.offset) {
      throw new SchematicFormatError(truncated);
    }

    return count;
  }

  /**
   * Byte arrays are viewed rather than copied: `Int8Array` has no alignment
   * requirement, so the input's own bytes can back it directly.
   */
  private readByteArray(): Int8Array {
    const count = this.readArrayCount(1);
    return new Int8Array(this.view.buffer, this.view.byteOffset + this.advance(count), count);
  }

  private readIntArray(): Int32Array {
    const count = this.readArrayCount(4);
    const values = new Int32Array(count);
    for (let index = 0; index < count; index += 1) {
      values[index] = this.readInt32();
    }

    return values;
  }

  private readLongArray(): BigInt64Array {
    const count = this.readArrayCount(8);
    const values = new BigInt64Array(count);
    for (let index = 0; index < count; index += 1) {
      values[index] = this.view.getBigInt64(this.advance(8));
    }

    return values;
  }

  private readList(depth: number): NbtList {
    const elementTag = this.readByte();
    const count = this.readCount();
    if (elementTag === tagEnd) {
      if (count !== 0) {
        throw new SchematicFormatError(unknownTag);
      }

      return { tag: "list", elementTag: tagEnd, value: [] };
    }
    if (elementTag < tagByte || elementTag > tagLongArray) {
      throw new SchematicFormatError(unknownTag);
    }

    const tag = elementTag as NbtTagId;
    const elements: NbtTag[] = [];
    // List elements are bare payloads: no tag id and no name.
    for (let index = 0; index < count; index += 1) {
      elements.push(this.readPayload(tag, depth + 1));
    }

    return { tag: "list", elementTag: tag, value: elements };
  }

  private readCompound(depth: number): NbtCompound {
    const value = new Map<string, NbtTag>();
    for (;;) {
      const tagId = this.readByte();
      if (tagId === tagEnd) {
        return { tag: "compound", value };
      }
      if (tagId < tagByte || tagId > tagLongArray) {
        throw new SchematicFormatError(unknownTag);
      }

      value.set(this.readName(), this.readPayload(tagId as NbtTagId, depth + 1));
    }
  }
}

/** Parses a complete big-endian NBT document and returns its root compound. */
export function readNbtCompound(bytes: Uint8Array): NbtCompound {
  return new NbtReader(bytes).readRoot();
}
