import { afterEach, describe, expect, it } from "vite-plus/test";
import { postNativeMessage, type NativeBridgeMessage } from "./bridge";

afterEach(() => {
  (globalThis as unknown as { webkit?: unknown }).webkit = undefined;
});

describe("postNativeMessage", () => {
  it("routes a message to the native handler", () => {
    const messages: NativeBridgeMessage[] = [];
    (globalThis as unknown as { webkit?: unknown }).webkit = {
      messageHandlers: {
        litematicaQL: {
          postMessage: (message: NativeBridgeMessage) => {
            messages.push(message);
          },
        },
      },
    };

    postNativeMessage({ type: "loaded", detail: "Cottage.litematic" });

    expect(messages).toEqual([{ type: "loaded", detail: "Cottage.litematic" }]);
  });
});
