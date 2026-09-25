import { postNativeMessage } from "./bridge";
import { SchematicViewer } from "./viewer";
import "./style.css";

// Native decoding and meshing supply metadata and a GLB; this module owns
// presentation, camera state, and failure panels.

interface LoadMetaInfo {
  name: string;
  // Display-ready content dimensions like `13 × 9 × 11`.
  dimensions: string;
  blockCount: number;
  blockEntityCount: number;
  // Reader notices about content that exists but is not shown.
  warnings?: string[];
  large: boolean;
}

interface MeshReadyInfo {
  name: string;
  triangles: number;
  url: string;
}

declare global {
  interface Window {
    litematicaQL: {
      loadMeta(info: LoadMetaInfo): Promise<void>;
      meshReady(info: MeshReadyInfo): Promise<void>;
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
const fileWarnings = requiredElement<HTMLElement>("file-warnings");
const controlsHint = requiredElement<HTMLElement>("controls-hint");

const viewer = new SchematicViewer(canvas);

const largeRenderStatusTitle = "Building preview";
const largeRenderStatusDetail = "This schematic is large — rendering it will take a while.";

window.addEventListener("resize", () => {
  viewer.resize();
});

setStatus("Ready", "Waiting for a schematic file…");
postNativeMessage({ type: "ready", detail: "" });

window.litematicaQL = {
  async loadMeta(info: LoadMetaInfo): Promise<void> {
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

  async meshReady(info: MeshReadyInfo): Promise<void> {
    if (!info.url) {
      throw new Error("The preview did not provide a mesh location.");
    }
    const response = await fetch(info.url);
    if (!response.ok) {
      throw new Error(`The mesh could not be read (HTTP ${response.status}).`);
    }
    await viewer.loadGlb(await response.arrayBuffer());
    status.hidden = true;
    postNativeMessage({ type: "loaded", detail: info.name });
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

function requiredElement<ElementType extends HTMLElement>(id: string): ElementType {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`Missing renderer element: ${id}`);
  }
  return element as ElementType;
}
