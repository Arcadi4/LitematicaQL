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
import { decodeBase64, formatDimensions } from "./bridge";

describe("decodeBase64", () => {
  it("recreates arbitrary bytes", () => {
    const bytes = new Uint8Array([0, 31, 139, 127, 128, 255]);
    const encoded = "AB+Lf4D/";

    expect(new Uint8Array(decodeBase64(encoded))).toEqual(bytes);
  });
});

describe("formatDimensions", () => {
  it("normalizes typed arrays", () => {
    expect(formatDimensions(new Int32Array([12, 7, 19]))).toEqual([12, 7, 19]);
  });

  it("rejects incomplete dimensions", () => {
    expect(formatDimensions([12, 7])).toBeUndefined();
  });
});
