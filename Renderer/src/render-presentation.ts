// LitematicaQL: macOS Quick Look plugin for Litematica schematics.
// Copyright (C) 2026 4rcadia
// SPDX-License-Identifier: AGPL-3.0-or-later

interface RenderPresentationTarget {
  invalidate(): void;
  renderManager?: {
    render(): void;
  };
}

type RequestFrame = (callback: FrameRequestCallback) => number;

export function presentRendererFrame(
  renderer: RenderPresentationTarget,
  requestFrame: RequestFrame = (callback) => window.requestAnimationFrame(callback),
): Promise<void> {
  if (!renderer.renderManager) {
    throw new Error("The schematic renderer did not finish initializing its render manager.");
  }

  renderer.invalidate();
  renderer.renderManager.render();

  return new Promise((resolve) => {
    // The second callback runs after the browser has had a chance to composite the rendered canvas.
    requestFrame(() => {
      requestFrame(() => resolve());
    });
  });
}
