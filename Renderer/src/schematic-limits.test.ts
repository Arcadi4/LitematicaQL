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
import {
  decodingLimits,
  diagnosticLimits,
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
});
