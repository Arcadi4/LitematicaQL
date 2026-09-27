import { describe, expect, it } from "vite-plus/test";
import { StatusModel } from "./status";

describe("status overlay lifecycle", () => {
  it("shows a plain fraction while a load is running", () => {
    const model = new StatusModel();
    model.loadStarted("");
    model.loadProgress(71, 193);

    const view = model.current;
    expect(view.kind).toBe("progress");
    expect(view.kind === "progress" && view.fraction).toBeCloseTo(71 / 193);
  });

  it("hides the overlay once the load settles", () => {
    const model = new StatusModel();
    model.loadStarted("");
    model.loadProgress(193, 193);
    model.loadSettled();

    expect(model.current.kind).toBe("hidden");
  });

  it("ignores progress that lands after the load settled", () => {
    const model = new StatusModel();
    model.loadStarted("");
    model.loadSettled();

    // The native side posts progress over fire-and-forget calls, so the final
    // batch report routinely arrives after the page has finished rendering.
    model.loadProgress(193, 193);

    expect(model.current.kind).toBe("hidden");
  });

  it("ignores progress that arrives before any load", () => {
    const model = new StatusModel();
    model.loadProgress(1, 193);

    expect(model.current.kind).toBe("hidden");
  });

  it("accepts progress again for the next load", () => {
    const model = new StatusModel();
    model.loadStarted("");
    model.loadSettled();
    model.loadStarted("");

    model.loadProgress(2, 193);

    const view = model.current;
    expect(view.kind).toBe("progress");
    expect(view.kind === "progress" && view.fraction).toBeCloseTo(2 / 193);
  });

  it("keeps a refusal visible over late progress", () => {
    const model = new StatusModel();
    model.loadStarted("");
    model.loadFailed("Preview unavailable", "This file is not a readable Minecraft schematic.");

    model.loadProgress(193, 193);

    const view = model.current;
    expect(view.kind).toBe("error");
    expect(view.kind === "error" && view.message).toBe(
      "This file is not a readable Minecraft schematic.",
    );
  });

  it("carries the large-render notice with the bar", () => {
    const model = new StatusModel();
    model.loadStarted("This schematic is large — rendering it will take a while.");

    const view = model.current;
    expect(view.kind === "progress" && view.notice).toBe(
      "This schematic is large — rendering it will take a while.",
    );
    expect(view.kind === "progress" && view.fraction).toBeUndefined();
  });
});
