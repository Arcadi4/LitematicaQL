export interface ProgressInfo {
  phase: "decode" | "mesh" | "upload";
  completed: number;
  total: number;
  bytes: number;
  triangles: number;
}

/**
 * What the status overlay shows. Progress states are a plain bar; numbers and
 * phase chatter are noise over a render that speaks for itself.
 */
export type StatusView =
  | { kind: "hidden" }
  | { kind: "idle"; title: string; message: string }
  | { kind: "progress"; notice: string; fraction: number | undefined }
  | { kind: "error"; title: string; message: string };

/**
 * Owns the status overlay's lifecycle. Native progress arrives over
 * fire-and-forget calls that routinely land after the page has already
 * finished or refused a load, and re-showing the overlay then pins a dead
 * message over a rendered preview — so updates belonging to a settled load
 * are dropped.
 */
export class StatusModel {
  private state: StatusView = { kind: "hidden" };

  get current(): StatusView {
    return this.state;
  }

  idle(title: string, message: string): void {
    this.state = { kind: "idle", title, message };
  }

  loadStarted(notice: string): void {
    this.state = { kind: "progress", notice, fraction: undefined };
  }

  loadProgress(completed: number, total: number): void {
    if (this.state.kind !== "progress") {
      return;
    }
    this.state = {
      ...this.state,
      fraction: total > 0 ? Math.min(1, Math.max(0, completed / total)) : undefined,
    };
  }

  /** The load rendered; the overlay closes and late progress is ignored. */
  loadSettled(): void {
    this.state = { kind: "hidden" };
  }

  /** The load was refused; the message stays until the next load starts. */
  loadFailed(title: string, message: string): void {
    this.state = { kind: "error", title, message };
  }
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
