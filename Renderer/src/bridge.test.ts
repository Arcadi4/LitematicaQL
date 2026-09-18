import { afterEach, describe, expect, it } from "vite-plus/test";
import { postNativeMessage, type NativeBridgeMessage } from "./bridge";

const nativeHandler = (): {
  messages: NativeBridgeMessage[];
  install: () => void;
  restore: () => void;
} => {
  const messages: NativeBridgeMessage[] = [];
  const host = globalThis as unknown as { webkit?: unknown };
  const original = host.webkit;
  return {
    messages,
    install: () => {
      host.webkit = {
        messageHandlers: {
          litematicaQL: {
            postMessage: (message: NativeBridgeMessage) => {
              messages.push(message);
            },
          },
        },
      };
    },
    restore: () => {
      host.webkit = original;
    },
  };
};

afterEach(() => {
  (globalThis as unknown as { webkit?: unknown }).webkit = undefined;
});

describe("postNativeMessage", () => {
  it("routes a message to the native handler", () => {
    const harness = nativeHandler();
    harness.install();

    postNativeMessage({ type: "loaded", detail: "Cottage.litematic" });

    expect(harness.messages).toEqual([{ type: "loaded", detail: "Cottage.litematic" }]);
  });

  it("does nothing without the native bridge", () => {
    expect(() => postNativeMessage({ type: "loading", detail: "x" })).not.toThrow();
  });
});
