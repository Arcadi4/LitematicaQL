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

interface NativeMessageHost {
  webkit?: {
    messageHandlers?: {
      litematicaQL?: NativeMessageHandler;
    };
  };
}

export function postNativeMessage(message: NativeBridgeMessage): void {
  const handler = (globalThis as unknown as NativeMessageHost).webkit?.messageHandlers
    ?.litematicaQL;
  handler?.postMessage(message);
}
