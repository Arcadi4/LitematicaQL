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

import { readFileSync } from "node:fs";
import type { Schematic } from "nucleation";
import { describe, expect, it } from "vite-plus/test";
import { decodeSchematic, prepareSchematicBytes } from "./schematic-formats";
import { SchematicFormatError } from "./schematic-limits";
import { convertStructureNbt, normalizeStructureSnbt } from "./structure-snbt";

function fixture(name: string): Uint8Array {
  return new Uint8Array(readFileSync(new URL(`../../Fixtures/Formats/${name}`, import.meta.url)));
}

const textDecoder = new TextDecoder("utf-8", { fatal: true });

/** The converted text, as Nucleation's structure-SNBT reader sees it. */
function convertedText(): string {
  return textDecoder.decode(convertStructureNbt(fixture("Structure.nbt")));
}

function decodeStructure(): Schematic {
  return decodeSchematic("structure", fixture("Structure.nbt"));
}

describe("structure NBT conversion", () => {
  it("emits text the structure-SNBT reader imports", () => {
    const schematic = decodeStructure();

    expect(schematic.getBlockString(0, 0, 0)).toBe("minecraft:oak_log[axis=y]");
    expect(schematic.getBlockString(2, 0, 0)).toBe("minecraft:chest");
  });

  it("drops entries whose palette name is air", () => {
    // The fixture places a third block on palette index 2, which is
    // minecraft:air, so the two real blocks are all that may survive.
    expect(decodeStructure().blockCount()).toBe(2);
  });

  it("carries the structure's declared size and data version", () => {
    const dimensions = decodeStructure().dimensions();

    expect([dimensions.x, dimensions.y, dimensions.z]).toEqual([4, 2, 2]);
    expect(decodeStructure().sourceDataVersion()).toBe(3953);
  });

  it("preserves the block entity payload and the entity", () => {
    const schematic = decodeStructure();
    const blockEntity = JSON.parse(schematic.getBlockEntityJson(2, 0, 0)) as {
      id: string;
      nbt: Record<string, unknown>;
    };

    expect(blockEntity.id).toBe("minecraft:chest");
    expect(blockEntity.nbt.CustomName).toEqual({ String: '{"text":"Loot"}' });

    const entities = JSON.parse(schematic.getEntitiesJson()) as {
      id: string;
      position: number[];
    }[];
    expect(entities).toHaveLength(1);
    expect(entities[0]!.id).toBe("minecraft:armor_stand");
    expect(entities[0]!.position).toEqual([0.5, 1, 0.5]);
  });

  it("writes palette properties in the bracket form", () => {
    expect(convertedText()).toContain('"minecraft:oak_log[axis=y]"');
    expect(convertedText()).not.toContain("{axis=y}");
  });

  it("refuses input that carries no usable size", () => {
    // A well-formed big-endian compound with no structure keys at all.
    expect(() => convertStructureNbt(new Uint8Array([0x0a, 0, 0, 0]))).toThrow(
      SchematicFormatError,
    );
  });

  it("refuses a structure beyond the reader's own axis and volume bounds", () => {
    expect(() => convertStructureNbt(sizeOnlyStructure([128, 128, 128]))).toThrow(
      /262,144 cells and 256 blocks per axis/u,
    );
  });

  it("refuses gzip data declaring more than the preview limit", () => {
    // Nothing but a gzip header whose trailer claims 96 MiB, which is past the
    // 64 MiB ceiling. The declared size is checked before any inflation runs.
    expect(() => convertStructureNbt(gzipDeclaring(96 * 1024 * 1024))).toThrow(
      /64 MiB preview limit/u,
    );
  });

  it("returns every non-structure input by reference", () => {
    const bytes = fixture("Classic.schematic");

    expect(prepareSchematicBytes("classic", bytes)).toBe(bytes);
    expect(prepareSchematicBytes("litematic", bytes)).toBe(bytes);
    expect(prepareSchematicBytes("snbt", bytes)).toBe(bytes);
  });
});

describe("structure SNBT normalization", () => {
  const normalize = (source: string) =>
    textDecoder.decode(normalizeStructureSnbt(new TextEncoder().encode(source)));

  it("rewrites brace properties with either separator to the bracket form", () => {
    const source =
      '{palette:["minecraft:oak_log{axis=y}"],data:[{pos:[0,0,0],state:"minecraft:oak_log{axis=y}"}],entities:[]}';
    const normalized = normalize(source);

    expect(normalized).toContain('state:"minecraft:oak_log[axis=y]"');
    // The palette list is only length-checked by the reader, so the rewrite
    // deliberately leaves it alone rather than guessing at its contents.
    expect(normalized).toContain('"minecraft:oak_log{axis=y}"');
  });

  it("rewrites the colon separator Nucleation's own writer emits", () => {
    const source =
      '{data:[{pos:[0,0,0],state:"minecraft:repeater{delay:2,facing:east}"}],entities:[]}';

    expect(normalize(source)).toContain('state:"minecraft:repeater[delay=2,facing=east]"');
  });

  it("leaves an empty property group alone", () => {
    const source = '{state:"minecraft:oak_log{}"}';

    expect(normalize(source)).toBe(source);
  });

  it("leaves a state string outside a block-state value alone", () => {
    const source = '{nbt:{description:"state:\\"minecraft:oak_log{axis=y}\\""}}';

    expect(normalize(source)).toBe(source);
  });

  it("returns the same reference when nothing matched", () => {
    const bytes = new TextEncoder().encode("{DataVersion:3953}");

    expect(normalizeStructureSnbt(bytes)).toBe(bytes);
  });

  it("returns the same reference for bytes that are not UTF-8", () => {
    const bytes = new Uint8Array([0xff, 0xfe, 0xfd]);

    expect(normalizeStructureSnbt(bytes)).toBe(bytes);
  });
});

/**
 * A big-endian compound carrying only `size`, which is all the converter has to
 * read before it decides whether the structure is within reach.
 */
function sizeOnlyStructure(size: [number, number, number]): Uint8Array {
  const bytes = [
    0x0a,
    0,
    0, // root compound
    11,
    0,
    4,
    ...utf8("size"), // TAG_Int_Array named "size"
    ...bigEndian(3),
    ...size.flatMap((axis) => bigEndian(axis)),
    0, // end of root compound
  ];

  return new Uint8Array(bytes);
}

/** A gzip stream whose trailer declares `declared` output bytes. */
function gzipDeclaring(declared: number): Uint8Array {
  const bytes = new Uint8Array(8);
  bytes.set([0x1f, 0x8b, 0x08, 0x00]);
  new DataView(bytes.buffer).setUint32(4, declared, true);
  return bytes;
}

function utf8(value: string): number[] {
  return [...new TextEncoder().encode(value)];
}

function bigEndian(value: number): number[] {
  return [(value >>> 24) & 0xff, (value >>> 16) & 0xff, (value >>> 8) & 0xff, value & 0xff];
}
