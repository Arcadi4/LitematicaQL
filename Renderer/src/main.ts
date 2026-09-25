import { postNativeMessage } from "./bridge";
import { SchematicViewer } from "./viewer";
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

interface ProgressInfo {
  phase: "decode" | "mesh" | "upload";
  completed: number;
  total: number;
  bytes: number;
  triangles: number;
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
const fileModelStats = requiredElement<HTMLElement>("file-model-stats");
const fileMemory = requiredElement<HTMLElement>("file-memory");
const fileWarnings = requiredElement<HTMLElement>("file-warnings");
const controlsHint = requiredElement<HTMLElement>("controls-hint");

const viewer = new SchematicViewer(canvas);
let activeGeneration = 0;
let streamAbort: AbortController | undefined;

const largeRenderStatusTitle = "Building preview";
const largeRenderStatusDetail = "This schematic is large — rendering it will take a while.";

window.addEventListener("resize", () => viewer.resize());

setStatus("Ready", "Waiting for a schematic file…");
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
    setStatus(
      largeRenderStatusTitle,
      info.large ? largeRenderStatusDetail : "Preparing block geometry…",
    );
  },

  async progress(info: ProgressInfo): Promise<void> {
    if (info.phase === "decode") {
      setStatus("Reading schematic", `${formatBytes(info.bytes)} · ${info.completed}/${info.total}`);
    } else if (info.phase === "mesh") {
      setStatus(
        "Building geometry",
        `${info.completed}/${info.total} batches · ${Math.round((info.completed / Math.max(1, info.total)) * 100)}% · ${formatBytes(info.bytes)} · ${info.triangles.toLocaleString()} triangles`,
      );
    } else {
      setStatus(
        "Uploading preview",
        `${Math.max(0, info.completed - 1)}/${Math.max(0, info.total - 1)} batches · ${formatBytes(info.bytes)} uploaded`,
      );
    }
  },

  async meshStart(info: MeshStartInfo): Promise<void> {
    const generation = activeGeneration;
    streamAbort?.abort();
    const abort = new AbortController();
    streamAbort = abort;
    setStatus("Uploading shared atlas", `0/${info.batchCount + 1} · preparing textures`);
    try {
      const result = await viewer.loadStream({
        atlasURL: info.atlasUrl,
        atlasWidth: info.atlasWidth,
        atlasHeight: info.atlasHeight,
        batchCount: info.batchCount,
        signal: abort.signal,
        fetchBatch: (index) => fetch(`lql-mesh://preview/batch/${index}`, { signal: abort.signal, cache: "no-store" }),
        onProgress: (progress: BatchProgress) => {
          if (generation !== activeGeneration || abort.signal.aborted) return;
          if (progress.phase === "atlas") {
            setStatus("Uploading shared atlas", `${formatBytes(progress.bytes)} · texture atlas uploaded once`);
          } else {
            setStatus(
              "Uploading geometry",
              `${progress.completed - 1}/${info.batchCount} batches · ${Math.round((progress.completed / progress.total) * 100)}% · ${formatBytes(progress.bytes)} transferred`,
            );
          }
        },
      });
      if (generation !== activeGeneration || abort.signal.aborted) {
        viewer.cancelStream();
        return;
      }
      if (result.triangles === 0) {
        throw new Error("The schematic contains no visible geometry to render.");
      }
      fileModelStats.textContent = `${result.triangles.toLocaleString()} triangles · ${formatBytes(result.bytes)} total model data`;
      const memory = (performance as Performance & { memory?: { usedJSHeapSize: number } }).memory;
      fileMemory.textContent = memory ? `Renderer memory ${formatBytes(memory.usedJSHeapSize)}` : "";
      fileMemory.hidden = !memory;
      status.hidden = true;
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
  fileModelStats.textContent = "";
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
  status.hidden = false;
  status.classList.add("status--error");
  statusTitle.textContent = "Preview unavailable";
  statusDetail.textContent = message;
  fileInfo.hidden = true;
  controlsHint.hidden = true;
}

function setStatus(title: string, detail: string): void {
  status.hidden = false;
  status.classList.remove("status--error");
  statusTitle.textContent = title;
  statusDetail.textContent = detail;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function requiredElement<ElementType extends HTMLElement>(id: string): ElementType {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`Missing renderer element: ${id}`);
  }
  return element as ElementType;
}
