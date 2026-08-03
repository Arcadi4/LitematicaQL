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

import { decodeBase64, type NativeReplyHandler } from "./bridge";

const resourcePackRequest = { type: "resourcePack" } as const;

function nativeResourcePackHandler(): NativeReplyHandler | undefined {
  return typeof window === "undefined"
    ? undefined
    : window.webkit?.messageHandlers?.litematicaQLResourcePack;
}

export async function loadBundledResourcePack(
  handler: NativeReplyHandler | undefined = nativeResourcePackHandler(),
): Promise<Blob> {
  if (!handler) {
    throw new Error("The native resource-pack bridge is unavailable.");
  }

  const encodedData = await handler.postMessage(resourcePackRequest);
  if (typeof encodedData !== "string") {
    throw new Error("The bundled block resources could not be read.");
  }

  return new Blob([decodeBase64(encodedData)], { type: "application/zip" });
}
