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

import { describe, expect, it } from "vite-plus/test";
import { readNbtCompound, type NbtCompound, type NbtTag } from "./nbt";
import {
  maximumNbtCollectionItems,
  maximumNbtDepth,
  SchematicFormatError,
} from "./schematic-limits";

/**
 * A big-endian NBT encoder whose tag ids are written out literally.
 *
 * That is deliberate: an encoder that derived its ids from the reader's table
 * would round-trip just as happily against a table shifted by one, which is
 * exactly the mistake this suite exists to catch. The ids below come from the
 * Java edition spec, so the two disagree the moment either is wrong.
 */
class Encoder {
  private readonly chunks: Uint8Array[] = [];
  private readonly scratch = new ArrayBuffer(8);

  encode(root: NbtTag): Uint8Array {
    // 10 is TAG_Compound, and the root always carries a name (empty here).
    this.chunks.push(new Uint8Array([10, 0, 0]));
    this.writePayload(root);
    return concat(this.chunks);
  }

  private writeNumber(write: (view: DataView) => void, width: number): void {
    const view = new DataView(this.scratch);
    write(view);
    this.chunks.push(new Uint8Array(this.scratch.slice(0, width)));
  }

  private writeString(value: string): void {
    const bytes = new TextEncoder().encode(value);
    this.writeNumber((view) => view.setUint16(0, bytes.length), 2);
    this.chunks.push(bytes);
  }

  private writePayload(tag: NbtTag): void {
    switch (tag.tag) {
      case "byte":
        return this.writeNumber((view) => view.setInt8(0, tag.value), 1);
      case "short":
        return this.writeNumber((view) => view.setInt16(0, tag.value), 2);
      case "int":
        return this.writeNumber((view) => view.setInt32(0, tag.value), 4);
      case "long":
        return this.writeNumber((view) => view.setBigInt64(0, tag.value), 8);
      case "float":
        return this.writeNumber((view) => view.setFloat32(0, tag.value), 4);
      case "double":
        return this.writeNumber((view) => view.setFloat64(0, tag.value), 8);
      case "byteArray":
        this.writeNumber((view) => view.setInt32(0, tag.value.length), 4);
        this.chunks.push(new Uint8Array(tag.value.buffer.slice(0, tag.value.length)));
        return;
      case "intArray":
        this.writeNumber((view) => view.setInt32(0, tag.value.length), 4);
        for (const element of tag.value) {
          this.writeNumber((view) => view.setInt32(0, element), 4);
        }
        return;
      case "longArray":
        this.writeNumber((view) => view.setInt32(0, tag.value.length), 4);
        for (const element of tag.value) {
          this.writeNumber((view) => view.setBigInt64(0, element), 8);
        }
        return;
      case "string":
        return this.writeString(tag.value);
      case "list": {
        // An empty list declares TAG_End as its element id.
        this.chunks.push(new Uint8Array([tag.value.length === 0 ? 0 : tag.elementTag]));
        this.writeNumber((view) => view.setInt32(0, tag.value.length), 4);
        for (const element of tag.value) {
          this.writePayload(element);
        }
        return;
      }
      default: {
        for (const [key, value] of tag.value) {
          this.chunks.push(new Uint8Array([tagId(value)]));
          this.writeString(key);
          this.writePayload(value);
        }
        this.chunks.push(new Uint8Array([0]));
        return;
      }
    }
  }
}

/** NBT tag ids, per the Java edition spec. */
const ids: Record<NbtTag["tag"], number> = {
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
};

function tagId(tag: NbtTag): number {
  return ids[tag.tag];
}

function concat(chunks: Uint8Array[]): Uint8Array {
  const length = chunks.reduce((total, chunk) => total + chunk.length, 0);
  const out = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.length;
  }

  return out;
}

function compound(entries: [string, NbtTag][]): NbtTag {
  return { tag: "compound", value: new Map(entries) };
}

function encode(root: NbtTag): Uint8Array {
  return new Encoder().encode(root);
}

function entry(root: NbtCompound, key: string): NbtTag {
  const tag = root.value.get(key);
  if (tag === undefined) {
    throw new Error(`Missing NBT entry: ${key}`);
  }

  return tag;
}

describe("NBT reader", () => {
  it("reads every tag type at its declared width", () => {
    const root = readNbtCompound(
      encode(
        compound([
          ["aByte", { tag: "byte", value: -3 }],
          ["aShort", { tag: "short", value: -300 }],
          ["anInt", { tag: "int", value: -70_000 }],
          ["aLong", { tag: "long", value: -9_000_000_000n }],
          ["aFloat", { tag: "float", value: 1.5 }],
          ["aDouble", { tag: "double", value: 2.5 }],
          ["aByteArray", { tag: "byteArray", value: new Int8Array([1, -1]) }],
          ["aString", { tag: "string", value: "hello" }],
          ["anIntArray", { tag: "intArray", value: new Int32Array([1, -2, 3]) }],
          ["aLongArray", { tag: "longArray", value: new BigInt64Array([-5n, 6n]) }],
        ]),
      ),
    );

    expect(entry(root, "aByte")).toEqual({ tag: "byte", value: -3 });
    expect(entry(root, "aShort")).toEqual({ tag: "short", value: -300 });
    expect(entry(root, "anInt")).toEqual({ tag: "int", value: -70_000 });
    expect(entry(root, "aLong")).toEqual({ tag: "long", value: -9_000_000_000n });
    expect(entry(root, "aFloat")).toEqual({ tag: "float", value: 1.5 });
    expect(entry(root, "aDouble")).toEqual({ tag: "double", value: 2.5 });
    expect(entry(root, "aByteArray")).toEqual({
      tag: "byteArray",
      value: new Int8Array([1, -1]),
    });
    expect(entry(root, "aString")).toEqual({ tag: "string", value: "hello" });
    expect(entry(root, "anIntArray")).toEqual({
      tag: "intArray",
      value: new Int32Array([1, -2, 3]),
    });
    expect(entry(root, "aLongArray")).toEqual({
      tag: "longArray",
      value: new BigInt64Array([-5n, 6n]),
    });
  });

  it("reads nested compounds and lists", () => {
    const root = readNbtCompound(
      encode(
        compound([
          ["outer", compound([["inner", compound([["leaf", { tag: "int", value: 7 }]])]])],
          [
            "entries",
            {
              tag: "list",
              elementTag: 10,
              value: [
                compound([["n", { tag: "int", value: 1 }]]),
                compound([["n", { tag: "int", value: 2 }]]),
              ],
            },
          ],
        ]),
      ),
    );

    const outer = entry(root, "outer") as NbtCompound;
    const inner = entry(outer, "inner") as NbtCompound;
    expect(entry(inner, "leaf")).toEqual({ tag: "int", value: 7 });

    const entries = entry(root, "entries");
    if (entries.tag !== "list") {
      throw new Error("Expected a list");
    }
    expect(entries.elementTag).toBe(10);
    expect(entries.value.map((value) => (value as NbtCompound).value.get("n"))).toEqual([
      { tag: "int", value: 1 },
      { tag: "int", value: 2 },
    ]);
  });

  it("reads an empty list as an element id of zero", () => {
    const root = readNbtCompound(
      encode(compound([["empty", { tag: "list", elementTag: 10, value: [] }]])),
    );

    expect(entry(root, "empty")).toEqual({ tag: "list", elementTag: 0, value: [] });
  });

  /**
   * A compound is untrusted input, so a key named `__proto__` has to land in
   * the map as data instead of reaching an object's prototype.
   */
  it("keeps a __proto__ key as data", () => {
    const root = readNbtCompound(
      encode(compound([["__proto__", { tag: "string", value: "data" }]])),
    );

    expect(root.value.get("__proto__")).toEqual({ tag: "string", value: "data" });
    expect(root.value.size).toBe(1);
  });

  it("ignores bytes after the root compound", () => {
    const bytes = encode(compound([["a", { tag: "int", value: 1 }]]));
    const padded = new Uint8Array(bytes.length + 4);
    padded.set(bytes);
    padded.set([0xde, 0xad, 0xbe, 0xef], bytes.length);

    expect(entry(readNbtCompound(padded), "a")).toEqual({ tag: "int", value: 1 });
  });

  it("rejects a root that is not a compound", () => {
    expect(() => readNbtCompound(new Uint8Array([3, 0, 0, 0, 0, 0, 1]))).toThrow(
      SchematicFormatError,
    );
  });

  it("rejects an unknown tag id", () => {
    // root compound, then a tag id no version of the format defines.
    expect(() =>
      readNbtCompound(new Uint8Array([0x0a, 0, 0, 0x0d, 0, 1, 0x61, 0, 0, 0, 1])),
    ).toThrow(SchematicFormatError);
  });

  it("rejects a truncated payload", () => {
    const bytes = encode(compound([["a", { tag: "long", value: 1n }]]));

    expect(() => readNbtCompound(bytes.slice(0, bytes.length - 4))).toThrow(SchematicFormatError);
  });

  it("rejects a negative collection length", () => {
    // TAG_Byte_Array named "a" declaring a length of -1.
    const bytes = new Uint8Array([0x0a, 0, 0, 7, 0, 1, 0x61, 0xff, 0xff, 0xff, 0xff]);

    expect(() => readNbtCompound(bytes)).toThrow(SchematicFormatError);
  });

  it("rejects a collection longer than the limit", () => {
    const declared = maximumNbtCollectionItems + 1;
    // TAG_List named "a" whose declared element count exceeds the cap, so the
    // reader refuses before it has to read a single element.
    const bytes = new Uint8Array([0x0a, 0, 0, 9, 0, 1, 0x61, 3]);
    const length = new Uint8Array(4);
    new DataView(length.buffer).setUint32(0, declared);

    expect(() => readNbtCompound(concat([bytes, length]))).toThrow(SchematicFormatError);
  });

  it("rejects a compound nested past the depth limit", () => {
    let nested: NbtTag = compound([["leaf", { tag: "int", value: 1 }]]);
    for (let level = 0; level <= maximumNbtDepth; level += 1) {
      nested = compound([[`level${level}`, nested]]);
    }

    expect(() => readNbtCompound(encode(nested))).toThrow(SchematicFormatError);
  });
});
