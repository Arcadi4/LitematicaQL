// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

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

export function decodeBase64(encoded: string): Uint8Array {
  const binary = atob(encoded);
  const bytes = new Uint8Array(binary.length);

  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }

  return bytes;
}

export function postNativeMessage(message: NativeBridgeMessage): void {
  window.webkit?.messageHandlers?.litematicaQL?.postMessage(message);
}

export function nativeResourcePackHandler(): NativeReplyHandler | undefined {
  return typeof window === "undefined"
    ? undefined
    : window.webkit?.messageHandlers?.litematicaQLResourcePack;
}
