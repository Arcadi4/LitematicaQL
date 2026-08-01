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
