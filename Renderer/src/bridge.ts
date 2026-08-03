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

export interface NativeBridgeMessage {
  type: "ready" | "loading" | "loaded" | "fatalError";
  detail?: string;
}

interface NativeMessageHandler {
  postMessage(message: NativeBridgeMessage): void;
}

export interface NativeReplyHandler {
  postMessage(message: { type: "resourcePack" }): Promise<unknown>;
}

declare global {
  interface Window {
    webkit?: {
      messageHandlers?: {
        litematicaQL?: NativeMessageHandler;
        litematicaQLResourcePack?: NativeReplyHandler;
      };
    };
  }
}

export function decodeBase64(encoded: string): ArrayBuffer {
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);

  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }

  return bytes.buffer;
}

export function postNativeMessage(message: NativeBridgeMessage): void {
  window.webkit?.messageHandlers?.litematicaQL?.postMessage(message);
}

export function formatDimensions(
  dimensions: Int32Array | number[] | null,
): [number, number, number] | undefined {
  if (!dimensions || dimensions.length < 3) {
    return undefined;
  }

  return [dimensions[0] ?? 0, dimensions[1] ?? 0, dimensions[2] ?? 0];
}
