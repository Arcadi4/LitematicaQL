// Quick Look can suspend requestAnimationFrame while preparing a hidden preview.
// The incremental pipeline yields with timers and does not wait for animation frames.
export const quickLookSafeMeshBuildingMode = "incremental" as const;
