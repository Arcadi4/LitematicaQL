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

import { MeshConfig, MeshResult, ResourcePack, type Schematic } from "nucleation";
import { decodeBase64, postNativeMessage } from "./bridge";
import { loadBundledResourcePack } from "./resource-pack";
import { parseSchematicWithinBudget } from "./schematic-limits";
import { SchematicViewer } from "./viewer";
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
  loadSchematic(name: string, encodedData: string): Promise<void> {
    const request = ++latestLoadRequest;
    setStatus("Opening schematic", "Reading compressed block data…");
    postNativeMessage({ type: "loading", detail: name });

    const load = loadQueue.then(() => renderSchematic(request, name, encodedData));
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
      setStatus("Ready", "Waiting for a .litematic file…");
      postNativeMessage({ type: "ready" });
    }

    return pack;
  } catch (error) {
    const normalized = normalizeError(error);
    failInitialization(normalized);
    throw normalized;
  }
}

async function renderSchematic(request: number, name: string, encodedData: string): Promise<void> {
  try {
    const pack = await resourcePackReady;
    if (request !== latestLoadRequest) {
      return;
    }

    const schematic = parseSchematicWithinBudget(decodeBase64(encodedData));
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

    showPreviewMetadata(name, preview.dimensions, blockCount, blockEntityCount);
    postNativeMessage({ type: "loaded", detail: name });
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
 * Thrown when Nucleation cannot build geometry for a schematic that passed the
 * block budget.
 *
 * Block count is a poor predictor of mesh size: 8.4 million blocks of solid
 * stone mesh into half a million triangles, while a 6.5 million block
 * checkerboard exhausts the mesher. Nucleation reports that exhaustion as a
 * WebAssembly trap rather than an error value, so the only reliable handling is
 * to treat any meshing failure as "too detailed to preview".
 */
class SchematicComplexityError extends Error {
  override name = "SchematicComplexityError";

  constructor(options?: ErrorOptions) {
    super(
      "This schematic has too much visible surface to preview. Its block geometry exceeds what the renderer can build.",
      options,
    );
  }
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
    throw new SchematicComplexityError({ cause: error });
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
