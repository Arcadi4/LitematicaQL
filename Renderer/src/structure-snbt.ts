// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import { gunzipSync } from "fflate";
import { Schematic } from "nucleation";
import { readNbtCompound, type NbtCompound, type NbtTag } from "./nbt";
import {
  maximumDecompressedBytes,
  maximumPaletteEntries,
  SchematicFormatError,
} from "./schematic-limits";

const gzipMagic = [0x1f, 0x8b];

/**
 * Nucleation imports `.nbt` through its structure-SNBT reader, which refuses
 * anything larger than this. The bound comes from `MAX_STRUCTURE_SNBT_VOLUME`
 * and `MAX_STRUCTURE_SNBT_DIMENSION` in Nucleation's `structure_snbt` module.
 */
const structureVolumeLimit = 262_144;
const structureAxisLimit = 256;

const unreadableStructure = "This .nbt file is not a readable Java structure.";
const asciiKey = /^[A-Za-z0-9_.+-]+$/u;
const exponential = /[eE]/u;

/**
 * Converts a Java structure `.nbt` into the structure-SNBT text Nucleation
 * imports, since Nucleation has no reader for the binary file format.
 *
 * The text dialect is quartz_nbt's, which has two traps worth recording: it
 * rejects exponent notation outright (`1e10d` fails the whole document), and it
 * reads an unrecognised numeric literal such as `NaNf` as a *string*, so
 * non-finite values are emitted as quoted strings to keep that explicit rather
 * than accidental.
 */
export function convertStructureNbt(bytes: Uint8Array): Uint8Array {
  const root = readNbtCompound(inflateIfCompressed(bytes));
  const size = readSize(root);
  const palette = readPalette(root);
  const dataVersion = readInt(root.value.get("DataVersion")) ?? Schematic.canonicalDataVersion();

  const entries = readBlocks(root, palette);
  const entities = readEntities(root);

  return new TextEncoder().encode(
    `{DataVersion:${dataVersion},size:[${size.join(",")}],palette:[${palette
      .map((state) => writeString(state))
      .join(",")}],data:[${entries.join(",")}],entities:[${entities.join(",")}]}`,
  );
}

/**
 * Rewrites Nucleation's brace block-state form into the bracket form its reader
 * accepts: `state: "minecraft:oak_log{axis=y}"` becomes
 * `state: "minecraft:oak_log[axis=y]"`.
 *
 * Real structure SNBT in the wild uses both spellings, and only the bracket
 * form parses, so `.snbt` inputs get one normalization retry after a first
 * import fails. This is a textual rewrite on a document that is already
 * unreadable, restricted to `state` values whose payload looks like a
 * namespaced block name with identifier-only properties.
 *
 * Returns the input unchanged and by reference when nothing matched, which lets
 * the caller skip a pointless retry.
 */
export function normalizeStructureSnbt(bytes: Uint8Array): Uint8Array {
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return bytes;
  }

  const braceState =
    /state:\s*"([\w.-]+:[\w/.-]+)\{([\w.-]+[=:][\w.\-/]+(?:,[\w.-]+[=:][\w.\-/]+)*)\}"/gu;
  const normalized = text.replace(
    braceState,
    (_, name: string, body: string) => `state:"${name}[${body.replace(/([\w.-]+)[=:]/gu, "$1=")}]"`,
  );
  return normalized === text ? bytes : new TextEncoder().encode(normalized);
}

function inflateIfCompressed(bytes: Uint8Array): Uint8Array {
  if (bytes.length < gzipMagic.length || gzipMagic.some((byte, index) => bytes[index] !== byte)) {
    // Some tools write uncompressed structure NBT.
    return bytes;
  }

  if (bytes.length < 8) {
    throw new SchematicFormatError("This .nbt file is truncated.");
  }

  // The gzip trailer's ISIZE is what the inflater sizes its output buffer from,
  // so reading it is both the decompression-bomb guard and a bound on how much
  // memory the decode can cost.
  const declared = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(
    bytes.length - 4,
    true,
  );
  if (declared > maximumDecompressedBytes) {
    throw new SchematicFormatError(
      `This file is not a readable Minecraft schematic, or it expands beyond the ${maximumDecompressedBytes / 1_048_576} MiB preview limit.`,
    );
  }

  const inflated = gunzipSync(bytes);
  if (inflated.length !== declared) {
    throw new SchematicFormatError("This .nbt file is truncated.");
  }

  return inflated;
}

function readInt(tag: NbtTag | undefined): number | undefined {
  return tag?.tag === "int" ? tag.value : undefined;
}

/** `size` is a list of ints, or an int array in files written by some tools. */
function readSize(root: NbtCompound): [number, number, number] {
  const size = readNumbers(root.value.get("size"), 3);
  if (!size || size.some((axis) => !Number.isInteger(axis) || axis <= 0)) {
    throw new SchematicFormatError(
      "This .nbt file is not a Java structure: it has no usable size.",
    );
  }

  const [x, y, z] = size as [number, number, number];
  if (
    x > structureAxisLimit ||
    y > structureAxisLimit ||
    z > structureAxisLimit ||
    x * y * z > structureVolumeLimit
  ) {
    throw new SchematicFormatError(
      `This structure is ${x} × ${y} × ${z}; .nbt previews are limited to ${structureVolumeLimit.toLocaleString()} cells and ${structureAxisLimit} blocks per axis.`,
    );
  }

  return [x, y, z];
}

function readNumbers(tag: NbtTag | undefined, expected: number): number[] | undefined {
  if (tag?.tag === "intArray") {
    return tag.value.length === expected ? Array.from(tag.value) : undefined;
  }
  if (tag?.tag !== "list" || tag.value.length !== expected) {
    return undefined;
  }

  const numbers: number[] = [];
  for (const element of tag.value) {
    switch (element.tag) {
      case "byte":
      case "short":
      case "int":
      case "float":
      case "double":
        numbers.push(element.value);
        break;
      default:
        return undefined;
    }
  }

  return numbers;
}

function readString(tag: NbtTag | undefined): string | undefined {
  return tag?.tag === "string" ? tag.value : undefined;
}

/** Palette entries become `name` or `name[key=value,…]`, keys sorted. */
function readPalette(root: NbtCompound): string[] {
  const palette = root.value.get("palette");
  if (palette?.tag !== "list" || palette.value.length > maximumPaletteEntries) {
    throw new SchematicFormatError(unreadableStructure);
  }

  const states: string[] = [];
  for (const entry of palette.value) {
    if (entry.tag !== "compound") {
      throw new SchematicFormatError(unreadableStructure);
    }

    const name = readString(entry.value.get("Name"));
    if (!name) {
      throw new SchematicFormatError(unreadableStructure);
    }

    const properties = entry.value.get("Properties");
    if (properties === undefined) {
      states.push(name);
      continue;
    }
    if (properties.tag !== "compound") {
      throw new SchematicFormatError(unreadableStructure);
    }

    const pairs: [string, string][] = [];
    for (const [key, value] of properties.value) {
      const property = readString(value);
      if (property === undefined) {
        throw new SchematicFormatError(unreadableStructure);
      }
      pairs.push([key, property]);
    }

    pairs.sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0));
    states.push(
      pairs.length === 0
        ? name
        : `${name}[${pairs.map(([key, value]) => `${key}=${value}`).join(",")}]`,
    );
  }

  return states;
}

/** Palette names that describe absence; their `data` entries are dropped. */
function isAirState(state: string): boolean {
  return state === "minecraft:air";
}

function readBlocks(root: NbtCompound, palette: string[]): string[] {
  const blocks = root.value.get("blocks");
  if (blocks?.tag !== "list") {
    throw new SchematicFormatError(unreadableStructure);
  }

  const entries: string[] = [];
  for (const entry of blocks.value) {
    if (entry.tag !== "compound") {
      throw new SchematicFormatError(unreadableStructure);
    }

    const position = readNumbers(entry.value.get("pos"), 3);
    const index = readInt(entry.value.get("state"));
    if (!position || index === undefined || index < 0 || index >= palette.length) {
      throw new SchematicFormatError(unreadableStructure);
    }

    const state = palette[index]!;
    // Air carries no geometry, and a filled structure would otherwise emit a
    // `data` entry per padded cell.
    if (isAirState(state)) {
      continue;
    }

    const nbt = entry.value.get("nbt");
    if (nbt !== undefined && nbt.tag !== "compound") {
      throw new SchematicFormatError(unreadableStructure);
    }

    const nbtSuffix = nbt ? `,nbt:${writeTag(nbt)}` : "";
    entries.push(`{pos:[${position.join(",")}],state:${writeString(state)}${nbtSuffix}}`);
  }

  return entries;
}

function readEntities(root: NbtCompound): string[] {
  const entities = root.value.get("entities");
  if (entities === undefined) {
    return [];
  }
  if (entities.tag !== "list") {
    throw new SchematicFormatError(unreadableStructure);
  }

  const entries: string[] = [];
  for (const entry of entities.value) {
    if (entry.tag !== "compound") {
      throw new SchematicFormatError(unreadableStructure);
    }

    const position = readNumbers(entry.value.get("pos"), 3);
    const nbt = entry.value.get("nbt");
    if (!position || nbt?.tag !== "compound") {
      continue;
    }

    const blockPos = readNumbers(entry.value.get("blockPos"), 3) ?? position.map(Math.floor);
    entries.push(
      `{pos:[${position.map(decimalLiteral).join(",")}],blockPos:[${blockPos
        .map((axis) => Math.floor(axis))
        .join(",")}],nbt:${writeTag(nbt)}}`,
    );
  }

  return entries;
}

/**
 * Renders a float or double the way quartz_nbt accepts it.
 *
 * The parser rejects exponent notation outright, so `1e-7` has to be expanded,
 * and it reads an unrecognised numeric literal such as `NaNf` as a *string*, so
 * a non-finite value is quoted rather than left to look like a number.
 */
function decimalLiteral(value: number): string {
  if (!Number.isFinite(value)) {
    return `"${value}"`;
  }

  const text = exponential.test(String(value)) ? expand(value) : String(value);
  return text.includes(".") ? text : `${text}.0`;
}

/**
 * Expands the values JavaScript prints in exponent notation. Above 1 the double
 * is integral, so `BigInt` gives its exact digits; below 1 twenty decimals is
 * the most `toFixed` offers, which is past the magnitude of anything a block
 * entity carries.
 */
function expand(value: number): string {
  if (Math.abs(value) >= 1) {
    return BigInt(value).toString();
  }

  return value.toFixed(20).replace(/0+$/u, "").replace(/\.$/u, ".0");
}

function writeString(value: string): string {
  return `"${value.replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
}

function writeKey(key: string): string {
  return asciiKey.test(key) ? key : writeString(key);
}

function writeTag(tag: NbtTag): string {
  switch (tag.tag) {
    case "byte":
      return `${tag.value}b`;
    case "short":
      return `${tag.value}s`;
    case "int":
      return `${tag.value}`;
    case "long":
      return `${tag.value}L`;
    case "float":
      return `${decimalLiteral(tag.value)}f`;
    case "double":
      return `${decimalLiteral(tag.value)}d`;
    case "string":
      return writeString(tag.value);
    case "byteArray":
      return `[B;${Array.from(tag.value, (value) => `${value}b`).join(",")}]`;
    case "intArray":
      return `[I;${Array.from(tag.value).join(",")}]`;
    case "longArray":
      return `[L;${Array.from(tag.value, (value) => `${value}L`).join(",")}]`;
    case "list":
      return `[${tag.value.map(writeTag).join(",")}]`;
    default:
      return `{${Array.from(
        tag.value,
        ([key, value]) => `${writeKey(key)}:${writeTag(value)}`,
      ).join(",")}}`;
  }
}
