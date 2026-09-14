// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import { describe, expect, it } from "vite-plus/test";
import {
  decodingLimits,
  diagnosticLimits,
  maximumNbtCollectionItems,
  maximumNbtNodes,
  maximumRenderedBlocks,
  maximumSchematicVolume,
} from "./schematic-limits";

/**
 * Widening `decodingLimits` decides whether an oversized schematic is reported
 * as oversized or as unreadable, so a wrapped or mistyped allowance silently
 * changes which files the preview accepts. JavaScript bitwise shifts wrap at 32
 * bits, which makes `1 << 40` evaluate to 256; the relationship between the two
 * limit sets is what catches that class of mistake.
 */
describe("decode limits", () => {
  it("declares every allowance as a usable positive integer", () => {
    for (const [name, value] of Object.entries(decodingLimits)) {
      expect(Number.isSafeInteger(value), `${name} must be a safe integer`).toBe(true);
      expect(value, `${name} must be positive`).toBeGreaterThan(0);
    }
  });

  it("widens every diagnostic allowance rather than narrowing it", () => {
    for (const [name, value] of Object.entries(diagnosticLimits)) {
      const preview = decodingLimits[name as keyof typeof decodingLimits];
      expect(value, `${name} must not fall below its preview allowance`).toBeGreaterThanOrEqual(
        preview,
      );
    }
  });

  it("keeps diagnostic allowances bounded so they cannot drive a large allocation", () => {
    for (const value of Object.values(diagnosticLimits)) {
      expect(value).toBeLessThanOrEqual(1_073_741_824);
    }
  });

  it("keeps the render budget inside the allowed volume", () => {
    expect(maximumRenderedBlocks).toBeLessThanOrEqual(maximumSchematicVolume);
  });

  /**
   * Both NBT allowances were below what ordinary files need before this, and
   * they are the difference between previewing a Sponge or Bedrock schematic
   * and reporting it as unreadable. These are measured floors from real files,
   * not round numbers: a 565 KB Sponge schematic declaring a 256³ region holds
   * 1,070,312 tags and 17,093,491 collection entries.
   */
  it("keeps the NBT allowances above what real files measure", () => {
    expect(maximumNbtCollectionItems).toBeGreaterThan(17_093_491);
    expect(maximumNbtNodes).toBeGreaterThan(1_070_312);
  });

  it("leaves the NBT allowances inside the exact-integer range", () => {
    expect(Number.isSafeInteger(maximumNbtCollectionItems)).toBe(true);
    expect(Number.isSafeInteger(maximumNbtNodes)).toBe(true);
  });
});
