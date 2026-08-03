// LitematicaQL: macOS Quick Look plugin for Litematica schematics.
// Copyright (C) 2026 4rcadia
// SPDX-License-Identifier: AGPL-3.0-or-later

interface RenderPresentationTarget {
  invalidate(): void;
  renderManager?: {
    render(): void;
  };
}

export function presentRendererFrame(renderer: RenderPresentationTarget): void {
  if (!renderer.renderManager) {
    throw new Error("The schematic renderer did not finish initializing its render manager.");
  }

  renderer.invalidate();
  renderer.renderManager.render();
}
