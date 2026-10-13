//! C ABI and native ownership adapter over litematica-preview-core.
use litematica_preview_core::{DecodeFailure, DecodedSchematic, PreviewOptions};
use nucleation::formats::limits::DecodeLimits;
use nucleation::meshing::{MeshConfig, ResourcePackSource};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub mod stream;

// Preview limits. One metric per orthogonal attribution.
const MAX_DECOMPRESSED_BYTES: usize = 4 * 1_024 * 1_024 * 1_024;
const MAX_AXIS_LENGTH: usize = 8_192;
const MAX_VOLUME: usize = 2048 * 2048 * 2048;
const MAX_REGIONS: usize = 64;
const MAX_PALETTE_ENTRIES: usize = 32_768;
const MAX_NBT_DEPTH: usize = 64;
const MAX_NBT_NODES: usize = 4_194_304;
const MAX_MESH_BLOCKS: i64 = 512 * 512 * 512;
/// Preview ceiling on one side of the shared texture atlas, in texels. Only
/// the ceiling: the builder grows an atlas to what a palette needs, so builds
/// with fewer distinct tiles keep the smaller atlases they have always had.
const MAX_ATLAS_SIDE: u32 = 2048;
/// Override ceiling on the same atlas, reached only after the user agreed to
/// the graphics-memory cost. 8192² of RGBA is 256 MB of host pixels while the
/// tiles are placed, on top of the atlas the device then keeps.
const OVERRIDE_ATLAS_SIDE: u32 = 8192;

// Status codes shared with the Swift bridge through the C header.
pub mod status {
    pub const OK: i32 = 0;
    pub const ERR_NULL: i32 = 1;
    pub const ERR_FORMAT: i32 = 2;
    pub const ERR_LIMIT: i32 = 3;
    pub const ERR_NO_BLOCKS: i32 = 4;
    pub const ERR_MESH: i32 = 5;
    pub const ERR_PACK: i32 = 6;
    pub const ERR_INTERNAL: i32 = 7;
    pub const ERR_CANCELLED: i32 = 8;
    pub const DONE: i32 = 9;
}

/// An application-owned decoded preview. Opaque to C callers.
pub struct NQLSchematic {
    source: DecodedSchematic,
    cancelled: Arc<AtomicBool>,
}

/// A parsed immutable resource pack that may be reused by sequential previews.
pub struct NQLResourcePack(ResourcePackSource);

#[repr(C)]
pub struct NQLSchematicInfo {
    pub block_count: i64,
    pub block_entity_count: i64,
    pub content_x: i32,
    pub content_y: i32,
    pub content_z: i32,
    pub content_min_x: i32,
    pub content_min_y: i32,
    pub content_min_z: i32,
}

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub struct NQLMeshInfo {
    pub batch_count: u32,
    pub triangle_count: i64,
}

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub struct NQLAtlasInfo {
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq)]
pub struct NQLBatchInfo {
    pub batch_index: u32,
    pub part_count: u32,
    pub vertex_count: u32,
    pub index_count: u32,
    pub triangle_count: u32,
    pub payload_length: u32,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
}

// A UTF-8 failure message. The host frees `message` with `nql_buffer_free`.
#[repr(C)]
pub struct NQLError {
    pub message: *mut u8,
    pub message_len: usize,
}

fn preview_limits() -> DecodeLimits {
    DecodeLimits {
        // Input size is the host's file acceptance check, not a decode quota.
        max_input_bytes: usize::MAX,
        max_decompressed_bytes: MAX_DECOMPRESSED_BYTES,
        max_dimension: MAX_AXIS_LENGTH,
        max_volume: MAX_VOLUME,
        max_regions: MAX_REGIONS,
        max_palette_entries: MAX_PALETTE_ENTRIES,
        // Entities and block entities are top-level collections inside the NBT
        // node budget above; string payloads and collection items likewise.
        max_entities: usize::MAX,
        max_block_entities: usize::MAX,
        max_nbt_depth: MAX_NBT_DEPTH,
        max_nbt_string_bytes: usize::MAX,
        max_nbt_collection_items: usize::MAX,
        max_nbt_nodes: MAX_NBT_NODES,
    }
}

/// No preview quotas: only the formats' own width limits stand between the
/// readers and the file's content. Reached solely from the override entry
/// points after the user asked to render despite a limit hit, so the caller
/// owns everything this admits.
fn override_limits() -> DecodeLimits {
    DecodeLimits {
        max_input_bytes: usize::MAX,
        max_decompressed_bytes: usize::MAX,
        max_dimension: usize::MAX,
        max_volume: usize::MAX,
        max_regions: usize::MAX,
        max_palette_entries: usize::MAX,
        max_entities: usize::MAX,
        max_block_entities: usize::MAX,
        max_nbt_depth: usize::MAX,
        max_nbt_string_bytes: usize::MAX,
        max_nbt_collection_items: usize::MAX,
        max_nbt_nodes: usize::MAX,
    }
}

/// Mesher configuration, with the atlas ceiling the caller chose.
///
/// `litematica_preview_core::mesh_config()` fixes the atlas ceiling at 2048
/// texels and takes no argument, so the preview's two mesh-stage quotas are
/// expressed here instead of being limited to whichever ceiling upstream
/// happens to prefer.
fn mesh_config(atlas_max_side: u32) -> MeshConfig {
    MeshConfig::new()
        .with_greedy_meshing(true)
        .with_ambient_occlusion(true)
        .with_atlas_max_size(atlas_max_side)
}

/// Mesh-stage quotas. One metric per orthogonal attribution: how many blocks
/// the stream may describe, and how much tile area their textures may occupy.
/// Both are preview quotas that the override entry lifts.
#[derive(Clone, Copy)]
pub(crate) struct MeshQuotas {
    pub(crate) block_budget: i64,
    pub(crate) atlas_max_side: u32,
}

/// Which of the two quota sets a mesh stream is being opened at.
///
/// The preview tier protects a preview from a palette whose tiles do not fit
/// the shared atlas. The override tier is what the preview tier's refusal buys
/// with the user's agreement, and levies nothing more.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MeshTier {
    Preview,
    Override,
}

impl MeshTier {
    fn quotas(self) -> MeshQuotas {
        match self {
            MeshTier::Preview => MeshQuotas {
                block_budget: MAX_MESH_BLOCKS,
                atlas_max_side: MAX_ATLAS_SIDE,
            },
            MeshTier::Override => MeshQuotas {
                block_budget: i64::MAX,
                atlas_max_side: OVERRIDE_ATLAS_SIDE,
            },
        }
    }
}

/// Meshing concurrency. Each worker meshes one chunk batch and every finished
/// batch is held whole until the host pulls it, so this cap is also the
/// preview's peak-geometry window: a deeper window bought no measurable
/// streaming throughput while holding another full chunk batch live, which
/// `Core/tests/large_builds.rs` pins.
fn worker_count() -> usize {
    std::thread::available_parallelism().map_or(2, |count| count.get().min(2).max(1))
}

fn decode_worker_count() -> usize {
    std::thread::available_parallelism().map_or(2, |count| count.get().min(2).max(1))
}

#[derive(Debug)]
pub enum MeshFailure {
    Pack(String),
    NoBlocks,
    /// A preview quota refused the mesh; recoverable by an override open.
    Limit(String),
    Mesh(String),
    Cancelled,
}

fn decode(bytes: &[u8], limits: &DecodeLimits) -> Result<NQLSchematic, DecodeFailure> {
    let workers = decode_worker_count();
    let thread_count = (workers > 1).then_some(workers as u8);
    let source = litematica_preview_core::decode_with_options(
        bytes,
        limits,
        PreviewOptions {
            chunk_size: Some(64),
            thread_count,
            speed_first: thread_count.is_some(),
            ..PreviewOptions::default()
        },
        || Ok(()),
    )?;
    Ok(NQLSchematic {
        source,
        cancelled: Arc::new(AtomicBool::new(false)),
    })
}

fn resource_pack(bytes: &[u8]) -> Result<NQLResourcePack, MeshFailure> {
    ResourcePackSource::from_bytes(bytes)
        .map(NQLResourcePack)
        .map_err(|error| MeshFailure::Pack(error.to_string()))
}

fn resource_pack_overlay(
    base: &NQLResourcePack,
    bytes: &[u8],
) -> Result<NQLResourcePack, MeshFailure> {
    litematica_preview_core::resource_pack_overlay(&base.0, bytes)
        .map(NQLResourcePack)
        .map_err(MeshFailure::Pack)
}

fn export_bytes(bytes: Vec<u8>) -> (*mut u8, usize) {
    let mut bytes = bytes;
    bytes.shrink_to_fit();
    let boxed = bytes.into_boxed_slice();
    let len = boxed.len();
    (Box::into_raw(boxed) as *mut u8, len)
}

unsafe fn fail(err_out: *mut NQLError, code: i32, message: &str) -> i32 {
    if !err_out.is_null() {
        let (message, message_len) = export_bytes(message.as_bytes().to_vec());
        *err_out = NQLError {
            message,
            message_len,
        };
    }
    code
}

unsafe fn init_error(err_out: *mut NQLError) {
    if !err_out.is_null() {
        *err_out = NQLError {
            message: std::ptr::null_mut(),
            message_len: 0,
        };
    }
}

unsafe fn open_schematic(
    data: *const u8,
    len: usize,
    out: *mut *mut NQLSchematic,
    err_out: *mut NQLError,
    limits: &DecodeLimits,
) -> i32 {
    init_error(err_out);
    if data.is_null() || out.is_null() {
        return fail(
            err_out,
            status::ERR_NULL,
            "The caller passed a null buffer.",
        );
    }
    *out = std::ptr::null_mut();
    let bytes = slice::from_raw_parts(data, len);
    match catch_unwind(AssertUnwindSafe(|| decode(bytes, limits))) {
        Ok(Ok(schematic)) => {
            *out = Box::into_raw(Box::new(schematic));
            status::OK
        }
        Ok(Err(DecodeFailure::Format(message))) => fail(err_out, status::ERR_FORMAT, &message),
        Ok(Err(DecodeFailure::Limit(message))) => {
            fail(err_out, status::ERR_LIMIT, &describe_limit(&message))
        }
        Err(_) => fail(
            err_out,
            status::ERR_INTERNAL,
            "Something went wrong while reading this schematic.",
        ),
    }
}

#[no_mangle]
pub unsafe extern "C" fn nql_schematic_open(
    data: *const u8,
    len: usize,
    out: *mut *mut NQLSchematic,
    err_out: *mut NQLError,
) -> i32 {
    open_schematic(data, len, out, err_out, &preview_limits())
}

/// Reads a schematic with no preview quotas, for schematics whose limit hit
/// the caller has already surfaced to the user with an offer to render
/// anyway. Every admission decision moves to the caller from here on.
#[no_mangle]
pub unsafe extern "C" fn nql_schematic_open_override(
    data: *const u8,
    len: usize,
    out: *mut *mut NQLSchematic,
    err_out: *mut NQLError,
) -> i32 {
    open_schematic(data, len, out, err_out, &override_limits())
}

#[no_mangle]
pub unsafe extern "C" fn nql_schematic_free(schematic: *mut NQLSchematic) {
    if !schematic.is_null() {
        drop(Box::from_raw(schematic));
    }
}

/// Cooperatively cancel decoding-derived meshing while retaining the handle.
/// Cancellation is monotonic; a cancelled handle cannot publish later geometry.
#[no_mangle]
pub unsafe extern "C" fn nql_schematic_cancel(schematic: *const NQLSchematic) -> i32 {
    if schematic.is_null() {
        return status::ERR_NULL;
    }
    (*schematic).cancelled.store(true, Ordering::Relaxed);
    status::OK
}

#[no_mangle]
pub unsafe extern "C" fn nql_schematic_warnings(
    schematic: *const NQLSchematic,
    out: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if schematic.is_null() || out.is_null() || out_len.is_null() {
        return status::ERR_NULL;
    }
    let (buffer, len) = export_bytes((*schematic).source.warnings().join("\n").into_bytes());
    *out = buffer;
    *out_len = len;
    status::OK
}

#[no_mangle]
pub unsafe extern "C" fn nql_schematic_info(
    schematic: *const NQLSchematic,
    out: *mut NQLSchematicInfo,
) -> i32 {
    if schematic.is_null() || out.is_null() {
        return status::ERR_NULL;
    }
    let source = &(*schematic).source;
    let content = source.content_dimensions();
    let origin = source.content_origin();
    *out = NQLSchematicInfo {
        block_count: source.block_count(),
        block_entity_count: source.block_entity_count(),
        content_x: content.0,
        content_y: content.1,
        content_z: content.2,
        content_min_x: origin.0,
        content_min_y: origin.1,
        content_min_z: origin.2,
    };
    status::OK
}

#[no_mangle]
pub unsafe extern "C" fn nql_resource_pack_open(
    data: *const u8,
    len: usize,
    out: *mut *mut NQLResourcePack,
    err_out: *mut NQLError,
) -> i32 {
    init_error(err_out);
    if data.is_null() || out.is_null() {
        return fail(
            err_out,
            status::ERR_NULL,
            "The caller passed a null buffer.",
        );
    }
    *out = std::ptr::null_mut();
    let bytes = slice::from_raw_parts(data, len);
    match catch_unwind(AssertUnwindSafe(|| resource_pack(bytes))) {
        Ok(Ok(pack)) => {
            *out = Box::into_raw(Box::new(pack));
            status::OK
        }
        Ok(Err(MeshFailure::Pack(error))) => fail(
            err_out,
            status::ERR_PACK,
            &format!("The bundled block resources are invalid: {error}"),
        ),
        Ok(Err(_)) => fail(
            err_out,
            status::ERR_INTERNAL,
            "Something went wrong while reading the bundled block resources.",
        ),
        Err(_) => fail(
            err_out,
            status::ERR_INTERNAL,
            "Something went wrong while reading the bundled block resources.",
        ),
    }
}

#[no_mangle]
pub unsafe extern "C" fn nql_resource_pack_free(pack: *mut NQLResourcePack) {
    if !pack.is_null() {
        drop(Box::from_raw(pack));
    }
}

#[no_mangle]
pub unsafe extern "C" fn nql_resource_pack_overlay(
    base: *const NQLResourcePack,
    data: *const u8,
    len: usize,
    out: *mut *mut NQLResourcePack,
    err_out: *mut NQLError,
) -> i32 {
    init_error(err_out);
    if base.is_null() || data.is_null() || out.is_null() {
        return fail(
            err_out,
            status::ERR_NULL,
            "The caller passed a null buffer.",
        );
    }
    *out = std::ptr::null_mut();
    match catch_unwind(AssertUnwindSafe(|| {
        resource_pack_overlay(&*base, slice::from_raw_parts(data, len))
    })) {
        Ok(Ok(pack)) => {
            *out = Box::into_raw(Box::new(pack));
            status::OK
        }
        Ok(Err(MeshFailure::Pack(error))) => fail(err_out, status::ERR_PACK, &error),
        _ => fail(
            err_out,
            status::ERR_INTERNAL,
            "Unable to read this texture pack. Try another Minecraft Java resource pack.",
        ),
    }
}

unsafe fn start_mesh_stream(
    schematic: *const NQLSchematic,
    pack: *const NQLResourcePack,
    atlas_rgba_out: *mut *mut u8,
    atlas_rgba_len: *mut usize,
    atlas_info_out: *mut NQLAtlasInfo,
    info_out: *mut NQLMeshInfo,
    stream_out: *mut *mut stream::NQLMeshStream<'_>,
    err_out: *mut NQLError,
    tier: MeshTier,
) -> i32 {
    init_error(err_out);
    if schematic.is_null()
        || pack.is_null()
        || atlas_rgba_out.is_null()
        || atlas_rgba_len.is_null()
        || stream_out.is_null()
    {
        return fail(
            err_out,
            status::ERR_NULL,
            "The caller passed a null buffer.",
        );
    }
    *atlas_rgba_out = std::ptr::null_mut();
    *atlas_rgba_len = 0;
    *stream_out = std::ptr::null_mut();
    let cancelled = (*schematic).cancelled.clone();
    let current = || {
        if cancelled.load(Ordering::Relaxed) {
            Err("Preview cancelled.".to_string())
        } else {
            Ok(())
        }
    };
    let opened = match catch_unwind(AssertUnwindSafe(|| {
        stream::NQLMeshStream::open(&*schematic, &*pack, &current, tier.quotas())
    })) {
        Ok(Ok(opened)) => opened,
        Ok(Err(error)) => {
            if tier != MeshTier::Preview {
                return mesh_failure(err_out, error);
            }
            // The atlas ceiling is the last quota the preview tier can lift, so
            // the probe moves exactly that field and leaves the block budget
            // where the preview tier put it. A different outcome can then only
            // be the ceiling: the tiles did not fit, the host has a quota to
            // offer, and the override atlas's side states the price. Its stream
            // is dropped again, so nothing is built at that cost until the user
            // has agreed to it. Every other refusal — the block budget, a
            // genuinely unreadable build — fails the probe too, because the
            // probe raises nothing but the ceiling, and so keeps its own
            // status and message.
            let atlas_probe = MeshQuotas {
                block_budget: tier.quotas().block_budget,
                atlas_max_side: OVERRIDE_ATLAS_SIDE,
            };
            let probe = catch_unwind(AssertUnwindSafe(|| {
                stream::NQLMeshStream::open(&*schematic, &*pack, &current, atlas_probe)
            }));
            return match probe {
                Ok(Ok((_, _, probe_info, _))) => {
                    let cost = u64::from(probe_info.width) * u64::from(probe_info.height) * 4;
                    fail(
                        err_out,
                        status::ERR_LIMIT,
                        &format!(
                            "This schematic's blocks need a {} × {} texture atlas of \
                             distinct block tiles. The preview limit is {} × {}, so \
                             rendering it anyway needs {} of graphics memory.",
                            grouped(probe_info.width as u64),
                            grouped(probe_info.height as u64),
                            grouped(u64::from(MAX_ATLAS_SIDE)),
                            grouped(u64::from(MAX_ATLAS_SIDE)),
                            ByteCount(cost)
                        ),
                    )
                }
                _ => mesh_failure(err_out, error),
            };
        }
        Err(_) => {
            return fail(
                err_out,
                status::ERR_MESH,
                "This schematic has too much visible surface to preview. Its block geometry exceeds what the renderer can build.",
            )
        }
    };
    let (stream, atlas, atlas_info, info) = opened;
    let (data, len) = export_bytes(atlas);
    *atlas_rgba_out = data;
    *atlas_rgba_len = len;
    if !atlas_info_out.is_null() {
        *atlas_info_out = atlas_info;
    }
    if !info_out.is_null() {
        *info_out = info;
    }
    *stream_out = Box::into_raw(Box::new(stream));
    status::OK
}

#[no_mangle]
pub unsafe extern "C" fn nql_mesh_stream_open(
    schematic: *const NQLSchematic,
    pack: *const NQLResourcePack,
    atlas_rgba_out: *mut *mut u8,
    atlas_rgba_len: *mut usize,
    atlas_info_out: *mut NQLAtlasInfo,
    info_out: *mut NQLMeshInfo,
    stream_out: *mut *mut stream::NQLMeshStream<'_>,
    err_out: *mut NQLError,
) -> i32 {
    start_mesh_stream(
        schematic,
        pack,
        atlas_rgba_out,
        atlas_rgba_len,
        atlas_info_out,
        info_out,
        stream_out,
        err_out,
        MeshTier::Preview,
    )
}

/// Lifts the render-stage block budget and the shared texture atlas's ceiling,
/// for schematics whose quota the caller has already surfaced to the user with
/// an offer to render anyway. The notice that earns the offer stays the
/// caller's to state before asking.
#[no_mangle]
pub unsafe extern "C" fn nql_mesh_stream_open_override(
    schematic: *const NQLSchematic,
    pack: *const NQLResourcePack,
    atlas_rgba_out: *mut *mut u8,
    atlas_rgba_len: *mut usize,
    atlas_info_out: *mut NQLAtlasInfo,
    info_out: *mut NQLMeshInfo,
    stream_out: *mut *mut stream::NQLMeshStream<'_>,
    err_out: *mut NQLError,
) -> i32 {
    start_mesh_stream(
        schematic,
        pack,
        atlas_rgba_out,
        atlas_rgba_len,
        atlas_info_out,
        info_out,
        stream_out,
        err_out,
        MeshTier::Override,
    )
}

/// The stream borrows schematic and pack; both must outlive it. A returned
/// batch owns its storage independently and remains valid until batch_free.
#[no_mangle]
pub unsafe extern "C" fn nql_mesh_stream_next(
    stream: *mut stream::NQLMeshStream<'_>,
    expected_batch: u32,
    batch_out: *mut *mut stream::NQLMeshBatch,
    info_out: *mut NQLBatchInfo,
    err_out: *mut NQLError,
) -> i32 {
    init_error(err_out);
    if stream.is_null() || batch_out.is_null() {
        return fail(
            err_out,
            status::ERR_NULL,
            "The caller passed a null buffer.",
        );
    }
    *batch_out = std::ptr::null_mut();
    if !info_out.is_null() {
        std::ptr::write_bytes(info_out, 0, 1);
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| (*stream).next(expected_batch)));
    match outcome {
        Err(_) => fail(
            err_out,
            status::ERR_MESH,
            "This schematic is too detailed to preview.",
        ),
        Ok(Err(error)) => mesh_failure(err_out, error),
        Ok(Ok(None)) => status::DONE,
        Ok(Ok(Some(batch))) => {
            if !info_out.is_null() {
                *info_out = batch.info.clone();
            }
            *batch_out = Box::into_raw(Box::new(batch));
            status::OK
        }
    }
}

/// All pointers in this view borrow the batch, and are read-only.
#[no_mangle]
pub unsafe extern "C" fn nql_mesh_batch_view(
    batch: *const stream::NQLMeshBatch,
) -> stream::NQLBatchView {
    (*batch).view()
}

#[no_mangle]
pub unsafe extern "C" fn nql_mesh_batch_free(batch: *mut stream::NQLMeshBatch) {
    if !batch.is_null() {
        drop(Box::from_raw(batch));
    }
}

#[no_mangle]
pub unsafe extern "C" fn nql_mesh_stream_free(stream: *mut stream::NQLMeshStream<'_>) {
    if !stream.is_null() {
        drop(Box::from_raw(stream));
    }
}

/// Thousands separators for the numbers describe_limit surfaces.
pub(crate) fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// A byte count in binary units, for the atlas price an override offer states.
/// The host renders its own size strings elsewhere; this one only has to say
/// how much graphics memory the atlas costs.
struct ByteCount(u64);

impl std::fmt::Display for ByteCount {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
        let mut value = self.0 as f64;
        let mut unit = 0;
        while value >= 1024.0 && unit + 1 < UNITS.len() {
            value /= 1024.0;
            unit += 1;
        }
        let text = if value >= 100.0 || unit == 0 {
            format!("{}", value as u64)
        } else {
            format!("{value:.1}")
        };
        write!(formatter, "{text} {}", UNITS[unit])
    }
}

/// Names the dimension a quota broke and the exact number, so hosts can show
/// figures instead of the engine's terse refusal. Anything unrecognized
/// passes through unchanged.
fn describe_limit(message: &str) -> String {
    let quota = if message.contains("palette") || message.contains("block-entry") {
        format!("block-state entries, up to {}", grouped(MAX_PALETTE_ENTRIES as u64))
    } else if message.contains("NBT node") {
        format!("NBT nodes, up to {}", grouped(MAX_NBT_NODES as u64))
    } else if message.contains("depth") {
        format!("NBT nesting depth, up to {}", grouped(MAX_NBT_DEPTH as u64))
    } else if message.contains("region") {
        format!("regions per file, up to {}", grouped(MAX_REGIONS as u64))
    } else if message.contains("dimension") {
        format!("blocks per axis, up to {}", grouped(MAX_AXIS_LENGTH as u64))
    } else if message.contains("volume") {
        format!("total block cells, up to {}", grouped(MAX_VOLUME as u64))
    } else if message.contains("decompressed") {
        format!("decompressed bytes, up to {}", grouped(MAX_DECOMPRESSED_BYTES as u64))
    } else {
        return message.to_string();
    };
    format!("Preview limit reached: {quota}.")
}

unsafe fn mesh_failure(err_out: *mut NQLError, error: MeshFailure) -> i32 {
    match error {
        MeshFailure::Pack(error) => fail(
            err_out,
            status::ERR_PACK,
            &format!("The bundled block resources are invalid: {error}"),
        ),
        MeshFailure::NoBlocks => fail(
            err_out,
            status::ERR_NO_BLOCKS,
            "This schematic contains no blocks to render.",
        ),
        MeshFailure::Limit(message) => fail(err_out, status::ERR_LIMIT, &describe_limit(&message)),
        MeshFailure::Cancelled => fail(err_out, status::ERR_CANCELLED, "Preview cancelled."),
        MeshFailure::Mesh(_) => fail(
            err_out,
            status::ERR_MESH,
            "This schematic has too much visible surface to preview. Its block geometry exceeds what the renderer can build.",
        ),
    }
}

#[no_mangle]
pub unsafe extern "C" fn nql_buffer_free(buffer: *mut u8, len: usize) {
    if !buffer.is_null() {
        drop(Vec::from_raw_parts(buffer, len, len));
    }
}
