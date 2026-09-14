// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import type { Schematic } from "nucleation";
import { parseSchematicWithinBudget, SchematicFormatError } from "./schematic-limits";
import { convertStructureNbt, normalizeStructureSnbt } from "./structure-snbt";

/** Every file format Nucleation can ingest through `Schematic.fromDataBounded`. */
export type SchematicFormat =
  | "bedrock"
  | "classic"
  | "litematic"
  | "snbt"
  | "snapshot"
  | "sponge"
  | "structure";

export const schematicFormats: Readonly<Record<string, SchematicFormat>> = {
  litematic: "litematic",
  schem: "sponge",
  schematic: "classic",
  nbt: "structure",
  snbt: "snbt",
  mcstructure: "bedrock",
  nusn: "snapshot",
};

export const supportedFileExtensions = Object.keys(schematicFormats);

/** The extension list the preview names when it is handed something else. */
export const supportedFileExtensionList = new Intl.ListFormat("en", {
  type: "conjunction",
  style: "long",
}).format(supportedFileExtensions.map((extension) => `.${extension}`));

/**
 * Reads the extension off a file name. A leading dot names a dotfile rather
 * than an extension, which is also how the native layer's `pathExtension`
 * reads it, so the two agree on what the app will accept.
 */
export function formatForFileName(fileName: string): SchematicFormat | undefined {
  const separator = fileName.lastIndexOf(".");
  return separator <= 0 ? undefined : schematicFormats[fileName.slice(separator + 1).toLowerCase()];
}

/** Drops one extension so the preview titles a file the way a person reads it. */
export function displayNameForFileName(fileName: string): string {
  const separator = fileName.lastIndexOf(".");
  return separator > 0 ? fileName.slice(0, separator) : fileName;
}

/**
 * Turns file bytes into whatever Nucleation actually reads. Only the Java
 * structure file format needs work: Nucleation has no reader for `.nbt`, so it
 * arrives as structure-SNBT text. Every other format is returned by reference
 * so a multi-megabyte buffer is never copied.
 */
export function prepareSchematicBytes(format: SchematicFormat, bytes: Uint8Array): Uint8Array {
  return format === "structure" ? convertStructureNbt(bytes) : bytes;
}

export function decodeSchematic(format: SchematicFormat, bytes: Uint8Array): Schematic {
  try {
    return parseSchematicWithinBudget(prepareSchematicBytes(format, bytes));
  } catch (error) {
    if (format !== "snbt" || !(error instanceof SchematicFormatError)) {
      throw error;
    }

    // Structure SNBT in the wild spells block states with braces, which
    // Nucleation's reader rejects. One normalization retry recovers those.
    const normalized = normalizeStructureSnbt(bytes);
    if (normalized === bytes) {
      throw error;
    }

    return parseSchematicWithinBudget(normalized);
  }
}
