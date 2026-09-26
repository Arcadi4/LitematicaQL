//! Real-world builds at million-block scale, through the C ABI.
//!
//! The `Formats` and `Demos` fixtures are authored by this project and stay
//! small enough to hold in memory at once. These two are third-party builds of
//! roughly a million to four million blocks that mesh to about 1.6 GB of
//! geometry, which makes them the only fixtures that exercise:
//!
//! - Litematic versions 5 and 6, and palettes built by Minecraft itself rather
//!   than by `generate-demos.ts`.
//! - The streaming path end to end. `preview.rs` keeps every batch payload in a
//!   `Vec`, so it cannot run these; each batch here is validated and released
//!   before the next is requested, exactly as the host does.
//! - Meshing a build with hundreds of greedy-meshed textures, where the atlas
//!   no longer covers the model and each texture must be introduced by the
//!   first batch that uses it.
//!
//! The test also reports throughput and peak resident geometry, because the
//! failure mode this guards against is a preview that quietly stops scaling:
//! a change that doubles the geometry or holds a second copy of it would still
//! pass a "does it draw" assertion on a fast machine.

mod support;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use litematicaql_native::{
    nql_buffer_free, nql_mesh_stream_free, nql_mesh_stream_next, nql_mesh_stream_open,
    nql_resource_pack_free, nql_resource_pack_open, nql_schematic_free, nql_schematic_info,
    nql_schematic_open, nql_schematic_warnings, status, NQLAtlasInfo, NQLBatchInfo, NQLError,
    NQLMeshInfo, NQLSchematicInfo,
};
use support::AllocationMeter;

#[global_allocator]
static ALLOCATOR: AllocationMeter = AllocationMeter;

/// Litematic regions are padded to whole chunks, and `nql_mesh_stream_open`
/// partitions the model into `CHUNK_SIZE` cubes, so batch bounds land on a
/// 64-block grid. A batch that is misaligned, oversized, or overlaps another
/// would mean geometry is duplicated or dropped.
const CHUNK_SIZE: f32 = 64.0;

/// What the pipeline has to keep producing for a bundled build.
///
/// Counts that come out of integer parsing are pinned exactly: a silent change
/// means blocks were dropped or duplicated, and the model would show a hole or
/// z-fighting that no amount of drawing correctness would catch. Counts that
/// depend on the mesher's floating-point output carry a one percent band so a
/// benign difference in vertex packing does not fail the run.
struct Baseline {
    name: &'static str,
    /// Litematic format version, which the two fixtures straddle.
    litematic_version: i32,
    block_count: i64,
    block_entities: i64,
    content: [i32; 3],
    batches: u32,
    greedy_textures: u32,
    atlas: [u32; 2],
    triangles: u64,
    /// Total batch bytes handed to the host. This is far larger than the peak
    /// live figure because the batches are meshed and released one at a time
    /// instead of all being resident at once; that gap is the whole design,
    /// and a change that closed it would leave the host holding gigabytes it
    /// has no reason to.
    payload_bytes: u64,
    /// Extent of the union of every batch bound.
    geometry: [f32; 3],
    /// Ceiling on peak live bytes while streaming: the recorded peak plus a
    /// quarter, absorbing the few percent of drift from how many chunks the
    /// mesher has in flight. Stated outright rather than as a ratio, because
    /// integer division rounds `5 / 4` to a factor of exactly one.
    peak_budget: usize,
}

/// Drift allowed on the counts the mesher produces, in hundredths of a
/// percent, so a benign difference in vertex packing does not fail a run.
const VOLUME_TOLERANCE: u64 = 100;

const BASELINES: &[Baseline] = &[
    Baseline {
        name: "Atrium",
        litematic_version: 5,
        block_count: 1_121_671,
        block_entities: 337,
        content: [658, 253, 575],
        batches: 193,
        greedy_textures: 40,
        atlas: [512, 512],
        triangles: 10_705_456,
        payload_bytes: 1_156_261_144,
        geometry: [704.0, 256.0, 576.0],
        peak_budget: 246_250_000,
    },
    Baseline {
        name: "Valkyrie",
        litematic_version: 6,
        block_count: 4_408_947,
        block_entities: 2_753,
        content: [1003, 256, 1002],
        batches: 276,
        greedy_textures: 43,
        atlas: [1024, 1024],
        triangles: 4_937_416,
        payload_bytes: 533_333_244,
        geometry: [1024.0, 320.0, 1024.0],
        peak_budget: 218_750_000,
    },
];

/// Time ceilings for one build, covering decode, mesh, and stream together.
///
/// These depend on the machine, so they are loose. Memory does not, so it gets
/// a per-build `peak_budget` instead.
struct Budget {
    /// Total wall clock for one build. Measured 3.2-3.5 s on a ten-core
    /// workstation, so this leaves room for a smaller CI runner.
    total: Duration,
    /// Streaming throughput. Measured 180-390 MB/s; a two-times regression in
    /// the mesher or the batch encoder still clears this.
    throughput_mb_per_s: f64,
}

const BUDGET: Budget = Budget {
    total: Duration::from_secs(20),
    throughput_mb_per_s: 40.0,
};

/// Freeing a preview has to give back everything but the harness's own
/// bookkeeping, which a megabyte-scale allowance comfortably covers.
const RETAINED_BUDGET: usize = 2 * 1024 * 1024;

/// Everything one run of the pipeline observed.
#[derive(Default)]
struct Measurement {
    block_count: i64,
    block_entities: i64,
    content: [i32; 3],
    notices: Vec<String>,
    atlas: [u32; 2],
    batches: u32,
    triangles: u64,
    vertices: u64,
    payload_bytes: u64,
    /// Format version read straight out of the file, so the pair is known to
    /// straddle the versions Litematica writes rather than assumed to.
    litematic_version: i32,
    greedy_textures: HashMap<u32, u32>,
    geometry_min: [f32; 3],
    geometry_max: [f32; 3],
    open: Duration,
    mesh: Duration,
    stream: Duration,
    /// Bytes the decoded schematic holds on its own.
    decoded_peak: usize,
    /// Bytes live while streaming, which is the whole preview working set.
    peak: usize,
    /// Bytes still live once the preview is torn down.
    retained: usize,
}

impl Measurement {
    fn total(&self) -> Duration {
        self.open + self.mesh + self.stream
    }

    fn throughput_mb_per_s(&self) -> f64 {
        self.payload_bytes as f64 / 1_000_000.0 / self.stream.as_secs_f64().max(f64::MIN_POSITIVE)
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn empty_error() -> NQLError {
    NQLError {
        message: std::ptr::null_mut(),
        message_len: 0,
    }
}

/// Read and release a C-allocated message, leaving the error zeroed.
fn take_error(error: &mut NQLError) -> String {
    let text = (!error.message.is_null()).then(|| unsafe {
        String::from_utf8_lossy(std::slice::from_raw_parts(error.message, error.message_len))
            .into_owned()
    });
    if !error.message.is_null() {
        unsafe { nql_buffer_free(error.message, error.message_len) };
    }
    text.unwrap_or_default()
}

fn release(buffer: *mut u8, length: usize) {
    if !buffer.is_null() {
        unsafe { nql_buffer_free(buffer, length) };
    }
}

fn open(pack_bytes: &[u8]) -> *mut litematicaql_native::NQLResourcePack {
    let mut pack = std::ptr::null_mut();
    let mut failure = empty_error();
    let status = unsafe {
        nql_resource_pack_open(pack_bytes.as_ptr(), pack_bytes.len(), &mut pack, &mut failure)
    };
    let message = take_error(&mut failure);
    assert_eq!(status, status::OK, "{message}");
    assert!(!pack.is_null());
    pack
}

fn open_schematic(bytes: &[u8]) -> *mut litematicaql_native::NQLSchematic {
    let mut handle = std::ptr::null_mut();
    let mut failure = empty_error();
    let status =
        unsafe { nql_schematic_open(bytes.as_ptr(), bytes.len(), &mut handle, &mut failure) };
    let message = take_error(&mut failure);
    assert_eq!(status, status::OK, "{message}");
    assert!(!handle.is_null());
    handle
}

fn info(handle: *const litematicaql_native::NQLSchematic) -> NQLSchematicInfo {
    let mut info = NQLSchematicInfo {
        block_count: 0,
        block_entity_count: 0,
        content_x: 0,
        content_y: 0,
        content_z: 0,
    };
    assert_eq!(
        unsafe { nql_schematic_info(handle, &mut info) },
        status::OK
    );
    info
}

fn notices(handle: *const litematicaql_native::NQLSchematic) -> Vec<String> {
    let mut buffer = std::ptr::null_mut();
    let mut length = 0;
    assert_eq!(
        unsafe { nql_schematic_warnings(handle, &mut buffer, &mut length) },
        status::OK
    );
    if length == 0 {
        release(buffer, length);
        return Vec::new();
    }
    let text = String::from_utf8(unsafe { std::slice::from_raw_parts(buffer, length) }.to_vec())
        .expect("notices are UTF-8");
    release(buffer, length);
    text.lines().map(str::to_owned).collect()
}

fn empty_batch() -> NQLBatchInfo {
    NQLBatchInfo {
        batch_index: 0,
        part_count: 0,
        vertex_count: 0,
        index_count: 0,
        triangle_count: 0,
        payload_length: 0,
        bounds_min: [0.0; 3],
        bounds_max: [0.0; 3],
    }
}

/// Decode, mesh, and stream one build, validating every batch as it arrives.
fn run(baseline: &Baseline, pack: *const litematicaql_native::NQLResourcePack) -> Measurement {
    let path = repo_root().join(format!("Fixtures/Large/{}.litematic", baseline.name));
    let bytes = fs::read(&path).expect("large build fixture");
    let (root, _) = quartz_nbt::io::read_nbt(
        &mut std::io::Cursor::new(&bytes),
        quartz_nbt::io::Flavor::GzCompressed,
    )
    .expect("the fixture is gzipped NBT");
    let litematic_version: i32 = root
        .get("Version")
        .unwrap_or_else(|error| panic!("{}: no litematic Version tag: {error}", baseline.name));

    // Measure from a single baseline taken before the model is touched, so
    // the peak covers the decoded schematic and the meshed geometry together
    // rather than whichever phase happened to reset the marker last.
    let before = AllocationMeter::live();
    AllocationMeter::reset_peak();
    let started = Instant::now();
    let handle = open_schematic(&bytes);
    let mut measured = Measurement {
        open: started.elapsed(),
        decoded_peak: AllocationMeter::peak().saturating_sub(before),
        litematic_version,
        ..Measurement::default()
    };

    let facts = info(handle);
    measured.block_count = facts.block_count;
    measured.block_entities = facts.block_entity_count;
    measured.content = [facts.content_x, facts.content_y, facts.content_z];
    measured.notices = notices(handle);

    let mut atlas = std::ptr::null_mut();
    let mut atlas_length = 0;
    let mut atlas_info = NQLAtlasInfo {
        width: 0,
        height: 0,
    };
    let mut mesh_info = NQLMeshInfo {
        batch_count: 0,
        triangle_count: 0,
    };
    let mut stream = std::ptr::null_mut();
    let mut failure = empty_error();

    let started = Instant::now();
    let status = unsafe {
        nql_mesh_stream_open(
            handle,
            pack,
            &mut atlas,
            &mut atlas_length,
            &mut atlas_info,
            &mut mesh_info,
            &mut stream,
            &mut failure,
        )
    };
    let message = take_error(&mut failure);
    assert_eq!(status, status::OK, "{}: {message}", baseline.name);
    measured.mesh = started.elapsed();
    assert!(!stream.is_null());
    assert!(!atlas.is_null());
    assert_eq!(
        &unsafe { std::slice::from_raw_parts(atlas, atlas_length) }[..8],
        b"\x89PNG\r\n\x1a\n",
        "{}: the shared atlas is not a PNG",
        baseline.name
    );
    release(atlas, atlas_length);
    measured.atlas = [atlas_info.width, atlas_info.height];
    measured.batches = mesh_info.batch_count;

    // Texture introduction is stream-wide state, so the first batch to use a
    // greedy texture embeds its PNG and later batches reference the index
    // alone. Tracking this across the whole stream is the only way to catch a
    // texture that is referenced before the renderer has seen it.
    let mut introduced: HashMap<u32, u32> = HashMap::new();
    let mut bounds: Vec<([f32; 3], [f32; 3])> = Vec::with_capacity(mesh_info.batch_count as usize);

    AllocationMeter::reset_peak();
    let started = Instant::now();
    for expected in 0..mesh_info.batch_count {
        let mut bytes = std::ptr::null_mut();
        let mut length = 0;
        let mut batch = empty_batch();
        let mut failure = empty_error();
        let status =
            unsafe { nql_mesh_stream_next(stream, expected, &mut bytes, &mut length, &mut batch, &mut failure) };
        let message = take_error(&mut failure);
        assert_eq!(status, status::OK, "{} batch {expected}: {message}", baseline.name);
        assert!(!bytes.is_null());
        let payload = unsafe { std::slice::from_raw_parts(bytes, length) };

        check_batch(baseline, expected, &batch, payload, &mut introduced);
        accumulate(&mut measured, &batch);
        bounds.push((batch.bounds_min, batch.bounds_max));

        release(bytes, length);
    }
    measured.stream = started.elapsed();
    measured.peak = AllocationMeter::peak().saturating_sub(before);

    // The host asks for one batch past the end to learn the stream is done.
    let mut bytes = std::ptr::null_mut();
    let mut length = 1;
    let mut batch = empty_batch();
    let mut failure = empty_error();
    let status = unsafe {
        nql_mesh_stream_next(stream, mesh_info.batch_count, &mut bytes, &mut length, &mut batch, &mut failure)
    };
    take_error(&mut failure);
    assert_eq!(status, status::DONE, "{}: stream did not end", baseline.name);
    assert!(bytes.is_null());
    assert_eq!(length, 0);
    assert_eq!(batch, empty_batch());

    measured.greedy_textures = introduced;
    cover(&bounds, &mut measured);
    duplicates(baseline, &bounds);

    unsafe {
        nql_mesh_stream_free(stream);
        nql_schematic_free(handle);
    }
    // Tearing the preview down has to give the memory back, or a host that
    // opens one file after another grows without bound.
    measured.retained = AllocationMeter::live().saturating_sub(before);
    measured
}

/// Check one batch against the contract the renderer's `decodeBatch` enforces.
fn check_batch(
    baseline: &Baseline,
    expected: u32,
    batch: &NQLBatchInfo,
    payload: &[u8],
    introduced: &mut HashMap<u32, u32>,
) {
    let where_ = format!("{} batch {expected}", baseline.name);
    assert_eq!(&payload[..4], b"LQMB", "{where_}: bad batch magic");
    assert_eq!(u16::from_le_bytes(payload[4..6].try_into().unwrap()), 1, "{where_}: bad version");
    assert_eq!(u16::from_le_bytes(payload[6..8].try_into().unwrap()), 64, "{where_}: bad header size");
    assert_eq!(u32::from_le_bytes(payload[8..12].try_into().unwrap()), expected, "{where_}: wrong index");
    assert_eq!(u32::from_le_bytes(payload[12..16].try_into().unwrap()), batch.part_count, "{where_}: wrong part count");
    assert_eq!(batch.batch_index, expected, "{where_}: reported index mismatch");
    assert_eq!(batch.payload_length as usize, payload.len(), "{where_}: truncated payload");
    assert_eq!(
        batch.triangle_count as u64 * 3,
        batch.index_count as u64,
        "{where_}: triangle and index counts disagree"
    );
    assert!(batch.part_count > 0, "{where_}: empty batch");
    assert!(batch.triangle_count > 0, "{where_}: batch has no triangles");

    let mut offset = 64usize;
    let mut vertices = 0u32;
    let mut indices = 0u32;
    for part in 0..batch.part_count {
        assert!(offset + 24 <= payload.len(), "{where_} part {part}: header overruns");
        let texture = u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap());
        let part_vertices = u32::from_le_bytes(payload[offset + 8..offset + 12].try_into().unwrap())
            as usize;
        let part_indices = u32::from_le_bytes(payload[offset + 12..offset + 16].try_into().unwrap())
            as usize;
        let texture_length =
            u32::from_le_bytes(payload[offset + 16..offset + 20].try_into().unwrap()) as usize;
        let where_ = format!("{where_} part {part}");

        assert!(part_vertices > 0, "{where_}: no vertices");
        assert!(
            part_vertices <= batch.vertex_count as usize,
            "{where_}: more vertices than the batch reports"
        );
        assert!(part_indices > 0, "{where_}: no indices");
        assert_eq!(part_indices % 3, 0, "{where_}: index count is not a whole number of triangles");

        // Index 0 is the shared atlas, which the host already uploaded; every
        // other texture is greedy-meshed and has to arrive exactly once.
        if texture == 0 {
            assert_eq!(texture_length, 0, "{where_}: the shared atlas was embedded in a batch");
        } else if texture_length > 0 {
            assert!(
                introduced.insert(texture, expected).is_none(),
                "{where_}: texture {texture} was embedded twice"
            );
        } else {
            // A material contributes one part per layer, so the same texture
            // legitimately appears twice in a batch; what must hold is that
            // some earlier batch already introduced it.
            assert!(
                introduced.contains_key(&texture),
                "{where_}: texture {texture} is referenced before any batch embeds it"
            );
        }
        // A texture must be introduced by the first batch that names it, so a
        // reference in an earlier batch than its own embedding is out of order.
        if let Some(introduced_in) = introduced.get(&texture) {
            assert!(
                *introduced_in <= expected,
                "{where_}: texture {texture} was introduced by a later batch"
            );
        }

        // Indices address the part's own vertex run, never the batch's.
        let first = offset + 24 + part_vertices * 48;
        let last = first + part_indices * 4;
        assert!(last <= payload.len(), "{where_}: attributes overrun");
        for slot in payload[first..last].chunks_exact(4) {
            let value = u32::from_le_bytes(slot.try_into().unwrap()) as usize;
            assert!(value < part_vertices, "{where_}: index {value} is out of range");
        }

        vertices += part_vertices as u32;
        indices += part_indices as u32;
        offset = (last + texture_length + 3) & !3;
    }
    assert_eq!(offset, payload.len(), "{where_}: parts do not fill the payload");
    assert_eq!(vertices, batch.vertex_count, "{where_}: vertex totals disagree");
    assert_eq!(indices, batch.index_count, "{where_}: index totals disagree");
}

fn accumulate(measured: &mut Measurement, batch: &NQLBatchInfo) {
    measured.triangles += batch.triangle_count as u64;
    measured.vertices += batch.vertex_count as u64;
    measured.payload_bytes += batch.payload_length as u64;
}

/// Record the volume the batches span between them.
fn cover(bounds: &[([f32; 3], [f32; 3])], measured: &mut Measurement) {
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for (low, high) in bounds {
        for axis in 0..3 {
            min[axis] = min[axis].min(low[axis]);
            max[axis] = max[axis].max(high[axis]);
        }
    }
    measured.geometry_min = min;
    measured.geometry_max = max;
}

/// Every batch must sit on the chunk grid, and no two may overlap: an overlap
/// means the same surface was meshed twice and drawn twice.
fn duplicates(baseline: &Baseline, bounds: &[([f32; 3], [f32; 3])]) {
    for (index, (low, high)) in bounds.iter().enumerate() {
        for axis in 0..3 {
            assert_eq!(
                low[axis] % CHUNK_SIZE,
                0.0,
                "{}: batch {index} is off the chunk grid at {low:?}",
                baseline.name
            );
            let extent = high[axis] - low[axis];
            assert!(
                (0.0..=CHUNK_SIZE).contains(&extent),
                "{}: batch {index} spans {extent} blocks on axis {axis}",
                baseline.name
            );
        }
        for (other, (other_low, other_high)) in bounds.iter().enumerate().skip(index + 1) {
            let disjoint =
                (0..3).any(|axis| low[axis] >= other_high[axis] || other_low[axis] >= high[axis]);
            assert!(
                disjoint,
                "{}: batches {index} and {other} overlap",
                baseline.name
            );
        }
    }
}

/// Fail when the pipeline stops producing what this build used to produce.
fn check_geometry(baseline: &Baseline, measured: &Measurement) {
    // Between them the two fixtures cover the litematic versions Litematica
    // writes, so a swap that quietly dropped version 5 coverage fails here.
    assert_eq!(
        measured.litematic_version, baseline.litematic_version,
        "{}: the file is not litematic version {}",
        baseline.name, baseline.litematic_version
    );
    assert_eq!(
        measured.block_count, baseline.block_count,
        "{}: the decoder reported a different block count",
        baseline.name
    );
    assert_eq!(
        measured.block_entities, baseline.block_entities,
        "{}: the decoder reported a different block entity count",
        baseline.name
    );
    assert_eq!(
        measured.content, baseline.content,
        "{}: the decoder reported different dimensions",
        baseline.name
    );
    assert!(
        measured.notices.is_empty(),
        "{}: decoding reported notices: {:?}",
        baseline.name,
        measured.notices
    );
    assert_eq!(
        measured.batches, baseline.batches,
        "{}: the model was partitioned into a different number of batches",
        baseline.name
    );
    assert_eq!(
        measured.atlas,
        baseline.atlas,
        "{}: the shared atlas changed size",
        baseline.name
    );

    // Greedy textures are numbered from one in the order they are introduced,
    // so the count and the highest index are the same number.
    assert_eq!(
        measured.greedy_textures.len(),
        baseline.greedy_textures as usize,
        "{}: a different number of greedy textures was needed",
        baseline.name
    );
    assert_eq!(
        measured.greedy_textures.keys().max(),
        Some(&baseline.greedy_textures),
        "{}: greedy texture numbering is no longer dense",
        baseline.name
    );
    // The atlas cannot cover a build this size, so the first batch that needs
    // one has to carry it. If every texture were already in the atlas, the
    // fixtures would no longer be testing the greedy path.
    assert!(
        measured.greedy_textures.values().any(|batch| *batch > 0),
        "{}: no texture was introduced after the first batch",
        baseline.name
    );

    within(baseline, "triangles", measured.triangles, baseline.triangles);
    within(baseline, "payload bytes", measured.payload_bytes, baseline.payload_bytes);

    // Meshed bounds are the chunk-padded region, not the content box, and this
    // is where that difference shows: Valkyrie reports 256 blocks of content
    // height but 320 of geometry.
    assert_eq!(measured.geometry_min, [0.0; 3], "{}: geometry does not start at the origin", baseline.name);
    for axis in 0..3 {
        assert_eq!(
            measured.geometry_max[axis], baseline.geometry[axis],
            "{}: geometry extent changed on axis {axis}",
            baseline.name
        );
        assert!(
            measured.geometry_max[axis] >= measured.content[axis] as f32,
            "{}: geometry on axis {axis} is smaller than the content box",
            baseline.name
        );
    }
}

/// Allow a one percent band around a baseline, rounding away from it.
fn within(baseline: &Baseline, what: &str, measured: u64, expected: u64) {
    let slack = expected / 100 * VOLUME_TOLERANCE;
    let low = expected.saturating_sub(slack);
    let high = expected + slack;
    assert!(
        (low..=high).contains(&measured),
        "{}: {what} drifted from {expected} to {measured}, outside {low}..={high}",
        baseline.name
    );
}

fn format_report(baseline: &Baseline, measured: &Measurement) -> String {
    let peak_budget = baseline.peak_budget;
    format!(
        "\
{name}: {blocks} blocks, {entities} block entities, {content:?}
  batches {batches}, {triangles} triangles, {vertices} vertices, {payload} payload bytes
  atlas {atlas:?}, {textures} greedy textures, geometry {low:?}..{high:?}
  open {open_ms} ms, mesh {mesh_ms} ms, stream {stream_ms} ms, total {total_ms} ms
  {throughput:.1} MB/s streaming, live peak {decoded} B decoded, {mesh_peak} B streaming (budget {peak_budget} B), {retained} B retained",
        name = baseline.name,
        blocks = measured.block_count,
        entities = measured.block_entities,
        content = measured.content,
        batches = measured.batches,
        triangles = measured.triangles,
        vertices = measured.vertices,
        payload = measured.payload_bytes,
        atlas = measured.atlas,
        textures = measured.greedy_textures.len(),
        low = measured.geometry_min,
        high = measured.geometry_max,
        open_ms = measured.open.as_millis(),
        mesh_ms = measured.mesh.as_millis(),
        stream_ms = measured.stream.as_millis(),
        total_ms = measured.total().as_millis(),
        throughput = measured.throughput_mb_per_s(),
        decoded = measured.decoded_peak,
        mesh_peak = measured.peak,
        retained = measured.retained,
    )
}

#[test]
fn large_builds_decode_mesh_and_stream_within_budget() {
    let pack_bytes = fs::read(repo_root().join("Renderer/vendor/pack.zip")).expect("bundled resource pack");
    let before_pack = AllocationMeter::live();
    let pack = open(&pack_bytes);
    let pack_bytes_live = AllocationMeter::live().saturating_sub(before_pack);

    let mut report = String::new();
    for baseline in BASELINES {
        let measured = run(baseline, pack);
        check_geometry(baseline, &measured);

        // Budgets are only meaningful for optimized code: the mesher runs about
        // fifteen times slower unoptimized, which would fail every run without
        // saying anything about the change under test.
        if !cfg!(debug_assertions) {
            let text = format_report(baseline, &measured);
            report.push_str(&text);
            report.push('\n');

            assert!(
                measured.total() <= BUDGET.total,
                "{} took {} ms, over the {} ms budget\n{text}",
                baseline.name,
                measured.total().as_millis(),
                BUDGET.total.as_millis()
            );
            assert!(
                measured.throughput_mb_per_s() >= BUDGET.throughput_mb_per_s,
                "{} streamed at {:.1} MB/s, under the {:.1} MB/s budget\n{text}",
                baseline.name,
                measured.throughput_mb_per_s(),
                BUDGET.throughput_mb_per_s
            );
            let peak_budget = baseline.peak_budget;
            assert!(
                measured.peak <= peak_budget,
                "{} held {} bytes live, over the {peak_budget} byte budget\n{text}",
                baseline.name,
                measured.peak
            );
            // Allocation totals are exact, so anything left behind is a leak
            // rather than noise. A few megabytes covers the bookkeeping the
            // reporting itself allocates.
            assert!(
                measured.retained <= RETAINED_BUDGET,
                "{} kept {} bytes live after teardown\n{text}",
                baseline.name,
                measured.retained
            );
        }
    }

    unsafe { nql_resource_pack_free(pack) };

    report.push_str(&format!(
        "resource pack: {pack_bytes_live} bytes live once parsed, shared by every preview\n"
    ));
    let destination = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("large-builds.txt");
    fs::write(&destination, &report).expect("write the large build report");
    println!("{}\nwrote {}", report.trim_end(), destination.display());
}
