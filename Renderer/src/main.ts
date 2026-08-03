// LitematicaQL: macOS Quick Look plugin for Litematica schematics.
// Copyright (C) 2026 4rcadia
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
//
// See the LICENSE file for the full license text.

import { SchematicRenderer, SchematicWrapper } from "schematic-renderer";
import { decodeBase64, formatDimensions, postNativeMessage } from "./bridge";
import { presentRendererFrame } from "./render-presentation";
import { loadBundledResourcePack } from "./resource-pack";
import { assertParsedSchematicWithinBudget, inspectCompressedLitematic } from "./schematic-budget";
import "./style.css";

declare global {
  interface Window {
    litematicaQL: {
      loadSchematic(name: string, encodedData: string): Promise<void>;
    };
  }
}

const canvas = requiredElement<HTMLCanvasElement>("schematic-canvas");
const status = requiredElement<HTMLElement>("status");
const statusTitle = requiredElement<HTMLElement>("status-title");
const statusDetail = requiredElement<HTMLElement>("status-detail");
const fileInfo = requiredElement<HTMLElement>("file-info");
const fileName = requiredElement<HTMLElement>("file-name");
const fileDimensions = requiredElement<HTMLElement>("file-dimensions");
const controlsHint = requiredElement<HTMLElement>("controls-hint");
const rendererInitializationTimeoutMilliseconds = 20_000;

let resolveRenderer!: (renderer: SchematicRenderer) => void;
let rejectRenderer!: (error: Error) => void;
const rendererReady = new Promise<SchematicRenderer>((resolve, reject) => {
  resolveRenderer = resolve;
  rejectRenderer = reject;
});
void rendererReady.catch(() => undefined);

let rendererInitializationSettled = false;
const rendererInitializationTimeout = window.setTimeout(() => {
  failRendererInitialization(
    new Error("The schematic renderer did not initialize within 20 seconds."),
  );
}, rendererInitializationTimeoutMilliseconds);

try {
  new SchematicRenderer(
    canvas,
    {},
    { vanilla: loadBundledResourcePack },
    {
      backgroundColor: 0x0b1016,
      cameraOptions: {
        defaultCameraPreset: "isometric",
        enableZoomInOnLoad: false,
        useTightBounds: true,
      },
      debugOptions: {
        showUnknownBlocks: true,
      },
      enableAdaptiveFPS: true,
      enableAnimatedTextures: false,
      enableAutoOrbit: false,
      enableDragAndDrop: false,
      enableGizmos: false,
      enableInteraction: true,
      enableProgressBar: false,
      maxPixelRatio: 1.5,
      meshBuildingMode: "batched",
      postProcessingOptions: {
        enabled: false,
      },
      resourcePackOptions: {
        showMissingPackNotice: false,
      },
      showAxes: false,
      showGrid: true,
      sidebarOptions: {
        enabled: false,
      },
      singleSchematicMode: true,
      targetFPS: 60,
      wasmMeshBuilderOptions: {
        enabled: true,
        greedyMeshingEnabled: false,
        maxWorkers: 2,
      },
      callbacks: {
        onRendererInitialized: (renderer) => {
          if (rendererInitializationSettled) {
            renderer.dispose();
            return;
          }

          rendererInitializationSettled = true;
          window.clearTimeout(rendererInitializationTimeout);
          resolveRenderer(renderer);
          setStatus("Ready", "Waiting for a .litematic file…");
          postNativeMessage({ type: "ready" });
        },
      },
    },
  );
} catch (error) {
  failRendererInitialization(normalizeError(error));
}

let latestLoadRequest = 0;
let loadQueue = Promise.resolve();

window.litematicaQL = {
  loadSchematic(name: string, encodedData: string): Promise<void> {
    const request = ++latestLoadRequest;
    setStatus("Opening schematic", "Reading compressed block data…");
    postNativeMessage({ type: "loading", detail: name });

    const load = loadQueue.then(() => renderSchematic(request, name, encodedData));
    loadQueue = load.catch(() => undefined);
    return load;
  },
};

async function renderSchematic(request: number, name: string, encodedData: string): Promise<void> {
  if (request !== latestLoadRequest) {
    return;
  }

  let parsedSchematic: SchematicWrapper | undefined;
  let schematicTransferred = false;
  try {
    const renderer = await rendererReady;
    if (request !== latestLoadRequest) {
      return;
    }

    const manager = renderer.schematicManager;
    if (!manager) {
      throw new Error("The schematic renderer did not finish initializing.");
    }

    const data = decodeBase64(encodedData);
    inspectCompressedLitematic(new Uint8Array(data));
    parsedSchematic = new SchematicWrapper();
    parsedSchematic.from_litematic(new Uint8Array(data));
    assertParsedSchematicWithinBudget({
      blockCount: parsedSchematic.get_block_count(),
      dimensions: parsedSchematic.get_dimensions(),
      volume: parsedSchematic.get_volume(),
    });
    if (request !== latestLoadRequest) {
      return;
    }

    await manager.removeAllSchematics();
    if (request !== latestLoadRequest) {
      return;
    }

    await manager.loadSchematic(name, parsedSchematic, undefined, {
      onProgress: ({ message }) => {
        if (request === latestLoadRequest) {
          setStatus("Building preview", message);
        }
      },
    });
    schematicTransferred = true;

    const schematic = manager.getSchematic(name);
    if (!schematic) {
      throw new Error("The schematic renderer did not retain the loaded schematic.");
    }

    if (request === latestLoadRequest) {
      setStatus("Building preview", "Preparing block geometry…");
    }
    await schematic.getMeshes();

    if (request !== latestLoadRequest) {
      return;
    }

    await renderer.cameraManager.focusOnSchematics({
      animationDuration: 0,
      padding: 1.18,
      useTightBounds: true,
    });

    await presentRendererFrame(renderer);
    if (request !== latestLoadRequest) {
      return;
    }

    const dimensions = formatDimensions(renderer.getSchematicDimensions(name));
    showPreviewMetadata(name, dimensions);
    postNativeMessage({ type: "loaded", detail: name });
  } catch (error) {
    const normalized = normalizeError(error);
    if (request === latestLoadRequest) {
      showLoadError(normalized.message);
    }
    throw normalized;
  } finally {
    if (parsedSchematic && !schematicTransferred) {
      parsedSchematic.free();
    }
  }
}

function failRendererInitialization(error: Error): void {
  if (rendererInitializationSettled) {
    return;
  }

  rendererInitializationSettled = true;
  window.clearTimeout(rendererInitializationTimeout);
  rejectRenderer(error);
  showFatalError(error.message);
}

function showPreviewMetadata(name: string, dimensions: [number, number, number] | undefined): void {
  fileName.textContent = name;
  fileDimensions.textContent = dimensions
    ? `${dimensions[0]} × ${dimensions[1]} × ${dimensions[2]} blocks`
    : "Litematica schematic";
  fileInfo.hidden = false;
  controlsHint.hidden = false;
  status.hidden = true;
}

function showLoadError(message: string): void {
  status.hidden = false;
  status.classList.add("status--error");
  statusTitle.textContent = "Preview unavailable";
  statusDetail.textContent = message;
  fileInfo.hidden = true;
  controlsHint.hidden = true;
}

function showFatalError(message: string): void {
  showLoadError(message);
  postNativeMessage({ type: "fatalError", detail: message });
}

function setStatus(title: string, detail: string): void {
  status.hidden = false;
  status.classList.remove("status--error");
  statusTitle.textContent = title;
  statusDetail.textContent = detail;
}

function requiredElement<ElementType extends HTMLElement>(id: string): ElementType {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`Missing renderer element: ${id}`);
  }
  return element as ElementType;
}

function normalizeError(error: unknown): Error {
  return error instanceof Error ? error : new Error(String(error));
}
