/**
 * Builds the app's bundled demo schematics.
 *
 * Every model here is authored in this file, block by block, so the bundled
 * demos carry no third-party content and no attribution requirement beyond the
 * project's own license. Run `pnpm --dir Renderer run demos` after changing a
 * model; the script refuses to write a file it cannot read back.
 *
 * Seven models exist because there are seven supported file formats, and each
 * format gets its own build so the gallery shows a distinct model rather
 * than the same one seven times. Nucleation writes the litematic, sponge,
 * snapshot, and mcstructure formats; the classic MCEdit and Java structure
 * formats are import-only, so this script encodes those two itself.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { gzipSync, gunzipSync } from "fflate";
import { Schematic } from "nucleation";

const outputDirectory = join(import.meta.dirname, "..", "..", "Fixtures", "Demos");

/**
 * gzip stamps the current time into its header by default, which would make
 * every run rewrite the fixtures with different bytes. Pinning it keeps the
 * generator idempotent, so a rebuild only shows a diff when a model changed.
 */
const deterministicGzip = { mtime: 0 };

/** Nucleation's own decoding limits. */
const decodeLimits = JSON.stringify({
  max_block_entities: 100_000,
  max_decompressed_bytes: 1_024 * 1_024 * 1_024,
  max_dimension: 4_096,
  max_entities: 100_000,
  max_input_bytes: 1_024 * 1_024 * 1_024,
  max_nbt_collection_items: 33_554_432,
  max_nbt_depth: 64,
  max_nbt_nodes: 4_194_304,
  max_nbt_string_bytes: 1_000_000,
  max_palette_entries: 4_096,
  max_regions: 64,
  max_volume: 16_777_216,
});

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/** A voxel position. */
type Position = readonly [number, number, number];

/** Splits an `"x,y,z"` key back into coordinates; keys only come from `Model.set`. */
function keyToPosition(key: string): Position {
  const [x, y, z] = key.split(",");
  return [Number(x), Number(y), Number(z)];
}

/**
 * A sparse voxel model keyed by position, holding the block state string a
 * player would write. Coordinates are absolute within the model's own box.
 */
class Model {
  blocks = new Map<string, string>();

  set(x: number, y: number, z: number, state: string): this {
    if (!Number.isInteger(x) || !Number.isInteger(y) || !Number.isInteger(z)) {
      throw new Error(`Block position must be integral: ${x}, ${y}, ${z}`);
    }
    this.blocks.set(`${x},${y},${z}`, state);
    return this;
  }

  /** Skips a cell so a later pass can leave a hole, e.g. for a doorway. */
  clear(x: number, y: number, z: number): this {
    this.blocks.delete(`${x},${y},${z}`);
    return this;
  }

  has(x: number, y: number, z: number): boolean {
    return this.blocks.has(`${x},${y},${z}`);
  }

  /** Lets callers walk a model's placed blocks uniformly, whatever built it. */
  *[Symbol.iterator](): Generator<[string, string]> {
    yield* this.blocks;
  }

  keys(): IterableIterator<string> {
    return this.blocks.keys();
  }

  /** Fills an inclusive box, in the order the caller expects overwrites to win. */
  box([x0, y0, z0]: Position, [x1, y1, z1]: Position, state: string): this {
    for (let x = x0; x <= x1; x += 1) {
      for (let y = y0; y <= y1; y += 1) {
        for (let z = z0; z <= z1; z += 1) {
          this.set(x, y, z, state);
        }
      }
    }
    return this;
  }

  /** Fills a box's shell, leaving its interior alone. */
  shell([x0, y0, z0]: Position, [x1, y1, z1]: Position, state: string): this {
    for (let x = x0; x <= x1; x += 1) {
      for (let y = y0; y <= y1; y += 1) {
        for (let z = z0; z <= z1; z += 1) {
          const onEdge = x === x0 || x === x1 || y === y0 || y === y1 || z === z0 || z === z1;
          if (onEdge) {
            this.set(x, y, z, state);
          }
        }
      }
    }
    return this;
  }

  /** A hollow square tube: four walls, no floor or ceiling. */
  walls([x0, y0, z0]: Position, [x1, y1, z1]: Position, state: string): this {
    for (let x = x0; x <= x1; x += 1) {
      for (let y = y0; y <= y1; y += 1) {
        for (let z = z0; z <= z1; z += 1) {
          const onEdge = x === x0 || x === x1 || z === z0 || z === z1;
          if (onEdge) {
            this.set(x, y, z, state);
          }
        }
      }
    }
    return this;
  }

  /**
   * A gable roof over a rectangle: one course per level, each inset by one
   * along the short axis so the two slopes meet at a ridge.
   *
   * `courses` caps how tall the roof gets. Without it the roof rises until the
   * slopes meet, which over a wide building buries the walls entirely.
   */
  gable(
    [x0, y0, z0]: Position,
    [x1, z1]: readonly [number, number],
    state: string,
    { ridgeAxis = "x", courses = Infinity }: { ridgeAxis?: "x" | "y"; courses?: number } = {},
  ): this {
    let low = ridgeAxis === "x" ? z0 : x0;
    let high = ridgeAxis === "x" ? z1 : x1;
    let level = 0;
    while (low <= high && level < courses) {
      const y = y0 + level;
      // Past the cap the ridge is a flat run, so the last course is solid.
      const closed = level === courses - 1 && low < high;
      if (ridgeAxis === "x") {
        this.box([x0, y, low], [x1, y, closed ? high : low], state);
        if (!closed && high !== low) {
          this.box([x0, y, high], [x1, y, high], state);
        }
      } else {
        this.box([low, y, z0], [closed ? high : low, y, z1], state);
        if (!closed && high !== low) {
          this.box([high, y, z0], [high, y, z1], state);
        }
      }
      low += 1;
      high -= 1;
      level += 1;
    }
    return this;
  }

  /** A regular octagon, the closest a voxel cylinder gets to round. */
  cylinder(
    cx: number,
    cz: number,
    radius: number,
    y0: number,
    y1: number,
    state: string,
    wallOnly = false,
  ): this {
    for (let x = cx - radius; x <= cx + radius; x += 1) {
      for (let z = cz - radius; z <= cz + radius; z += 1) {
        const dx = x - cx;
        const dz = z - cz;
        // A slightly generous radius reads as round at these sizes.
        if (dx * dx + dz * dz > radius * radius + radius) {
          continue;
        }
        const rim = Math.abs(dx) >= radius - 1 || Math.abs(dz) >= radius - 1;
        for (let y = y0; y <= y1; y += 1) {
          if (!wallOnly || rim) {
            this.set(x, y, z, state);
          }
        }
      }
    }
    return this;
  }

  /** A hollow ball of leaves, thinned at the poles so canopies do not look cubic. */
  canopy(cx: number, cy: number, cz: number, radius: number, state: string): this {
    for (let x = cx - radius; x <= cx + radius; x += 1) {
      for (let y = cy - radius; y <= cy + radius; y += 1) {
        for (let z = cz - radius; z <= cz + radius; z += 1) {
          const dx = x - cx;
          const dy = y - cy;
          const dz = z - cz;
          const distance = dx * dx + dy * dy + dz * dz;
          if (distance > radius * radius + radius) {
            continue;
          }
          this.set(x, y, z, state);
        }
      }
    }
    return this;
  }

  /** A stepped pyramid whose base is a full square and whose top is `1 × 1`. */
  pyramid(
    [x0, z0]: readonly [number, number],
    [x1, z1]: readonly [number, number],
    y0: number,
    state: string,
  ): this {
    let lowX = x0;
    let lowZ = z0;
    let highX = x1;
    let highZ = z1;
    let y = y0;
    while (lowX <= highX && lowZ <= highZ) {
      this.box([lowX, y, lowZ], [highX, y, highZ], state);
      lowX += 1;
      lowZ += 1;
      highX -= 1;
      highZ -= 1;
      y += 1;
    }
    return this;
  }

  get size(): { x: number; y: number; z: number } {
    const xs: number[] = [];
    const ys: number[] = [];
    const zs: number[] = [];
    for (const key of this.blocks.keys()) {
      const [x, y, z] = keyToPosition(key);
      xs.push(x);
      ys.push(y);
      zs.push(z);
    }

    return {
      x: Math.max(...xs) + 1,
      y: Math.max(...ys) + 1,
      z: Math.max(...zs) + 1,
    };
  }
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

const oak = "minecraft:oak_planks";
const spruce = "minecraft:spruce_planks";
const spruceLog = "minecraft:spruce_log[axis=x]";
const stoneBrick = "minecraft:stone_bricks";
const mossyBrick = "minecraft:mossy_stone_bricks";
const chiseledBrick = "minecraft:chiseled_stone_bricks";
const glass = "minecraft:glass";
const grass = "minecraft:grass_block";
const dirtPath = "minecraft:dirt_path";
const oakLeaf = "minecraft:oak_leaves[persistent=true]";
const lantern = "minecraft:lantern";
const sandstone = "minecraft:sandstone";
const smoothSandstone = "minecraft:smooth_sandstone";
const cutSandstone = "minecraft:cut_sandstone";
const water = "minecraft:water[level=0]";
const fence = "minecraft:oak_fence";
const sprucFence = "minecraft:spruce_fence";

/** A timber cottage: stone footing, plank walls, glazed windows, gable roof. */
function cottage(): Model {
  const model = new Model();
  // 13 × 9 footprint. The depth matters: a 1:1 gable closes in
  // ceil((depth + 2) / 2) courses, so 9 deep closes in 6 and the roof reaches a
  // real ridge instead of flattening into a slab on top of the walls.
  model.box([0, 0, 0], [12, 0, 8], stoneBrick);

  // Walls, four courses, so the roof does not dwarf the building.
  model.shell([0, 1, 0], [12, 4, 8], oak);

  // Corner posts in a darker timber.
  for (const [x, z] of [
    [0, 0],
    [12, 0],
    [0, 8],
    [12, 8],
  ] as const) {
    model.box([x, 1, z], [x, 4, z], "minecraft:spruce_log[axis=y]");
  }

  // Windows on every wall, at eye height.
  for (const x of [3, 5, 7, 9]) {
    model.set(x, 3, 0, glass);
  }
  for (const z of [2, 4, 6]) {
    model.set(0, 3, z, glass);
    model.set(12, 3, z, glass);
  }

  // Doorway, two blocks wide and tall.
  for (const x of [5, 6]) {
    model.clear(x, 1, 0);
    model.clear(x, 2, 0);
  }

  // Roof: six courses of spruce overhanging the walls by one block, which
  // brings the slopes to a ridge at y=10.
  model.gable([-1, 5, -1], [13, 9], spruce);
  model.gable([-1, 5, -1], [13, 9], spruceLog);

  // A path to the door, with two planted posts.
  model.box([5, 0, -3], [6, 0, -1], dirtPath);
  model.set(4, 0, -2, "minecraft:oak_log[axis=y]");
  model.set(7, 0, -2, "minecraft:oak_log[axis=y]");

  // Hanging lanterns either side of the door.
  model.set(4, 3, 0, lantern);
  model.set(7, 3, 0, lantern);
  return model;
}

/**
 * A stone gateway: two piers carrying a corbelled arch, wide enough that the
 * opening stays the subject of the model. A narrow opening reads as a wall.
 */
function archway(): Model {
  const model = new Model();
  const deck = 15;

  model.box([0, 0, 0], [deck, 0, 4], stoneBrick);

  // Piers, two blocks thick, leaving a nine-wide opening between them.
  model.box([0, 1, 0], [2, 6, 4], stoneBrick);
  model.box([12, 1, 0], [14, 6, 4], stoneBrick);

  // Corbelled arch: each course steps one block further in from each side, so
  // the opening closes over five courses instead of a flat lintel.
  for (let step = 0; step < 5; step += 1) {
    const inset = 3 + step;
    model.box([inset, 7 + step, 0], [inset, 7 + step, 4], stoneBrick);
    model.box([14 - inset, 7 + step, 0], [14 - inset, 7 + step, 4], stoneBrick);
  }

  // Keystone closing the crown, in the dressed block.
  model.box([7, 11, 0], [7, 11, 4], chiseledBrick);

  // Top course running the full width, tying the piers together.
  model.box([0, 12, 0], [14, 12, 4], stoneBrick);
  model.box([0, 13, 0], [14, 13, 4], chiseledBrick);

  // A mossy footing and a course of moss along the springing, for age.
  model.box([0, 0, 0], [2, 0, 4], mossyBrick);
  model.box([12, 0, 0], [14, 0, 4], mossyBrick);
  model.box([3, 6, 0], [3, 6, 4], mossyBrick);
  model.box([11, 6, 0], [11, 6, 4], mossyBrick);

  // Lanterns hanging under each side of the arch.
  for (const x of [4, 10]) {
    model.set(x, 9, 1, lantern);
    model.set(x, 9, 3, lantern);
  }

  // A worn path through the opening.
  model.box([3, 0, 1], [11, 0, 3], dirtPath);
  return model;
}

/** A broad oak with a layered canopy and a flowering floor. */
function bloom(): Model {
  const model = new Model();
  model.box([0, 0, 0], [10, 0, 10], grass);
  for (let x = 0; x <= 10; x += 1) {
    for (let z = 0; z <= 10; z += 1) {
      if ((x + z) % 3 === 0 && x > 0 && z > 0 && x < 10 && z < 10) {
        model.set(x, 1, z, "minecraft:short_grass");
      }
    }
  }

  // Trunk, tapering with a root flare.
  model.box([5, 1, 5], [5, 7, 5], "minecraft:oak_log[axis=y]");
  for (const [x, z] of [
    [4, 5],
    [6, 5],
    [5, 4],
    [5, 6],
  ] as const) {
    model.set(x, 1, z, "minecraft:oak_log[axis=y]");
  }

  // Canopy: a wide base ball plus a smaller crown, so the silhouette reads as
  // a tree rather than a sphere on a stick.
  model.canopy(5, 9, 5, 3, oakLeaf);
  model.canopy(5, 11, 5, 2, oakLeaf);
  model.set(5, 12, 5, oakLeaf);

  // Flowers around the base.
  for (const [x, z, state] of [
    [2, 2, "minecraft:dandelion"],
    [8, 2, "minecraft:poppy"],
    [2, 8, "minecraft:poppy"],
    [8, 8, "minecraft:dandelion"],
    [3, 6, "minecraft:azure_bluet"],
    [7, 4, "minecraft:cornflower"],
  ] as const) {
    model.set(x, 1, z, state);
  }

  return model;
}

/** A plank bridge on stone piers, with a fenced walkway. */
function bridge(): Model {
  const model = new Model();
  model.box([0, 0, 0], [16, 0, 8], "minecraft:water[level=0]");

  // Deck.
  model.box([0, 3, 3], [16, 3, 5], oak);
  for (let x = 0; x <= 16; x += 1) {
    model.set(x, 3, 2, spruce);
    model.set(x, 3, 6, spruce);
  }

  // Piers reaching into the water.
  for (const x of [2, 8, 14]) {
    model.box([x, 1, 3], [x, 2, 5], stoneBrick);
    model.set(x, 0, 3, mossyBrick);
    model.set(x, 0, 5, mossyBrick);
  }

  // Railings, with a gap at each end for the approach.
  for (let x = 1; x <= 15; x += 1) {
    model.set(x, 4, 2, fence);
    model.set(x, 4, 6, fence);
  }
  for (const z of [2, 6]) {
    model.set(0, 4, z, "minecraft:spruce_fence");
    model.set(16, 4, z, sprucFence);
    model.set(0, 5, z, "minecraft:spruce_fence");
    model.set(16, 5, z, sprucFence);
  }

  // Lamp posts at the four corners.
  for (const [x, z] of [
    [1, 2],
    [15, 2],
    [1, 6],
    [15, 6],
  ] as const) {
    model.set(x, 5, z, "minecraft:oak_fence");
    model.set(x, 6, z, lantern);
  }

  // Approaches rising out of the water.
  model.box([0, 2, 3], [1, 2, 5], stoneBrick);
  model.box([15, 2, 3], [16, 2, 5], stoneBrick);
  return model;
}

/** A stepped sandstone pyramid crowned with a beacon. */
function pyramid(): Model {
  const model = new Model();
  model.box([0, 0, 0], [14, 0, 14], smoothSandstone);
  model.pyramid([0, 0], [14, 14], 0, sandstone);

  // Highlight every other step so the tiers stay legible in 3D.
  for (let level = 1; level < 7; level += 2) {
    const inset = level;
    model.box([inset, level, inset], [14 - inset, level, 14 - inset], cutSandstone);
  }

  // A lapis-and-gold crown.
  model.box([6, 8, 6], [8, 8, 8], "minecraft:gold_block");
  model.set(7, 9, 7, "minecraft:lapis_block");
  model.set(7, 10, 7, "minecraft:beacon");

  // Four stairways up the faces.
  for (let step = 1; step <= 5; step += 1) {
    model.set(7, step, 6 - step - 1, "minecraft:sandstone_stairs[facing=south]");
    model.set(7, step, 14 - (6 - step - 1), "minecraft:sandstone_stairs[facing=north]");
  }

  return model;
}

/** A walled garden with a pond, hedges, and lantern-lit path. */
function garden(): Model {
  const model = new Model();
  model.box([0, 0, 0], [12, 0, 12], grass);

  // Pond: water at the surface, with sand and clay showing at its edges.
  model.box([2, 0, 2], [6, 0, 6], water);
  for (const [x, z] of [
    [1, 2],
    [1, 3],
    [7, 4],
    [5, 7],
    [2, 7],
  ] as const) {
    model.set(x, 0, z, "minecraft:sand");
  }
  for (const [x, z] of [
    [3, 3],
    [5, 5],
  ] as const) {
    model.set(x, 0, z, "minecraft:clay");
    model.set(x, 1, z, "minecraft:lily_pad");
  }

  // Winding path.
  model.box([8, 0, 0], [9, 0, 12], dirtPath);
  model.box([8, 0, 8], [12, 0, 9], dirtPath);

  // Hedge rows along two edges.
  model.box([0, 1, 0], [0, 2, 12], "minecraft:oak_leaves[persistent=true]");
  model.box([0, 1, 12], [12, 2, 12], "minecraft:oak_leaves[persistent=true]");

  // Flower beds.
  for (const [x, z, state] of [
    [2, 9, "minecraft:dandelion"],
    [3, 10, "minecraft:poppy"],
    [4, 9, "minecraft:cornflower"],
    [5, 10, "minecraft:azure_bluet"],
    [10, 2, "minecraft:poppy"],
    [11, 4, "minecraft:dandelion"],
    [10, 6, "minecraft:allium"],
  ] as const) {
    model.set(x, 1, z, state);
  }

  // A bench and two lamp posts.
  model.box([10, 1, 10], [12, 1, 10], oak);
  model.set(10, 2, 11, "minecraft:oak_fence");
  model.set(12, 2, 11, "minecraft:oak_fence");
  for (const [x, z] of [
    [8, 3],
    [8, 10],
  ] as const) {
    model.set(x, 1, z, "minecraft:oak_fence");
    model.set(x, 2, z, "minecraft:oak_fence");
    model.set(x, 3, z, lantern);
  }

  return model;
}

// ---------------------------------------------------------------------------
// File format encoding
// ---------------------------------------------------------------------------

/** Builds a Nucleation schematic, which is what most of the writers consume. */
function toNucleation(
  model: Model,
  { name, author, description }: { name: string; author: string; description: string },
): Schematic {
  const schematic = Schematic.create(name);
  schematic.setAuthor(author);
  schematic.setDescription(description);

  for (const [key, state] of model.blocks) {
    const [x, y, z] = keyToPosition(key);
    schematic.setBlockFromString(x, y, z, state);
  }

  return schematic;
}

/** One written NBT tag. The variants mirror the format's own tag numbering. */
type NbtTag =
  | { kind: "byte"; value: number }
  | { kind: "short"; value: number }
  | { kind: "int"; value: number }
  | { kind: "long"; value: bigint }
  | { kind: "float"; value: number }
  | { kind: "double"; value: number }
  | { kind: "byteArray"; value: number[] }
  | { kind: "intArray"; value: number[] }
  | { kind: "longArray"; value: bigint[] }
  | { kind: "string"; value: string }
  | { kind: "list"; elementId: number; value: NbtTag[] }
  | { kind: "compound"; value: [string, NbtTag][] };

/**
 * A big-endian NBT writer. Java edition NBT is network byte order, and the tag
 * ids below are the format's own numbering (4 is Long, 7 is ByteArray, 11 is
 * IntArray, 12 is LongArray — not payload-width order).
 */
class NbtWriter {
  #chunks: Uint8Array[] = [];
  #scratch = new ArrayBuffer(8);

  #number(write: (view: DataView) => void, width: number): void {
    const view = new DataView(this.#scratch);
    write(view);
    this.#chunks.push(new Uint8Array(this.#scratch.slice(0, width)));
  }

  string(value: string): void {
    const bytes = new TextEncoder().encode(value);
    this.#number((view) => view.setUint16(0, bytes.length), 2);
    this.#chunks.push(bytes);
  }

  #payload(tag: NbtTag): void {
    switch (tag.kind) {
      case "byte":
        return this.#number((view) => view.setInt8(0, tag.value), 1);
      case "short":
        return this.#number((view) => view.setInt16(0, tag.value), 2);
      case "int":
        return this.#number((view) => view.setInt32(0, tag.value), 4);
      case "long":
        return this.#number((view) => view.setBigInt64(0, tag.value), 8);
      case "float":
        return this.#number((view) => view.setFloat32(0, tag.value), 4);
      case "double":
        return this.#number((view) => view.setFloat64(0, tag.value), 8);
      case "byteArray":
        this.#number((view) => view.setInt32(0, tag.value.length), 4);
        this.#chunks.push(Uint8Array.from(tag.value, (byte) => byte & 0xff));
        return;
      case "intArray":
        this.#number((view) => view.setInt32(0, tag.value.length), 4);
        for (const element of tag.value) {
          this.#number((view) => view.setInt32(0, element), 4);
        }
        return;
      case "string":
        return this.string(tag.value);
      case "list": {
        // An empty list declares TAG_End as its element id.
        this.#chunks.push(new Uint8Array([tag.value.length === 0 ? 0 : tag.elementId]));
        this.#number((view) => view.setInt32(0, tag.value.length), 4);
        for (const element of tag.value) {
          this.#payload(element);
        }
        return;
      }
      case "compound": {
        for (const [key, value] of tag.value) {
          this.#chunks.push(new Uint8Array([tagIds[value.kind]]));
          this.string(key);
          this.#payload(value);
        }
        this.#chunks.push(new Uint8Array([0]));
        return;
      }
    }
  }

  finish(root: NbtTag): Uint8Array {
    this.#chunks.push(new Uint8Array([10, 0, 0]));
    this.#payload(root);
    const length = this.#chunks.reduce((total, chunk) => total + chunk.length, 0);
    const out = new Uint8Array(length);
    let offset = 0;
    for (const chunk of this.#chunks) {
      out.set(chunk, offset);
      offset += chunk.length;
    }

    return out;
  }
}

const tagIds: Record<NbtTag["kind"], number> = {
  byte: 1,
  short: 2,
  int: 3,
  long: 4,
  float: 5,
  double: 6,
  byteArray: 7,
  string: 8,
  list: 9,
  compound: 10,
  intArray: 11,
  longArray: 12,
};

const int = (value: number): NbtTag => ({ kind: "int", value });
const short = (value: number): NbtTag => ({ kind: "short", value });
const string = (value: string): NbtTag => ({ kind: "string", value });
const compound = (entries: [string, NbtTag][]): NbtTag => ({ kind: "compound", value: entries });
/** List element ids are written explicitly so an empty list still knows its type. */
const compoundList = (values: NbtTag[]): NbtTag => ({ kind: "list", elementId: 10, value: values });
const intList = (values: readonly number[]): NbtTag => ({
  kind: "list",
  elementId: 3,
  value: values.map((value) => int(value)),
});

/**
 * Encodes a Java structure `.nbt`: a palette of block-state compounds, a flat
 * list of placed blocks, and the enclosing size, everything gzipped.
 */
function encodeStructure(model: Model): Uint8Array {
  const palette: string[] = [];
  const indices = new Map<string, number>();
  const blocks: NbtTag[] = [];

  for (const [key, state] of [...model.blocks].sort(byPosition)) {
    if (!indices.has(state)) {
      indices.set(state, palette.length);
      palette.push(state);
    }
    const [x, y, z] = keyToPosition(key);
    blocks.push(
      compound([
        ["pos", intList([x, y, z])],
        ["state", int(indices.get(state)!)],
      ]),
    );
  }

  const size = model.size;
  return gzipSync(
    new NbtWriter().finish(
      compound([
        ["DataVersion", int(3_953)],
        ["size", intList([size.x, size.y, size.z])],
        ["palette", compoundList(palette.map(structurePaletteEntry))],
        ["blocks", compoundList(blocks)],
        ["entities", compoundList([])],
      ]),
    ),
    deterministicGzip,
  );
}

/** Splits `minecraft:oak_log[axis=y]` into a structure palette compound. */
function structurePaletteEntry(state: string): NbtTag {
  const bracket = state.indexOf("[");
  if (bracket < 0) {
    return compound([["Name", string(state)]]);
  }

  const name = state.slice(0, bracket);
  const body = state.slice(bracket + 1, -1);
  const properties = body.split(",").map((pair): [string, NbtTag] => {
    const [key, value] = pair.split("=") as [string, string];
    return [key, string(value)];
  });

  return compound([
    ["Name", string(name)],
    // Sorted so the encoding is stable across runs.
    ["Properties", compound(properties.sort(([left], [right]) => (left < right ? -1 : 1)))],
  ]);
}

/**
 * Encodes structure SNBT by hand rather than through Nucleation's writer, which
 * pads every region out to its full bounding box. Only placed blocks are
 * listed here, which is what a person writing this format by hand produces.
 */
function encodeStructureSnbt(model: Model): Uint8Array {
  const palette = new Map<string, number>();
  for (const [, state] of model.blocks) {
    if (!palette.has(state)) {
      palette.set(state, palette.size);
    }
  }

  const body = [...model.blocks]
    .sort(byPosition)
    .map(([key, state]) => {
      const [x, y, z] = keyToPosition(key);
      return `{pos:[${x},${y},${z}],state:"${state}"}`;
    })
    .join(",");

  return new TextEncoder().encode(
    `{DataVersion:3953,size:[${model.size.x},${model.size.y},${model.size.z}],` +
      `palette:[${[...palette.keys()].map((state) => `"${state}"`).join(",")}],` +
      `data:[${body}],entities:[]}`,
  );
}

function byPosition([left]: [string, string], [right]: [string, string]): number {
  const a = keyToPosition(left);
  const b = keyToPosition(right);
  return a[1] - b[1] || a[2] - b[2] || a[0] - b[0];
}

// ---------------------------------------------------------------------------
// Classic MCEdit `.schematic`
// ---------------------------------------------------------------------------

/**
 * Legacy numeric block ids, the only vocabulary the MCEdit file format can
 * express. Each entry is the id and metadata this script encodes; the decoded
 * name is what Nucleation's own legacy table produces, which the round-trip
 * check at the bottom compares against.
 *
 * Wooden planks (id 5) are deliberately absent: Nucleation's `wood_variant`
 * ignores the suffix it is given, so id 5 decodes to the non-existent block
 * `minecraft:oak` rather than `minecraft:oak_planks`.
 */
const legacyBlocks: Record<string, [number, number]> = {
  "minecraft:cobblestone": [4, 0],
  "minecraft:glass": [20, 0],
  "minecraft:stone_bricks": [98, 0],
  "minecraft:mossy_stone_bricks": [98, 1],
  "minecraft:chiseled_stone_bricks": [98, 3],
  "minecraft:bricks": [45, 0],
  "minecraft:glowstone": [89, 0],
  "minecraft:oak_fence": [85, 0],
  "minecraft:glass_pane": [102, 0],
};

/**
 * A round tower in the legacy palette. It is authored separately from the
 * watchtower above because the MCEdit file format cannot carry slabs, stairs, or
 * modern palettes, so sharing one model would force every other format down to
 * this vocabulary too.
 */
function legacyTower(): Model {
  const model = new Model();
  const centre = 4;

  const inOctagon = (x: number, z: number, radius: number): boolean => {
    const dx = x - centre;
    const dz = z - centre;
    return dx * dx + dz * dz <= radius * radius + radius;
  };

  for (let x = 0; x <= 8; x += 1) {
    for (let z = 0; z <= 8; z += 1) {
      if (!inOctagon(x, z, 4)) {
        continue;
      }
      model.set(x, 0, z, "minecraft:cobblestone");
      for (let y = 1; y <= 13; y += 1) {
        if (inOctagon(x, z, 3)) {
          continue;
        }
        model.set(
          x,
          y,
          z,
          y % 5 === 0 ? "minecraft:chiseled_stone_bricks" : "minecraft:stone_bricks",
        );
      }
    }
  }

  // Window slits on the four cardinal faces.
  for (const y of [3, 7, 11]) {
    model.set(centre - 3, y, centre, "minecraft:glass_pane");
    model.set(centre + 3, y, centre, "minecraft:glass_pane");
    model.set(centre, y, centre - 3, "minecraft:glass_pane");
    model.set(centre, y, centre + 3, "minecraft:glass_pane");
  }

  // Brick buttresses at the base.
  for (const [x, z] of [
    [centre - 4, centre],
    [centre + 4, centre],
    [centre, centre - 4],
    [centre, centre + 4],
  ] as const) {
    model.set(x, 1, z, "minecraft:bricks");
    model.set(x, 2, z, "minecraft:bricks");
  }

  // Timber balcony, then alternating merlons above it.
  for (let x = 0; x <= 8; x += 1) {
    for (let z = 0; z <= 8; z += 1) {
      if (!inOctagon(x, z, 4)) {
        continue;
      }
      model.set(x, 14, z, "minecraft:bricks");
      if (!inOctagon(x, z, 3) && (x + z) % 2 === 0) {
        model.set(x, 15, z, "minecraft:stone_bricks");
        model.set(x, 16, z, "minecraft:stone_bricks");
      }
    }
  }

  // A fence post carrying a lamp, so the silhouette has a focal point.
  model.set(centre, 15, centre, "minecraft:oak_fence");
  model.set(centre, 16, centre, "minecraft:glowstone");

  // Doorway through the base.
  model.clear(centre, 1, 0);
  model.clear(centre, 2, 0);

  return model;
}

function encodeClassic(model: Model): Uint8Array {
  let width = 0;
  let height = 0;
  let length = 0;
  for (const key of model.keys()) {
    const [x, y, z] = keyToPosition(key);
    width = Math.max(width, x + 1);
    height = Math.max(height, y + 1);
    length = Math.max(length, z + 1);
  }

  // MCEdit indexes `y * length * width + z * width + x`.
  const blocks = new Uint8Array(width * height * length);
  const data = new Uint8Array(width * height * length);
  for (const [key, name] of model) {
    const legacy = legacyBlocks[name];
    if (legacy === undefined) {
      throw new Error(`The MCEdit file format cannot express ${name}`);
    }
    const [x, y, z] = keyToPosition(key);
    const [id, meta] = legacy;
    const index = y * length * width + z * width + x;
    blocks[index] = id;
    data[index] = meta;
  }
  return gzipSync(
    new NbtWriter().finish(
      compound([
        ["Width", short(width)],
        ["Height", short(height)],
        ["Length", short(length)],
        ["Materials", string("Alpha")],
        ["Blocks", { kind: "byteArray", value: [...blocks].map(toSignedByte) }],
        ["Data", { kind: "byteArray", value: [...data].map(toSignedByte) }],
        ["Entities", compoundList([])],
        ["TileEntities", compoundList([])],
      ]),
    ),
    deterministicGzip,
  );
}

/** NBT stores byte arrays as signed values. */
function toSignedByte(value: number): number {
  return value > 127 ? value - 256 : value;
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

const common = {
  author: "LitematicaQL",
  description: "Bundled demo built by the LitematicaQL renderer's generator.",
};

interface Demo {
  file: string;
  /** Which of the seven supported formats this demo is written in. */
  format: "litematic" | "sponge" | "classic" | "structure" | "snbt" | "bedrock" | "snapshot";
  model: Model;
  encode: (model: Model) => Uint8Array;
}

const demos: Demo[] = [
  {
    file: "Cottage.litematic",
    format: "litematic",
    model: cottage(),
    encode: (model) =>
      Buffer.from(toNucleation(model, { ...common, name: "Cottage" }).toLitematicB64(), "base64"),
  },
  {
    file: "Archway.schem",
    format: "sponge",
    model: archway(),
    // Sponge v3 is what WorldEdit writes today; the v2 reader is covered by
    // the format fixtures instead.
    encode: (model) =>
      Buffer.from(
        toNucleation(model, { ...common, name: "Archway" }).saveAsB64("schematic", "v3", ""),
        "base64",
      ),
  },
  {
    file: "Watchtower.schematic",
    format: "classic",
    model: legacyTower(),
    encode: encodeClassic,
  },
  {
    file: "Bloom.nbt",
    format: "structure",
    model: bloom(),
    encode: encodeStructure,
  },
  {
    file: "Bridge.snbt",
    format: "snbt",
    model: bridge(),
    encode: encodeStructureSnbt,
  },
  {
    file: "Pyramid.mcstructure",
    format: "bedrock",
    model: pyramid(),
    encode: (model) =>
      Buffer.from(toNucleation(model, { ...common, name: "Pyramid" }).toMcstructureB64(), "base64"),
  },
  {
    file: "Garden.nusn",
    format: "snapshot",
    model: garden(),
    encode: (model) =>
      Buffer.from(toNucleation(model, { ...common, name: "Garden" }).toSnapshotB64(), "base64"),
  },
];

/**
 * Reads a structure `.nbt` back into positions and block states.
 *
 * Nucleation has no importer for the binary structure file format, so nothing
 * else can read this file back. This minimal reader exists to verify the
 * encoder on the writing side, so a malformed tag or a mis-sized palette
 * fails the build here.
 */
function readStructureNbt(compressed: Uint8Array): Map<string, string> {
  const bytes = gunzipSync(compressed);
  let offset = 0;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const decoder = new TextDecoder();

  const take = (count: number): number => {
    if (offset + count > view.byteLength) {
      throw new Error("Truncated structure NBT");
    }
    const start = offset;
    offset += count;
    return start;
  };
  const name = (): string => {
    const length = view.getUint16(take(2));
    return decoder.decode(new Uint8Array(view.buffer, view.byteOffset + take(length), length));
  };

  const payload = (tag: number): NbtValue => {
    switch (tag) {
      case 1:
        return view.getInt8(take(1));
      case 2:
        return view.getInt16(take(2));
      case 3:
        return view.getInt32(take(4));
      case 4:
        return view.getBigInt64(take(8));
      case 5:
        return view.getFloat32(take(4));
      case 6:
        return view.getFloat64(take(8));
      case 7: {
        const count = view.getInt32(take(4));
        return new Int8Array(view.buffer, view.byteOffset + take(count), count);
      }
      case 8:
        return name();
      case 9: {
        const elementTag = view.getUint8(take(1));
        const count = view.getInt32(take(4));
        return Array.from({ length: count }, () => payload(elementTag));
      }
      case 10: {
        const entries: ParsedCompound = new Map();
        for (;;) {
          const childTag = view.getUint8(take(1));
          if (childTag === 0) {
            return entries;
          }
          const key = name();
          entries.set(key, { tag: childTag, value: payload(childTag) });
        }
      }
      case 11: {
        const count = view.getInt32(take(4));
        return Array.from({ length: count }, () => view.getInt32(take(4)));
      }
      default: {
        const count = view.getInt32(take(4));
        return Array.from({ length: count }, () => view.getBigInt64(take(8)));
      }
    }
  };

  if (view.getUint8(take(1)) !== 10) {
    throw new Error("Structure NBT root is not a compound");
  }
  name();
  const root = payload(10) as ParsedCompound;

  // A parsed list holds bare payloads, so palette entries arrive as maps
  // rather than the {tag, value} envelopes a compound's children carry.
  const palette = (root.get("palette")!.value as ParsedCompound[]).map((entry) => {
    const blockName = entry.get("Name")!.value as string;
    const properties = entry.get("Properties");
    if (properties === undefined) {
      return blockName;
    }

    const pairs = [...(properties.value as ParsedCompound)].map(
      ([key, value]) => `${key}=${value.value as string}`,
    );
    return `${blockName}[${pairs.sort().join(",")}]`;
  });

  const blocks = new Map<string, string>();
  for (const entry of root.get("blocks")!.value as ParsedCompound[]) {
    const [x, y, z] = entry.get("pos")!.value as [number, number, number];
    const state = palette[entry.get("state")!.value as number]!;
    blocks.set(`${x},${y},${z}`, state);
  }

  return blocks;
}

/** One parsed NBT tag: a numeric id plus whatever payload that id implies. */
interface ParsedTag {
  tag: number;
  value: NbtValue;
}

/** A parsed compound's children, keyed by name. */
type ParsedCompound = Map<string, ParsedTag>;

/** Everything the minimal reader can produce; lists hold bare payloads. */
type NbtValue = number | bigint | string | Int8Array | NbtValue[] | ParsedCompound;

mkdirSync(outputDirectory, { recursive: true });

let mismatches = 0;
for (const demo of demos) {
  const written = demo.encode(demo.model);
  const size = demo.model.size;
  writeFileSync(join(outputDirectory, demo.file), written);
  console.log(`${demo.file}: ${written.length} bytes, ${size.x}×${size.y}×${size.z}`);

  // Every file format this script encodes by hand has to survive a decode, so a
  // wrong tag id or a mis-sized palette fails the run rather than shipping.
  const decoded = demo.format === "structure" ? readStructureNbt(written) : decodeBlocks(written);
  for (const [key, state] of demo.model) {
    const problem = compare(decoded.get(key), state);
    if (problem) {
      console.error(`  ${demo.file}: ${key} ${problem}`);
      mismatches += 1;
    }
  }
}

if (mismatches > 0) {
  throw new Error(`${mismatches} block(s) did not survive the round trip`);
}

console.log(`\nWrote and verified ${demos.length} demos in ${outputDirectory}`);

/** Reads a file back and maps each position to its block state string. */
function decodeBlocks(bytes: Uint8Array): Map<string, string> {
  // Nucleation's declarations type byte inputs as `Array<number>` even though
  // the runtime accepts typed arrays; casting avoids copying the buffer.
  const schematic = Schematic.fromDataBounded(bytes as unknown as Array<number>, decodeLimits);
  const decoded = new Map<string, string>();
  for (const block of JSON.parse(schematic.getNonAirBlocksJson()) as Array<{
    x: number;
    y: number;
    z: number;
    name: string;
    properties: [string, string][];
  }>) {
    // `properties` arrives as [key, value] pairs, which are rewritten into the
    // same `key=value` form the models are written in.
    const properties = block.properties
      .map(([key, value]) => `${key}=${value}`)
      .sort((left, right) => left.localeCompare(right))
      .join(",");
    decoded.set(
      `${block.x},${block.y},${block.z}`,
      properties.length === 0 ? block.name : `${block.name}[${properties}]`,
    );
  }

  return decoded;
}

/** Splits `minecraft:oak_log[axis=y]` into its name and its `key=value` pairs. */
function parseState(state: string): { name: string; properties: Map<string, string> } {
  const bracket = state.indexOf("[");
  if (bracket < 0) {
    return { name: state, properties: new Map() };
  }

  const properties = new Map<string, string>();
  for (const pair of state.slice(bracket + 1, -1).split(",")) {
    const [key, value] = pair.split("=") as [string, string];
    properties.set(key, value);
  }

  return { name: state.slice(0, bracket), properties };
}

/**
 * Reports how a decoded block differs from the authored one, or `undefined` when
 * it matches.
 *
 * A file format may fill in defaults the model left out — Nucleation expands
 * `sandstone_stairs[facing=south]` to that block's full legal property set — so
 * the check is that the authored name and every authored property came back,
 * not that the decoded string is character-identical.
 */
function compare(decoded: string | undefined, authored: string): string | undefined {
  if (decoded === undefined) {
    return `is missing (expected ${authored})`;
  }

  const expected = parseState(authored);
  const actual = parseState(decoded);
  if (actual.name !== expected.name) {
    return `decoded as ${actual.name}, expected ${expected.name}`;
  }

  for (const [key, value] of expected.properties) {
    if (actual.properties.get(key) !== value) {
      return `decoded as ${decoded}, which does not carry ${key}=${value}`;
    }
  }

  return undefined;
}
