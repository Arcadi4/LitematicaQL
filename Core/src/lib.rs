//! C ABI and native ownership adapter over litematica-preview-core.
use litematica_preview_core::{DecodeFailure, DecodedSchematic, PreviewOptions};
use nucleation::formats::limits::DecodeLimits;
use nucleation::meshing::ResourcePackSource;
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
    Mesh(String),
    Cancelled,
}

fn decode(bytes: &[u8]) -> Result<NQLSchematic, DecodeFailure> {
    let workers = decode_worker_count();
    let thread_count = (workers > 1).then_some(workers as u8);
    let source = litematica_preview_core::decode_with_options(
        bytes,
        &preview_limits(),
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

#[no_mangle]
pub unsafe extern "C" fn nql_schematic_open(
    data: *const u8,
    len: usize,
    out: *mut *mut NQLSchematic,
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
    match catch_unwind(AssertUnwindSafe(|| decode(bytes))) {
        Ok(Ok(schematic)) => {
            *out = Box::into_raw(Box::new(schematic));
            status::OK
        }
        Ok(Err(DecodeFailure::Format(message))) => fail(err_out, status::ERR_FORMAT, &message),
        Ok(Err(DecodeFailure::Limit(message))) => fail(err_out, status::ERR_LIMIT, &message),
        Err(_) => fail(
            err_out,
            status::ERR_INTERNAL,
            "Something went wrong while reading this schematic.",
        ),
    }
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
    let outcome = catch_unwind(AssertUnwindSafe(|| {
        let cancelled = (*schematic).cancelled.clone();
        let current = || {
            if cancelled.load(Ordering::Relaxed) {
                Err("Preview cancelled.".to_string())
            } else {
                Ok(())
            }
        };
        stream::NQLMeshStream::open(&*schematic, &*pack, &current)
    }));
    match outcome {
        Err(_) => fail(
            err_out,
            status::ERR_MESH,
            "This schematic has too much visible surface to preview. Its block geometry exceeds what the renderer can build.",
        ),
        Ok(Err(error)) => mesh_failure(err_out, error),
        Ok(Ok((stream, atlas, atlas_info, info))) => {
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
    }
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
