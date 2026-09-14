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

import { Schematic } from "nucleation";

/** Compressed `.litematic` bytes accepted from the native bridge. */
export const maximumInputBytes = 64 * 1_024 * 1_024;

/** Inflated NBT bytes Nucleation may allocate while decoding a schematic. */
export const maximumDecompressedBytes = 64 * 1_024 * 1_024;

export const maximumRegionCount = 64;
export const maximumAxisLength = 4_096;
export const maximumSchematicVolume = 16_777_216;
export const maximumPaletteEntries = 4_096;
export const maximumEntityEntries = 100_000;
export const maximumBlockEntityEntries = 100_000;
export const maximumNbtDepth = 64;
export const maximumNbtStringBytes = 1_000_000;

/**
 * A Sponge `BlockData` byte array declares one VarInt per padded cell, so a
 * region that fills `maximumSchematicVolume` entries needs that count widened
 * by the width of a VarInt to stay readable.
 */
export const maximumNbtCollectionItems = maximumSchematicVolume * 2;

/**
 * A 565 KB Sponge schematic declaring a 256³ region holds 1,070,312 tags, so
 * this has to sit well above the volume bound. Nucleation's own default is 64M.
 */
export const maximumNbtNodes = 4_194_304;

/**
 * Nucleation bounds the schematic volume but not the number of non-empty
 * blocks, so the render budget stays a separate check on the parse result.
 */
export const maximumRenderedBlocks = 8_388_608;

/** Limits Nucleation enforces while decompressing and decoding. */
export const decodingLimits = {
  max_block_entities: maximumBlockEntityEntries,
  max_decompressed_bytes: maximumDecompressedBytes,
  max_dimension: maximumAxisLength,
  max_entities: maximumEntityEntries,
  max_input_bytes: maximumInputBytes,
  max_nbt_collection_items: maximumNbtCollectionItems,
  max_nbt_depth: maximumNbtDepth,
  max_nbt_nodes: maximumNbtNodes,
  max_nbt_string_bytes: maximumNbtStringBytes,
  max_palette_entries: maximumPaletteEntries,
  max_regions: maximumRegionCount,
  max_volume: maximumSchematicVolume,
} as const;

/**
 * The same limits with the preview allowances widened, used only to tell an
 * oversized schematic apart from an unreadable one. Nucleation reports both as
 * a bare `NucleationError.Parse`, so a second decode is the only way to recover
 * the distinction.
 *
 * These stay deliberately modest. Lifting them without bound would let a hostile
 * file declare an enormous volume and drive a matching allocation inside the
 * Quick Look extension; `max_decompressed_bytes` still caps the real input.
 */
export const diagnosticLimits = {
  ...decodingLimits,
  max_dimension: 65_536,
  max_palette_entries: 65_536,
  max_regions: 4_096,
  max_volume: 67_108_864,
} as const;

export class SchematicLimitError extends Error {
  override name = "SchematicLimitError";

  constructor(
    message: string,
    readonly blockCount: number,
    readonly dimensions: [number, number, number],
  ) {
    super(message);
  }
}

export class SchematicFormatError extends Error {
  override name = "SchematicFormatError";

  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
  }
}

/**
 * Thrown when Nucleation traps on a schematic instead of reporting an error.
 *
 * A WebAssembly trap poisons the instance for the rest of the page, so a trap
 * must never be followed by a second decode, and the input must not be reported
 * as unreadable.
 */
export class SchematicComplexityError extends Error {
  override name = "SchematicComplexityError";

  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
  }
}

function isWebAssemblyTrap(error: unknown): error is WebAssembly.RuntimeError {
  return error instanceof WebAssembly.RuntimeError;
}

function parseWithinLimits(bytes: Uint8Array, limits: object): Schematic {
  // Nucleation's generated declarations type byte inputs as `Array<number>`,
  // but its Diplomat runtime reads any typed array without copying. Converting
  // would duplicate a multi-megabyte buffer on every load.
  return Schematic.fromDataBounded(bytes as unknown as number[], JSON.stringify(limits));
}

function measuredDimensions(schematic: Schematic): [number, number, number] {
  const dimensions = schematic.dimensions();
  return [dimensions.x, dimensions.y, dimensions.z];
}

/**
 * Decodes a schematic, refusing anything outside the preview budget.
 *
 * Nucleation reports every rejection as the same opaque parse error, so a
 * second decode with the preview allowances lifted decides whether the input
 * was oversized or unreadable and lets the preview explain which. A trap is the
 * one exception: it means the decoder exhausted itself, and it leaves the
 * instance unusable, so it short-circuits both passes.
 */
export function parseSchematicWithinBudget(bytes: Uint8Array): Schematic {
  const schematic = decodeOrExplain(bytes);
  const blockCount = schematic.blockCount();
  if (blockCount > maximumRenderedBlocks) {
    throw oversizedSchematicError(blockCount, measuredDimensions(schematic));
  }

  return schematic;
}

function decodeOrExplain(bytes: Uint8Array): Schematic {
  let previewFailure: unknown;
  try {
    return parseWithinLimits(bytes, decodingLimits);
  } catch (error) {
    if (isWebAssemblyTrap(error)) {
      throw new SchematicComplexityError(
        "This schematic is too large or too detailed to decode in the preview.",
        { cause: error },
      );
    }

    previewFailure = error;
  }

  let diagnostic: Schematic;
  try {
    diagnostic = parseWithinLimits(bytes, diagnosticLimits);
  } catch (error) {
    if (isWebAssemblyTrap(error)) {
      throw new SchematicComplexityError(
        "This schematic is too large or too detailed to decode in the preview.",
        { cause: error },
      );
    }

    throw new SchematicFormatError(
      `This file is not a readable Minecraft schematic, or it expands beyond the ${maximumDecompressedBytes / 1_048_576} MiB preview limit.`,
      { cause: previewFailure },
    );
  }

  throw oversizedSchematicError(diagnostic.blockCount(), measuredDimensions(diagnostic));
}

function oversizedSchematicError(
  blockCount: number,
  dimensions: [number, number, number],
): SchematicLimitError {
  if (blockCount > maximumRenderedBlocks) {
    return new SchematicLimitError(
      `This schematic renders ${blockCount.toLocaleString()} blocks, beyond the ${maximumRenderedBlocks.toLocaleString()}-block preview limit.`,
      blockCount,
      dimensions,
    );
  }

  return new SchematicLimitError(
    `This schematic is ${dimensions[0]} × ${dimensions[1]} × ${dimensions[2]}, beyond the ${maximumAxisLength}-block per-axis and ${maximumSchematicVolume.toLocaleString()}-block preview limits.`,
    blockCount,
    dimensions,
  );
}
