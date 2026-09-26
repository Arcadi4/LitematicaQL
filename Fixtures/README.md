# Demos

`Demos/` holds the seven schematics the app bundles and shows on its welcome
screen. Every one is authored block by block in
`Renderer/scripts/generate-demos.ts` — they contain no third-party builds, no
scans of anyone else's world, and no attribution requirement beyond this
project's own license. Regenerate them with:

```sh
pnpm --dir Renderer run demos
```

The generator refuses to write a file it cannot read back, so a file format whose
encoding drifts fails the run instead of shipping.

Each supported file format gets its own build, because seven cards showing the
same model seven times would say nothing about the formats:

- `Cottage.litematic` is a timber cottage: a stone footing, four courses of plank walls with glazed windows on every wall, a two-block doorway with lanterns, and a six-course spruce gable roof that overhangs the walls by one block. Its footprint is deliberately 13 × 9, because a gable closes over `ceil((depth + 2) / 2)` courses — make the building deeper and the roof flattens into a slab instead of reaching a ridge.
- `Archway.schem` is a stone gateway: two piers carrying a corbelled arch whose courses step inward to a dressed keystone, with a mossy footing and lanterns hung under the span. The opening is nine blocks wide on purpose, since a narrow one reads as a solid wall.
- `Watchtower.schematic` is an eight-sided tower with window slits, brick buttresses, a brick balcony, alternating merlons, and a lamp on a post. It is authored straight into the MCEdit legacy palette, since that file format cannot carry anything past 1.12 and mixing it with the others would force every format down to that vocabulary.
- `Bloom.nbt` is a broad oak with a tapering trunk, a two-tier canopy, and a flowering floor.
- `Bridge.snbt` is a plank bridge on stone piers, with fenced railings, lamp posts, and approaches rising out of the water.
- `Pyramid.mcstructure` is a stepped sandstone pyramid with highlighted tiers, stairways up two faces, and a lapis-and-gold crown.
- `Garden.nusn` is a walled garden with a lily pond, hedge rows, flower beds, a bench, and lantern-lit posts.

# Format fixtures

`Formats/` holds one minimal file for each supported file format, used by the
renderer's format tests. Each is small enough to be read in full, and the tests
assert what it decodes to rather than any upstream content.

- `Structure.nbt` is a gzipped Java structure. Its palette is `minecraft:oak_log` with `axis=y`, `minecraft:chest`, and `minecraft:air`; it places the log at `(0,0,0)`, the chest at `(2,0,0)` with a block entity, and one armor stand. A third entry names palette index 2, which is air, so it must be dropped on import: two blocks and one entity is the correct result.
- `Structure.snbt` is the same structure as text, spelled with the brace block-state form (`state: "minecraft:oak_log{axis=y}"`), which Nucleation's own reader rejects. It loads through the normalization retry.
- `Classic.schematic` is a gzipped MCEdit `.schematic`: seven `minecraft:stone` cells and one air cell, so it decodes to seven blocks.
- `Sponge.schem` is a gzipped Sponge schematic v2 with the same seven-cell shape and palette. The bundled `Archway.schem` demo is v3, so both supported Sponge versions stay exercised.
- `Bedrock.mcstructure` is an uncompressed Bedrock structure. It is produced by Nucleation's own writer rather than by hand, because its `block_indices` layers are `TAG_List` of `TAG_Int` and a hand-built `TAG_Int_Array` layer is skipped silently. It decodes to seven `minecraft:stone` blocks.
- `Snapshot.nusn` is Nucleation's snapshot of that same seven-block source.
- `Region.mca` is an Anvil region file containing four chunks in a 2×2 cluster. It exercises single-region MCA parsing, 4-chunk window selection, and coordinate normalization.

# Large build fixtures

`Large/` holds two third-party Minecraft builds, each a million blocks or more.
They are the only fixtures that reach the scale where the native bridge's
streaming path matters, and they are never bundled with the app. Both were
published for others to download, and who to credit for each is recorded in
`ThirdParty/large-build-fixtures-NOTICE.txt`.

- `Valkyrie.litematic` is a Litematica export of Space Valkyria 3 V2, a 2b2t End
  base. It is Litematic version 6 from Minecraft 1.19.4, 4,408,947 blocks over a
  1003 × 256 × 1002 content box, and it meshes to 276 batches and 533 MB of
  geometry needing 43 greedy-meshed textures beyond the shared atlas.
- `Atrium.litematic` is a Litematica export of 艾兰德·中庭, the build behind a
  bilibili tutorial. It is Litematic version 5 from Minecraft 1.20.1, 1,121,671
  blocks over a 658 × 253 × 575 content box, and it meshes to 193 batches and
  1.16 GB of geometry needing 40 greedy textures. It is the larger of the two
  in triangles despite having fewer blocks, which is why both are kept.

Together they cover Litematic versions 5 and 6, palettes written by Minecraft
rather than by `generate-demos.ts`, and a model far too big for the shared atlas
to cover. `Core/tests/large_builds.rs` runs them end to end: it decodes, meshes,
and streams every batch, validating each one against the same contract the
renderer's `decodeBatch` enforces, then reports throughput and peak memory
against recorded budgets. Run it with `--nocapture` to see the report, which is
also written to `Core/target/tmp/large-builds.txt`.

Two properties of these builds are worth keeping in mind when reading the test.
Their meshed bounds are the chunk-padded region rather than the content box —
Valkyrie reports 256 blocks of content height against 320 blocks of geometry —
and their peak live memory is a fraction of the geometry they produce, because
the stream meshes and releases one chunk at a time. Both are the reasons the
test asserts on a 64-block grid and on peak bytes rather than on totals.
