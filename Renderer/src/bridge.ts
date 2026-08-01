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
