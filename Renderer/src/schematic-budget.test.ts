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

import { gzip } from "pako";
import { describe, expect, it } from "vite-plus/test";
import {
  assertParsedSchematicWithinBudget,
  inspectCompressedLitematic,
  maximumAxisLength,
  maximumRenderedBlocks,
  maximumSchematicVolume,
  measureInflatedSize,
  SchematicBudgetError,
} from "./schematic-budget";

describe("compressed schematic budgets", () => {
  it("inspects bounded Litematica region data", () => {
    const inspection = inspectCompressedLitematic(
      compressedLitematic([{ position: [0, 0, 0], size: [2, 2, 2] }]),
    );

    expect(inspection.regionCount).toBe(1);
    expect(inspection.totalVolume).toBe(2 * 2 * 2);
    expect(inspection.inflatedBytes).toBeGreaterThan(0);
  });

  it("accepts inflated output at the configured boundary", () => {
    const compressed = gzip(new Uint8Array(1_024));

    expect(measureInflatedSize(compressed, 1_024)).toBe(1_024);
  });

  it("rejects inflated output above the configured boundary", () => {
    const compressed = gzip(new Uint8Array(1_025));

    expect(() => measureInflatedSize(compressed, 1_024)).toThrow(SchematicBudgetError);
  });

  it("rejects a malformed gzip stream", () => {
    expect(() => measureInflatedSize(new Uint8Array([0x1f, 0x8b]))).toThrow(SchematicBudgetError);
  });

  it("accepts a negatively oriented region within the bounding budget", () => {
    const inspection = inspectCompressedLitematic(
      compressedLitematic([{ position: [10, 0, 0], size: [-2, 1, 1] }]),
    );

    expect(inspection.totalVolume).toBe(2);
  });

  it("accepts empty entity lists whose element type is TAG_End", () => {
    const inspection = inspectCompressedLitematic(
      compressedLitematic([{ position: [0, 0, 0], size: [1, 1, 1] }], { emptyEntityLists: true }),
    );

    expect(inspection.regionCount).toBe(1);
  });

  it("rejects small regions whose combined position span is enormous", () => {
    const compressed = compressedLitematic([
      { position: [-2_147_483_648, 0, 0], size: [1, 1, 1] },
      { position: [2_147_483_647, 0, 0], size: [1, 1, 1] },
    ]);

    expect(() => inspectCompressedLitematic(compressed)).toThrow(SchematicBudgetError);
  });
});

type RegionDefinition = {
  position: [number, number, number];
  size: [number, number, number];
};

type LitematicOptions = {
  emptyEntityLists?: boolean;
};

function compressedLitematic(
  regions: RegionDefinition[],
  options: LitematicOptions = {},
): Uint8Array {
  const bytes: number[] = [10, 0, 0];
  pushTagHeader(bytes, 10, "Regions");
  for (const [index, region] of regions.entries()) {
    pushTagHeader(bytes, 10, `region-${index}`);
    pushVector(bytes, "Position", region.position);
    pushVector(bytes, "Size", region.size);

    if (options.emptyEntityLists) {
      pushEmptyEndList(bytes, "Entities");
      pushEmptyEndList(bytes, "TileEntities");
    }

    pushTagHeader(bytes, 9, "BlockStatePalette");
    bytes.push(10);
    pushInt(bytes, 1);
    bytes.push(0);

    const volume = region.size.reduce((product, size) => product * Math.abs(size), 1);
    const blockStateLongs = Math.ceil((volume * 2) / 64);
    pushTagHeader(bytes, 12, "BlockStates");
    pushInt(bytes, blockStateLongs);
    for (let byte = 0; byte < blockStateLongs * 8; byte += 1) {
      bytes.push(0);
    }
    bytes.push(0);
  }
  bytes.push(0, 0);
  return gzip(Uint8Array.from(bytes));
}

function pushEmptyEndList(bytes: number[], name: string): void {
  pushTagHeader(bytes, 9, name);
  bytes.push(0);
  pushInt(bytes, 0);
}

function pushVector(bytes: number[], name: string, values: [number, number, number]): void {
  pushTagHeader(bytes, 10, name);
  for (const [axis, value] of [
    ["x", values[0]],
    ["y", values[1]],
    ["z", values[2]],
  ] as const) {
    pushTagHeader(bytes, 3, axis);
    pushInt(bytes, value);
  }
  bytes.push(0);
}

function pushTagHeader(bytes: number[], type: number, name: string): void {
  bytes.push(type);
  const encoded = new TextEncoder().encode(name);
  bytes.push((encoded.byteLength >> 8) & 0xff, encoded.byteLength & 0xff, ...encoded);
}

function pushInt(bytes: number[], value: number): void {
  bytes.push((value >> 24) & 0xff, (value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff);
}

describe("parsed schematic budgets", () => {
  it("accepts dimensions and counts at their limits", () => {
    expect(() =>
      assertParsedSchematicWithinBudget({
        blockCount: maximumRenderedBlocks,
        dimensions: [maximumAxisLength, 1, maximumSchematicVolume / maximumAxisLength],
        volume: maximumSchematicVolume,
      }),
    ).not.toThrow();
  });

  it("rejects an oversized parsed dimension", () => {
    expect(() =>
      assertParsedSchematicWithinBudget({
        blockCount: 1,
        dimensions: [maximumAxisLength + 1, 1, 1],
        volume: 1,
      }),
    ).toThrow(SchematicBudgetError);
  });

  it("rejects an oversized parsed block count", () => {
    expect(() =>
      assertParsedSchematicWithinBudget({
        blockCount: maximumRenderedBlocks + 1,
        dimensions: [1, 1, 1],
        volume: 1,
      }),
    ).toThrow(SchematicBudgetError);
  });
});
