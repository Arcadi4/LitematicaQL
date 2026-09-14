// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import { MeshConfig, MeshResult, ResourcePack, type Schematic } from "nucleation";
import { decodeBase64, postNativeMessage } from "./bridge";
import { loadBundledResourcePack } from "./resource-pack";
import {
  decodeSchematic,
  displayNameForFileName,
  formatForFileName,
  supportedFileExtensionList,
} from "./schematic-formats";
import { SchematicComplexityError, SchematicFormatError } from "./schematic-limits";
import { SchematicViewer } from "./viewer";
import "./style.css";

declare global {
  interface Window {
    litematicaQL: {
      loadSchematic(fileName: string, encodedData: string): Promise<void>;
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
const fileBlockCount = requiredElement<HTMLElement>("file-block-count");
const fileBlockEntities = requiredElement<HTMLElement>("file-block-entities");
const controlsHint = requiredElement<HTMLElement>("controls-hint");
const rendererInitializationTimeoutMilliseconds = 20_000;

const viewer = new SchematicViewer(canvas);

let initializationSettled = false;
const initializationTimeout = window.setTimeout(() => {
  failInitialization(new Error("The schematic renderer did not initialize within 20 seconds."));
}, rendererInitializationTimeoutMilliseconds);

const resourcePackReady = initializeResourcePack();

window.addEventListener("resize", () => {
  viewer.resize();
});

let latestLoadRequest = 0;
let loadQueue: Promise<void> = Promise.resolve();

window.litematicaQL = {
  loadSchematic(fileName: string, encodedData: string): Promise<void> {
    const request = ++latestLoadRequest;
    const displayName = displayNameForFileName(fileName);
    setStatus("Opening schematic", "Reading compressed block data…");
    postNativeMessage({ type: "loading", detail: displayName });

    const load = loadQueue.then(() => renderSchematic(request, fileName, encodedData));
    loadQueue = load.catch(() => undefined);
    return load;
  },
};

async function initializeResourcePack(): Promise<ResourcePack> {
  try {
    const bytes = await loadBundledResourcePack();
    // Nucleation's declarations type byte inputs as `Array<number>` while its
    // runtime accepts typed arrays; see `schematic-limits.ts`.
    const pack = ResourcePack.fromBytes(bytes as unknown as number[]);
    if (pack.blockstateCount() === 0) {
      throw new Error("The bundled block resources contain no block states.");
    }

    if (!initializationSettled) {
      initializationSettled = true;
      window.clearTimeout(initializationTimeout);
      setStatus("Ready", "Waiting for a schematic file…");
      postNativeMessage({ type: "ready" });
    }

    return pack;
  } catch (error) {
    const normalized = normalizeError(error);
    failInitialization(normalized);
    throw normalized;
  }
}

async function renderSchematic(
  request: number,
  fileName: string,
  encodedData: string,
): Promise<void> {
  try {
    const pack = await resourcePackReady;
    if (request !== latestLoadRequest) {
      return;
    }

    const format = formatForFileName(fileName);
    if (!format) {
      throw new SchematicFormatError(
        `LitematicaQL can preview ${supportedFileExtensionList} files.`,
      );
    }

    // Converting and decoding both block the thread for large files, so the
    // status is on screen before either runs.
    await paintStatus("Opening schematic", "Decoding block data…");
    if (request !== latestLoadRequest) {
      return;
    }

    const schematic = decodeSchematic(format, decodeBase64(encodedData));
    const blockCount = schematic.blockCount();
    const blockEntityCount = countBlockEntities(schematic);

    const preview = await meshSchematic(schematic, pack, request);
    if (!preview) {
      return;
    }

    await viewer.loadGlb(preview.glb);
    if (request !== latestLoadRequest) {
      return;
    }

    const displayName = displayNameForFileName(fileName);
    showPreviewMetadata(displayName, preview.dimensions, blockCount, blockEntityCount);
    postNativeMessage({ type: "loaded", detail: displayName });
  } catch (error) {
    const normalized = normalizeError(error);
    if (request === latestLoadRequest) {
      showLoadError(normalized.message);
    }
    throw normalized;
  }
}

interface MeshedPreview {
  dimensions: [number, number, number];
  glb: ArrayBuffer;
}

/**
 * Meshes the schematic into GLB bytes, or `undefined` once a newer load has
 * superseded this one. Meshing is synchronous inside Nucleation and can run for
 * seconds on large schematics, so the status line is painted and given a turn of
 * the event loop before the thread is blocked.
 */
async function meshSchematic(
  schematic: Schematic,
  pack: ResourcePack,
  request: number,
): Promise<MeshedPreview | undefined> {
  await paintStatus("Building preview", "Preparing block geometry…");
  if (request !== latestLoadRequest) {
    return undefined;
  }

  try {
    const mesh = MeshResult.create(schematic, pack, MeshConfig.create());
    const bounds = mesh.bounds();
    // Litematica regions are padded to whole chunks, so the declared region size
    // overstates the schematic. The geometry bounds describe what is drawn, and
    // one world unit is one block.
    const dimensions: [number, number, number] = [
      Math.round(bounds.maxX - bounds.minX),
      Math.round(bounds.maxY - bounds.minY),
      Math.round(bounds.maxZ - bounds.minZ),
    ];

    await paintStatus("Building preview", "Preparing preview…");
    if (request !== latestLoadRequest) {
      return undefined;
    }

    const glb = decodeBase64(mesh.glbDataB64());
    return { dimensions, glb: glb.buffer as ArrayBuffer };
  } catch (error) {
    // Block count is a poor predictor of mesh size: 8.4 million blocks of solid
    // stone mesh into half a million triangles, while a 6.5 million block
    // checkerboard exhausts the mesher. Nucleation reports that exhaustion as a
    // WebAssembly trap rather than an error value, so the only reliable handling
    // is to treat any meshing failure as "too detailed to preview".
    throw new SchematicComplexityError(
      "This schematic has too much visible surface to preview. Its block geometry exceeds what the renderer can build.",
      { cause: error },
    );
  }
}

/**
 * Updates the status line, then waits for the compositor to present it.
 * Deliberately a timer rather than an animation frame: Quick Look suspends
 * animation callbacks while a preview is offscreen, which would hang this path.
 */
async function paintStatus(title: string, detail: string): Promise<void> {
  setStatus(title, detail);
  await new Promise<void>((resolve) => {
    window.setTimeout(() => resolve(), 0);
  });
}

function countBlockEntities(schematic: Schematic): number {
  const blockEntities: unknown = JSON.parse(schematic.getAllBlockEntitiesJson());
  return Array.isArray(blockEntities) ? blockEntities.length : 0;
}

function failInitialization(error: Error): void {
  if (initializationSettled) {
    return;
  }

  initializationSettled = true;
  window.clearTimeout(initializationTimeout);
  showFatalError(error.message);
}

function showPreviewMetadata(
  name: string,
  dimensions: [number, number, number],
  blockCount: number,
  blockEntityCount: number,
): void {
  fileName.textContent = name;
  fileDimensions.textContent = `${dimensions[0]} × ${dimensions[1]} × ${dimensions[2]}`;
  fileBlockCount.textContent = `${blockCount.toLocaleString()} blocks`;
  fileBlockEntities.textContent = `${blockEntityCount.toLocaleString()} block entities`;
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
