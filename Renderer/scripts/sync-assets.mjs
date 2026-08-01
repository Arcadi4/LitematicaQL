import { copyFile, mkdir } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const require = createRequire(import.meta.url);
const projectRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const rendererEntry = require.resolve("schematic-renderer");
const rendererDist = dirname(rendererEntry);
const rendererRoot = join(rendererDist, "..");

await mkdir(join(projectRoot, "public"), { recursive: true });
await mkdir(join(projectRoot, "third-party"), { recursive: true });

await copyFile(join(rendererDist, "pack.zip"), join(projectRoot, "public", "pack.zip"));
await copyFile(
  join(rendererRoot, "LICENSE"),
  join(projectRoot, "third-party", "schematic-renderer-LICENSE"),
);

const threeEntry = require.resolve("three");
const threeRoot = join(dirname(threeEntry), "..");
await copyFile(join(threeRoot, "LICENSE"), join(projectRoot, "third-party", "three-LICENSE"));

const nucleationPackage = require.resolve("nucleation/package.json");
const nucleationRoot = dirname(nucleationPackage);
await copyFile(
  join(nucleationRoot, "LICENSE"),
  join(projectRoot, "third-party", "nucleation-LICENSE"),
);
