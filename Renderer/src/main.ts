import { postNativeMessage } from "./bridge";
import { SchematicViewer } from "./viewer";
import { StatusModel, formatBytes, type ProgressInfo } from "./status";
import type { BatchProgress } from "./stream";
import "./style.css";

interface LoadMetaInfo {
  name: string;
  dimensions: string;
  blockCount: number;
  blockEntityCount: number;
  warnings?: string[];
  large: boolean;
}

interface MeshStartInfo {
  name: string;
  atlasUrl: string;
  atlasWidth: number;
  atlasHeight: number;
  batchCount: number;
}

declare global {
  interface Window {
    litematicaQL: {
      loadMeta(info: LoadMetaInfo): Promise<void>;
      progress(info: ProgressInfo): Promise<void>;
      meshStart(info: MeshStartInfo): Promise<void>;
      loadError(message: string): Promise<void>;
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
const fileMemory = requiredElement<HTMLElement>("file-memory");
const fileWarnings = requiredElement<HTMLElement>("file-warnings");
const controlsHint = requiredElement<HTMLElement>("controls-hint");

const viewer = new SchematicViewer(canvas);
let activeGeneration = 0;
let streamAbort: AbortController | undefined;

const statusModel = new StatusModel();

function renderStatus(): void {
  const view = statusModel.current;
  status.hidden = view.kind === "hidden";
  status.classList.toggle("status--error", view.kind === "error");
  if (view.kind === "progress") {
    statusTitle.textContent = "Building preview";
    statusDetail.textContent = view.notice;
    statusDetail.hidden = view.notice === "";
  } else if (view.kind === "error" || view.kind === "idle") {
    statusTitle.textContent = view.title;
    statusDetail.textContent = view.message;
    statusDetail.hidden = false;
  }
}

const largeRenderStatusDetail = "This schematic is large — rendering it will take a while.";

window.addEventListener("resize", () => viewer.resize());

statusModel.idle("Ready", "Waiting for a schematic file…");
renderStatus();
postNativeMessage({ type: "ready", detail: "" });

window.litematicaQL = {
  async loadMeta(info: LoadMetaInfo): Promise<void> {
    activeGeneration += 1;
    streamAbort?.abort();
    streamAbort = undefined;
    viewer.cancelStream();
    viewer.clearContent();
    showPreviewMetadata(
      info.name,
      info.dimensions,
      info.blockCount,
      info.blockEntityCount,
      info.warnings ?? [],
    );
    statusModel.loadStarted(info.large ? largeRenderStatusDetail : "");
    renderStatus();
  },

  async progress(info: ProgressInfo): Promise<void> {
    statusModel.loadProgress(info.completed, info.total);
    renderStatus();
  },

  async meshStart(info: MeshStartInfo): Promise<void> {
    const generation = activeGeneration;
    streamAbort?.abort();
    const abort = new AbortController();
    streamAbort = abort;
    statusModel.loadProgress(0, info.batchCount + 1);
    renderStatus();
    try {
      const result = await viewer.loadStream({
        atlasURL: info.atlasUrl,
        atlasWidth: info.atlasWidth,
        atlasHeight: info.atlasHeight,
        batchCount: info.batchCount,
        signal: abort.signal,
        fetchBatch: (index) =>
          fetch(`lql-mesh://preview/batch/${index}`, { signal: abort.signal, cache: "no-store" }),
        onProgress: (progress: BatchProgress) => {
          if (generation !== activeGeneration || abort.signal.aborted) return;
          statusModel.loadProgress(progress.completed, progress.total);
          renderStatus();
        },
      });
      if (generation !== activeGeneration || abort.signal.aborted) {
        viewer.cancelStream();
        return;
      }
      if (result.triangles === 0) {
        throw new Error("The schematic contains no visible geometry to render.");
      }
      const memory = (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory;
      fileMemory.textContent = memory
        ? `Renderer memory ${formatBytes(memory.usedJSHeapSize)}`
        : "";
      fileMemory.hidden = !memory;
      statusModel.loadSettled();
      renderStatus();
      postNativeMessage({ type: "loaded", detail: info.name });
    } catch (error) {
      if (error instanceof DOMException && error.name === "AbortError") return;
      showLoadError(error instanceof Error ? error.message : "The preview could not be uploaded.");
    }
  },

  // Quick Look discards rejected previews, so the page must render native errors.
  async loadError(message: string): Promise<void> {
    showLoadError(message);
  },
};

function showPreviewMetadata(
  name: string,
  dimensions: string,
  blockCount: number,
  blockEntityCount: number,
  warnings: string[],
): void {
  fileName.textContent = name;
  fileDimensions.textContent = dimensions;
  fileBlockCount.textContent = `${blockCount.toLocaleString()} blocks`;
  fileBlockEntities.textContent = `${blockEntityCount.toLocaleString()} block entities`;
  fileMemory.hidden = true;
  fileInfo.hidden = false;
  controlsHint.hidden = false;
  fileWarnings.replaceChildren(
    ...warnings.map((warning) => {
      const item = document.createElement("li");
      item.textContent = warning;
      return item;
    }),
  );
  fileWarnings.hidden = warnings.length === 0;
}

function showLoadError(message: string): void {
  statusModel.loadFailed("Preview unavailable", message);
  renderStatus();
  fileInfo.hidden = true;
  controlsHint.hidden = true;
}

function requiredElement<ElementType extends HTMLElement>(id: string): ElementType {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`Missing renderer element: ${id}`);
  }
  return element as ElementType;
}
