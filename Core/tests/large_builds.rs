//! Real-world builds at million-block scale, through the C ABI.
//!
//! The `Formats` and `Demos` fixtures are small project builds, and these
//! four are third-party or stress builds that mesh in bulk. They are the
//! only fixtures that exercise:
//!
//! - Litematic versions 5 and 6, and palettes that Minecraft itself built
//!   rather than `generate-demos.ts`.
//! - The streaming path end to end. The test validates and frees each batch
//!   before it requests the next one, as the host does. `preview.rs` keeps
//!   every batch payload in a `Vec`, so it cannot run these fixtures.
//! - A build with hundreds of greedy textures. The atlas cannot cover the
//!   model, so the first batch that uses each texture must carry its
//!   pixels.
//! - A dense palette of non-solid blocks (`DenseNonSolid`). The mesher can
//!   cull almost no hidden face and can greedy-merge almost none. The
//!   vertex volume per block is therefore about ten times the organic
//!   builds. A regression in either optimization fails here, and texture
//!   preparation takes most of the runtime.
//! - A solid interior fifteen times larger than any other fixture's block
//!   count (`Concrete`). Every hidden face is culled, so only the shell
//!   survives. This is the only build that makes the compact block store
//!   dominate memory. Its wall-clock ceiling, not a throughput floor,
//!   guards it.
//!
//! The test writes a report and asserts throughput, memory, and wall-clock
//! budgets in optimized builds. A change that doubles the geometry or keeps
//! a second copy of it still passes a draw check on a fast machine. These
//! budgets catch it.

mod support;

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use litematicaql_native::{
    nql_buffer_free, nql_mesh_batch_free, nql_mesh_stream_free, nql_mesh_stream_next,
    nql_mesh_stream_open, nql_resource_pack_free, nql_resource_pack_open, nql_schematic_free,
    nql_schematic_info, nql_schematic_open, nql_schematic_warnings, status, NQLAtlasInfo,
    NQLBatchInfo, NQLError, NQLMeshInfo, NQLSchematicInfo,
};
use support::AllocationMeter;

#[global_allocator]
static ALLOCATOR: AllocationMeter = AllocationMeter;

/// Litematic pads regions to whole chunks, and `nql_mesh_stream_open`
/// partitions the model into cubes of this size, so batch bounds sit on
/// this grid.
const CHUNK_SIZE: f32 = 64.0;

/// Recorded output that the pipeline must reproduce for one large build.
///
/// The check pins the counts from integer parsing exactly. A difference
/// means the decoder dropped or duplicated blocks, and the model shows a
/// hole or z-fighting. The check applies the [`VOLUME_TOLERANCE`] band to
/// counts from mesher floating-point output.
struct Baseline {
    name: &'static str,
    litematic_version: i32,
    block_count: i64,
    block_entities: i64,
    content: [i32; 3],
    batches: u32,
    greedy_textures: u32,
    atlas: [u32; 2],
    /// Whether the stream introduces some greedy texture after the first
    /// batch. Organic builds have spatial texture coherence, so this is the
    /// normal case. A generated stress build samples the palette uniformly,
    /// so its first batch uses every texture.
    introduces_later_textures: bool,
    triangles: u64,
    /// Total batch bytes handed to the host over the whole stream. The host
    /// releases each batch before the next request, so the peak live bytes
    /// stay far below this total. A change that closes that gap makes the
    /// host hold gigabytes.
    payload_bytes: u64,
    /// Extent of the union of every batch bound.
    geometry: [f32; 3],
    /// Ceiling on peak live bytes while streaming. It is the recorded peak
    /// plus a quarter, with room for drift in the count of chunks that the
    /// mesher processes at once. The value is absolute, not a ratio, because
    /// integer division rounds `5 / 4` down to one.
    peak_budget: usize,
    /// Wall-clock ceiling for one build. The ceiling is per build, because
    /// the ratio of block count to surviving geometry differs greatly across
    /// the fixtures.
    total_budget: Duration,
    /// Floor on throughput, in millions of triangles per second. The floor
    /// does not use bytes per second, because the compact vertex layout
    /// stores fewer bytes per triangle. A build with a culled interior
    /// leaves almost no triangles per block, so its floor is zero and the
    /// wall-clock ceiling above guards it.
    million_triangles_per_s: f64,
}

/// Drift allowed on the counts that the mesher produces, in hundredths of a
/// percent. A small difference in vertex packing does not fail the run.
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
        introduces_later_textures: true,
        triangles: 10_705_456,
        payload_bytes: 1_156_261_144,
        geometry: [704.0, 256.0, 576.0],
        peak_budget: 137_560_888,
        total_budget: Duration::from_secs(45),
        million_triangles_per_s: 0.4,
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
        introduces_later_textures: true,
        triangles: 4_937_416,
        payload_bytes: 533_333_244,
        geometry: [1024.0, 320.0, 1024.0],
        peak_budget: 129_110_885,
        total_budget: Duration::from_secs(45),
        million_triangles_per_s: 0.4,
    },
    Baseline {
        name: "DenseNonSolid",
        litematic_version: 6,
        block_count: 912_632,
        block_entities: 0,
        content: [97, 97, 97],
        batches: 8,
        greedy_textures: 458,
        atlas: [2048, 2048],
        introduces_later_textures: false,
        triangles: 28_667_554,
        // Payload recorded under the current 24-byte vertex layout.
        payload_bytes: 3_438_230_290,
        geometry: [128.0, 128.0, 128.0],
        peak_budget: 3_615_043_178,
        total_budget: Duration::from_secs(45),
        million_triangles_per_s: 0.4,
    },
    Baseline {
        name: "Concrete",
        litematic_version: 6,
        block_count: 67_108_864,
        block_entities: 0,
        content: [256, 256, 1024],
        batches: 256,
        greedy_textures: 16,
        atlas: [128, 128],
        introduces_later_textures: false,
        triangles: 2_097_350,
        // Payload recorded under the current 24-byte vertex layout.
        payload_bytes: 200_000_000,
        geometry: [256.0, 256.0, 1024.0],
        peak_budget: 1_200_000_000,
        // Sixty-seven million blocks cull to a shell, about one triangle per
        // thirty-two blocks. A throughput floor low enough for this build
        // does not catch a regression.
        total_budget: Duration::from_secs(45),
        million_triangles_per_s: 0.0,
    },
];

/// Teardown must free everything but the bookkeeping of the test harness.
/// This allowance covers that bookkeeping.
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
    /// Litematic version read from the file, so the check confirms the
    /// fixture version instead of assuming it.
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

    fn million_triangles_per_s(&self) -> f64 {
        self.triangles as f64 / 1_000_000.0 / self.stream.as_secs_f64().max(f64::MIN_POSITIVE)
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

/// Read and free the C message of the error. The freed pointer stays in
/// place, so the caller must not use the error again.
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
        nql_resource_pack_open(
            pack_bytes.as_ptr(),
            pack_bytes.len(),
            &mut pack,
            &mut failure,
        )
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
        content_min_x: 0,
        content_min_y: 0,
        content_min_z: 0,
    };
    assert_eq!(unsafe { nql_schematic_info(handle, &mut info) }, status::OK);
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

/// Decode, mesh, and stream one build. Check each batch as it arrives.
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

    // The harness records one allocation baseline before the decode. Each
    // peak figure subtracts it, so the figures exclude the fixture bytes
    // that the harness holds.
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
        atlas_length,
        atlas_info.width as usize * atlas_info.height as usize * 4
    );
    release(atlas, atlas_length);
    measured.atlas = [atlas_info.width, atlas_info.height];
    measured.batches = mesh_info.batch_count;

    // Texture introduction is stream-wide state: the first batch that uses
    // a greedy texture carries its RGBA pixels, and later batches reference
    // the index alone. The map therefore catches a texture used before the
    // renderer sees it.
    let mut introduced: HashMap<u32, u32> = HashMap::new();
    let mut bounds: Vec<([f32; 3], [f32; 3])> = Vec::with_capacity(mesh_info.batch_count as usize);

    AllocationMeter::reset_peak();
    let started = Instant::now();
    for expected in 0..mesh_info.batch_count {
        let mut bytes = std::ptr::null_mut();
        let mut batch = empty_batch();
        let mut failure = empty_error();
        let status =
            unsafe { nql_mesh_stream_next(stream, expected, &mut bytes, &mut batch, &mut failure) };
        let message = take_error(&mut failure);
        assert_eq!(
            status,
            status::OK,
            "{} batch {expected}: {message}",
            baseline.name
        );
        assert!(!bytes.is_null());
        let payload = unsafe { (*bytes).view() };

        check_batch(baseline, expected, &batch, &payload, &mut introduced);
        accumulate(&mut measured, &batch);
        bounds.push((batch.bounds_min, batch.bounds_max));

        unsafe { nql_mesh_batch_free(bytes) };
    }
    measured.stream = started.elapsed();
    measured.peak = AllocationMeter::peak().saturating_sub(before);

    // The host asks for one batch past the end to learn that the stream is
    // done.
    let mut bytes = std::ptr::null_mut();
    let mut batch = empty_batch();
    let mut failure = empty_error();
    let status = unsafe {
        nql_mesh_stream_next(
            stream,
            mesh_info.batch_count,
            &mut bytes,
            &mut batch,
            &mut failure,
        )
    };
    take_error(&mut failure);
    assert_eq!(
        status,
        status::DONE,
        "{}: stream did not end",
        baseline.name
    );
    assert!(bytes.is_null());
    assert_eq!(batch, empty_batch());

    measured.greedy_textures = introduced;
    cover(&bounds, &mut measured);
    duplicates(baseline, &bounds);

    unsafe {
        nql_mesh_stream_free(stream);
        nql_schematic_free(handle);
    }
    // Teardown must give the memory back, or a host that opens one file
    // after another grows without bound.
    measured.retained = AllocationMeter::live().saturating_sub(before);
    measured
}

/// Check the native Metal buffers as each batch arrives, and store none of
/// them.
fn check_batch(
    baseline: &Baseline,
    expected: u32,
    batch: &NQLBatchInfo,
    view: &litematicaql_native::stream::NQLBatchView,
    introduced: &mut HashMap<u32, u32>,
) {
    let where_ = format!("{} batch {expected}", baseline.name);
    assert_eq!(
        batch.batch_index, expected,
        "{where_}: reported index mismatch"
    );
    assert_eq!(batch.triangle_count as u64 * 3, batch.index_count as u64);
    let parts = unsafe { std::slice::from_raw_parts(view.parts, view.part_count) };
    let geometry = unsafe { std::slice::from_raw_parts(view.geometry, view.geometry_length) };
    assert_eq!(parts.len(), batch.part_count as usize);
    let mut vertices = 0;
    let mut indices = 0;
    let mut referenced: Vec<u32> = Vec::new();
    for part in parts {
        assert!(
            part.vertex_count > 0 && part.index_count > 0,
            "{where_}: empty part"
        );
        assert_eq!(part.index_count % 3, 0);
        assert!(matches!(part.index_size, 2 | 4));
        assert!(part.vertex_offset + part.vertex_count as usize * 24 <= geometry.len());
        // The stream delivers the atlas once at open, so no batch embeds it.
        if part.texture_index != 0 {
            referenced.push(part.texture_index);
        }
        let end = part.index_offset + part.index_count as usize * part.index_size as usize;
        assert!(end <= geometry.len());
        for index in geometry[part.index_offset..end].chunks_exact(part.index_size as usize) {
            let index = if part.index_size == 2 {
                u16::from_le_bytes(index.try_into().unwrap()) as u32
            } else {
                u32::from_le_bytes(index.try_into().unwrap())
            };
            assert!(index < part.vertex_count, "{where_}: index out of bounds");
        }
        vertices += part.vertex_count;
        indices += part.index_count;
    }
    let mut texture_bytes = 0;
    for texture in unsafe { std::slice::from_raw_parts(view.textures, view.texture_count) } {
        assert!(texture.index != 0, "{where_}: duplicate atlas");
        assert!(texture.repeats, "{where_}: clamped texture in a batch");
        assert_eq!(
            texture.length,
            texture.width as usize * texture.height as usize * 4,
            "{where_}: texture {} is not RGBA",
            texture.index
        );
        assert!(
            introduced.insert(texture.index, expected).is_none(),
            "{where_}: duplicate texture"
        );
        texture_bytes += texture.length;
    }
    for index in referenced {
        assert!(
            introduced.contains_key(&index),
            "{where_}: texture used before introduction"
        );
    }
    assert_eq!(vertices, batch.vertex_count);
    assert_eq!(indices, batch.index_count);
    assert_eq!(
        geometry.len() + texture_bytes,
        batch.payload_length as usize
    );
}

fn accumulate(measured: &mut Measurement, batch: &NQLBatchInfo) {
    measured.triangles += batch.triangle_count as u64;
    measured.vertices += batch.vertex_count as u64;
    measured.payload_bytes += batch.payload_length as u64;
}

/// Record the union of all batch bounds.
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

/// Every batch must sit on the chunk grid and span at most one chunk per
/// axis. No two batches overlap. A misaligned or oversized batch duplicates
/// or drops geometry. An overlap draws the same surface twice.
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

/// Compare a measured build with its baseline record. Each difference fails
/// the check.
fn check_geometry(baseline: &Baseline, measured: &Measurement) {
    // Together the fixtures cover every litematic version that Litematica
    // writes. A change that drops version 5 coverage therefore fails this
    // check.
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
        measured.atlas, baseline.atlas,
        "{}: the shared atlas changed size",
        baseline.name
    );

    // Greedy texture indices start at one and follow introduction order, so
    // the count and the highest index are the same number.
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
    // The atlas cannot cover a build of this size, so the first batch that
    // needs a texture must carry it. A build that samples its palette
    // uniformly introduces every texture in its first batch, so this check
    // applies to the organic fixtures only.
    if baseline.introduces_later_textures {
        assert!(
            measured.greedy_textures.values().any(|batch| *batch > 0),
            "{}: no texture was introduced after the first batch",
            baseline.name
        );
    }

    within(
        baseline,
        "triangles",
        measured.triangles,
        baseline.triangles,
    );
    // Compact vertices and variable-width indices reduce the payload bytes,
    // so the ceiling is two thirds of the baseline payload. A baseline
    // recorded under the current 24-byte layout pins twice its measured
    // payload, so the ceiling leaves room for growth.
    assert!(measured.payload_bytes < baseline.payload_bytes * 2 / 3);

    // Meshed bounds are the chunk-padded region, not the content box.
    // Valkyrie shows the difference: 256 blocks of content height, 320
    // blocks of geometry.
    assert_eq!(
        measured.geometry_min, [0.0; 3],
        "{}: geometry does not start at the origin",
        baseline.name
    );
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

/// Check one measured count against the [`VOLUME_TOLERANCE`] band around
/// the baseline. The band rounds away from the baseline.
fn within(baseline: &Baseline, what: &str, measured: u64, expected: u64) {
    let slack = (expected * VOLUME_TOLERANCE).div_ceil(10_000);
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
  {throughput:.1} M triangles/s streaming, live peak {decoded} B decoded, {mesh_peak} B streaming (budget {peak_budget} B), {retained} B retained",
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
        throughput = measured.million_triangles_per_s(),
        decoded = measured.decoded_peak,
        mesh_peak = measured.peak,
        retained = measured.retained,
    )
}

#[test]
fn large_builds_decode_mesh_and_stream_within_budget() {
    let pack_bytes =
        fs::read(repo_root().join("Resources/pack.zip")).expect("bundled resource pack");
    let before_pack = AllocationMeter::live();
    let pack = open(&pack_bytes);
    let pack_bytes_live = AllocationMeter::live().saturating_sub(before_pack);

    let mut report = String::new();
    for baseline in BASELINES {
        let measured = run(baseline, pack);
        check_geometry(baseline, &measured);

        // The budget assertions run only in optimized builds. Unoptimized
        // code runs the mesher about fifteen times slower, so every budget
        // fails there. Such a failure says nothing about the change under
        // test.
        if !cfg!(debug_assertions) {
            let text = format_report(baseline, &measured);
            report.push_str(&text);
            report.push('\n');

            assert!(
                measured.total() <= baseline.total_budget,
                "{} took {} ms, over the {} ms budget\n{text}",
                baseline.name,
                measured.total().as_millis(),
                baseline.total_budget.as_millis()
            );
            assert!(
                measured.million_triangles_per_s() >= baseline.million_triangles_per_s,
                "{} streamed at {:.1} M triangles/s, under the {:.1} M triangles/s budget\n{text}",
                baseline.name,
                measured.million_triangles_per_s(),
                baseline.million_triangles_per_s
            );
            let peak_budget = baseline.peak_budget;
            assert!(
                measured.peak <= peak_budget,
                "{} held {} bytes live, over the {peak_budget} byte budget\n{text}",
                baseline.name,
                measured.peak
            );
            // Allocation totals are exact, so anything left behind is a leak.
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
