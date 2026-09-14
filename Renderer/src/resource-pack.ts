// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import { decodeBase64, nativeResourcePackHandler, type NativeReplyHandler } from "./bridge";

const resourcePackRequest = { type: "resourcePack" } as const;

/**
 * Reads the bundled Minecraft resource pack through the native bridge. The pack
 * ships beside the renderer inside the app and extension bundles; Quick Look
 * blocks network access and cannot `fetch` sibling files, so the bytes always
 * arrive through this bridge.
 */
export async function loadBundledResourcePack(
  handler: NativeReplyHandler | undefined = nativeResourcePackHandler(),
): Promise<Uint8Array> {
  if (!handler) {
    throw new Error("The native resource-pack bridge is unavailable.");
  }

  const encodedData = await handler.postMessage(resourcePackRequest);
  if (typeof encodedData !== "string") {
    throw new Error("The bundled block resources could not be read.");
  }

  return decodeBase64(encodedData);
}
