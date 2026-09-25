import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";
import {
  AmbientLight,
  Box3,
  Color,
  DirectionalLight,
  GridHelper,
  type LineBasicMaterial,
  type Object3D,
  PerspectiveCamera,
  Scene,
  SRGBColorSpace,
  Vector3,
  WebGLRenderer,
} from "three";
import {
  StreamUploader,
  type StreamUploaderOptions,
  type StreamUploadResult,
  uploadStream,
} from "./stream";

const backgroundColor = 0x0b1016;
const gridMajorColor = 0x2a3440;
const gridMinorColor = 0x1a2229;

const voxelIsometricElevation = Math.atan(1 / Math.SQRT2);

const previewFramePadding = 1.18;

const fieldOfViewDegrees = 28;
const maximumPixelRatio = 1.5;
const gridExtentMultiplier = 1.15;
const minimumGridExtent = 16;
const worldUp = new Vector3(0, 1, 0);

/** Unit vector from the origin toward the initial isometric camera. */
const isometricCameraDirection = new Vector3(
  Math.cos(voxelIsometricElevation) * Math.sin(Math.PI / 4),
  Math.sin(voxelIsometricElevation),
  Math.cos(voxelIsometricElevation) * Math.cos(Math.PI / 4),
).normalize();

/**
 * Camera distance that fits the whole bounding box in the narrower viewport
 * axis. Fitting the box rather than its bounding sphere keeps long, flat
 * schematics from being pushed into the distance, since a sphere wrapped around
 * a wide strip is far larger than the strip a viewer actually sees.
 */
function fittingDistance(box: Box3, aspect: number, padding: number): number {
  const tanVertical = Math.tan((fieldOfViewDegrees * Math.PI) / 360) / padding;
  const tanHorizontal = tanVertical * aspect;
  const forward = isometricCameraDirection.clone().negate();
  const right = new Vector3().crossVectors(forward, worldUp).normalize();
  const up = new Vector3().crossVectors(right, forward).normalize();
  const centre = box.getCenter(new Vector3());

  let distance = 0;
  for (const corner of boxCorners(box)) {
    const offset = corner.sub(centre);
    const depth = offset.dot(forward);
    distance = Math.max(
      distance,
      Math.abs(offset.dot(up)) / tanVertical - depth,
      Math.abs(offset.dot(right)) / tanHorizontal - depth,
    );
  }

  return distance;
}

function boxCorners(box: Box3): Vector3[] {
  const corners: Vector3[] = [];
  for (const x of [box.min.x, box.max.x]) {
    for (const y of [box.min.y, box.max.y]) {
      for (const z of [box.min.z, box.max.z]) {
        corners.push(new Vector3(x, y, z));
      }
    }
  }

  return corners;
}

/**
 * Quick Look suspends `requestAnimationFrame` callbacks while a preview is
 * offscreen, so each path that changes the image calls {@link render} directly.
 */
export class SchematicViewer {
  private readonly camera: PerspectiveCamera;
  private readonly canvas: HTMLCanvasElement;
  private readonly controls: OrbitControls;
  private readonly renderer: WebGLRenderer;
  private readonly scene: Scene;
  private stagedUploader: StreamUploader | undefined;
  private contentUploader: StreamUploader | undefined;
  private content: Object3D | undefined;
  private grid: GridHelper | undefined;
  private framedBounds: Box3 | undefined;
  private fittedDistance = 0;
  constructor(canvas: HTMLCanvasElement) {
    this.canvas = canvas;
    this.renderer = new WebGLRenderer({ antialias: true, canvas, preserveDrawingBuffer: true });
    this.renderer.setPixelRatio(Math.min(window.devicePixelRatio, maximumPixelRatio));
    this.renderer.outputColorSpace = SRGBColorSpace;
    this.renderer.setClearColor(backgroundColor, 1);

    this.scene = new Scene();
    this.scene.background = new Color(backgroundColor);
    this.scene.add(new AmbientLight(0xffffff, 1.5));

    const sun = new DirectionalLight(0xffffff, 1.4);
    sun.position.set(1, 1.4, 0.8);
    this.scene.add(sun);

    this.camera = new PerspectiveCamera(fieldOfViewDegrees, 1, 0.1, 1_000);
    this.controls = new OrbitControls(this.camera, canvas);
    this.controls.enableDamping = false;
    this.controls.addEventListener("change", () => this.render());

    this.resize();
  }

  async loadStream(options: Omit<StreamUploaderOptions, "uploader" | "onStaged">): Promise<StreamUploadResult> {
    this.releaseStaged();
    const uploader = new StreamUploader(options.decoder);
    this.stagedUploader = uploader;
    this.scene.add(uploader.root);
    try {
      const result = await uploadStream({
        ...options,
        uploader,
        onStaged: () => this.render(),
      });
      this.releaseContent();
      this.content = result.root;
      this.frameContent();
      this.stagedUploader = undefined;
      this.contentUploader = uploader;
      return result;
    } catch (error) {
      this.releaseStaged();
      this.content = undefined;
      throw error;
    }
  }

  cancelStream(): void {
    this.releaseStaged();
  }

  clearContent(): void {
    this.releaseContent();
  }

  render(): void {
    this.renderer.render(this.scene, this.camera);
  }

  /**
   * Matches the drawing buffer to the canvas layout box and redraws.
   *
   * Nothing draws on a timer, so a resize must repaint explicitly: `setSize`
   * reallocates and clears the drawing buffer, which would otherwise leave a
   * blank panel until the viewer happened to interact. The aspect change also
   * alters how much of the model fits, so the camera is refitted to match.
   */
  resize(): void {
    const width = this.canvas.clientWidth;
    const height = this.canvas.clientHeight;
    if (width === 0 || height === 0) {
      return;
    }

    this.renderer.setSize(width, height, false);
    this.camera.aspect = width / height;
    this.camera.updateProjectionMatrix();
    this.refitCamera();
    this.render();
  }

  dispose(): void {
    this.releaseStaged();
    this.releaseContent();
    this.controls.dispose();
    this.renderer.dispose();
  }

  private frameContent(): void {
    if (!this.content) {
      return;
    }

    const bounds = new Box3().setFromObject(this.content);
    const offset = bounds.getCenter(new Vector3()).negate();
    const distance = fittingDistance(bounds, this.camera.aspect, previewFramePadding);
    if (!Number.isFinite(distance) || distance <= 0) {
      throw new Error("The rendered schematic has no visible extent.");
    }

    // Camera, orbit target, and grid share the origin-centered model frame.
    this.content.position.add(offset);
    bounds.translate(offset);
    this.framedBounds = bounds;
    this.fittedDistance = distance;
    this.rebuildGrid(bounds);

    this.camera.position.copy(isometricCameraDirection).multiplyScalar(distance);
    this.applyClipPlanes(distance);
    this.camera.updateProjectionMatrix();

    this.controls.minDistance = distance * 0.05;
    this.controls.maxDistance = distance * 8;
    this.controls.target.set(0, 0, 0);
    this.controls.update();
    this.render();
  }

  /**
   * Re-frames the model after the viewport aspect ratio changes, preserving how
   * far the viewer has zoomed relative to the fitted distance. Without this a
   * narrower panel would crop a model that the previous aspect fitted exactly.
   */
  private refitCamera(): void {
    if (!this.framedBounds || this.fittedDistance <= 0) {
      return;
    }

    const fitted = fittingDistance(this.framedBounds, this.camera.aspect, previewFramePadding);
    if (!Number.isFinite(fitted) || fitted <= 0) {
      return;
    }

    const currentDistance = this.camera.position.distanceTo(this.controls.target);
    const zoomRatio = currentDistance / this.fittedDistance;
    this.fittedDistance = fitted;

    const direction = this.camera.position.clone().sub(this.controls.target);
    if (direction.lengthSq() === 0) {
      direction.copy(isometricCameraDirection);
    }
    direction.normalize();

    const distance = fitted * zoomRatio;
    this.camera.position.copy(this.controls.target).addScaledVector(direction, distance);
    this.applyClipPlanes(fitted);
    this.controls.minDistance = fitted * 0.05;
    this.controls.maxDistance = fitted * 8;
  }

  /**
   * Sets the depth range from the fitted distance rather than the current one,
   * so the planes stay valid no matter how far the viewer has zoomed.
   */
  private applyClipPlanes(fittedDistance: number): void {
    this.camera.near = Math.max(fittedDistance / 1_000, 0.1);
    this.camera.far = fittedDistance * 12;
  }

  /**
   * Nucleation models use one world unit per block. The centered model's
   * horizontal footprint therefore determines the grid extent.
   */
  private rebuildGrid(bounds: Box3): void {
    this.releaseGrid();

    const size = bounds.getSize(new Vector3());
    const extent = Math.max(Math.max(size.x, size.z) * gridExtentMultiplier, minimumGridExtent);
    const grid = new GridHelper(
      extent,
      Math.max(Math.round(extent), 4),
      gridMajorColor,
      gridMinorColor,
    );
    grid.position.y = bounds.min.y;
    grid.material.opacity = 0.5;
    grid.material.transparent = true;
    this.grid = grid;
    this.scene.add(grid);
  }

  private releaseStaged(): void {
    const staged = this.stagedUploader;
    if (!staged) return;
    this.stagedUploader = undefined;
    this.scene.remove(staged.root);
    staged.dispose();
  }

  private releaseContent(): void {
    this.releaseGrid();
    const uploader = this.contentUploader;
    if (!uploader) {
      this.content = undefined;
      return;
    }
    this.framedBounds = undefined;
    this.fittedDistance = 0;
    this.scene.remove(uploader.root);
    uploader.dispose();
    this.contentUploader = undefined;
    this.content = undefined;
  }

  private releaseGrid(): void {
    if (!this.grid) {
      return;
    }

    this.scene.remove(this.grid);
    this.grid.geometry.dispose();
    (this.grid.material as LineBasicMaterial).dispose();
    this.grid = undefined;
  }
}
