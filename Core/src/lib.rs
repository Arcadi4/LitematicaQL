//! C ABI and native ownership adapter over litematica-preview-core.
use litematica_preview_core::{DecodeFailure, DecodedSchematic, PreviewOptions};
use nucleation::formats::limits::DecodeLimits;
use nucleation::meshing::{MeshConfig, ResourcePackSource};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub mod stream;

// Preview limits.
const MAX_DECOMPRESSED_BYTES: usize = 4 * 1_024 * 1_024 * 1_024;
const MAX_AXIS_LENGTH: usize = 8_192;
const MAX_VOLUME: usize = 2048 * 2048 * 2048;
const MAX_REGIONS: usize = 64;
const MAX_PALETTE_ENTRIES: usize = 32_768;
const MAX_NBT_DEPTH: usize = 64;
const MAX_NBT_NODES: usize = 4_194_304;
const MAX_MESH_BLOCKS: i64 = 512 * 512 * 512;
/// Maximum side of the shared texture atlas, in texels. The atlas builder
/// grows the atlas only as far as the palette needs, so a palette with fewer
/// distinct tiles gets a smaller atlas.
const MAX_ATLAS_SIDE: u32 = 2048;
/// Atlas side for the override tier. The builder holds 256 MB of host pixels
/// while it places the tiles, and the device keeps the finished atlas after
/// that. Only the override tier uses this side.
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

/// A decoded preview that the host owns. Opaque to C callers.
pub struct NQLSchematic {
    source: DecodedSchematic,
    cancelled: Arc<AtomicBool>,
}

/// A parsed immutable resource pack. Sequential previews can reuse it.
pub struct NQLResourcePack(ResourcePackSource);

/// One layer of a composed resource pack. `layers[0]` has the lowest priority.
/// `name` is UTF-8 and names this layer in a failure message.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NQLPackLayer {
    pub data: *const u8,
    pub len: usize,
    pub name: *const u8,
    pub name_len: usize,
}

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
        // node budget, MAX_NBT_NODES. String payloads and collection items count
        // toward it too.
        max_entities: usize::MAX,
        max_block_entities: usize::MAX,
        max_nbt_depth: MAX_NBT_DEPTH,
        max_nbt_string_bytes: usize::MAX,
        max_nbt_collection_items: usize::MAX,
        max_nbt_nodes: MAX_NBT_NODES,
    }
}

/// No preview quotas. The format readers apply only their own width limits.
/// Only the override entry points call this function.
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

/// Mesher configuration, with the atlas side from the tier's [`MeshQuotas`].
///
/// `litematica_preview_core::mesh_config()` hard-codes the atlas side and takes
/// no argument. This function sets the side itself, so the preview and override
/// tiers can use different sizes.
fn mesh_config(atlas_max_side: u32) -> MeshConfig {
    MeshConfig::new()
        .with_greedy_meshing(true)
        .with_ambient_occlusion(true)
        .with_atlas_max_size(atlas_max_side)
}

/// Mesh-stage quotas. `block_budget` is the maximum block count that the
/// stream can describe. `atlas_max_side` is the maximum side of the shared
/// texture atlas.
#[derive(Clone, Copy)]
pub(crate) struct MeshQuotas {
    pub(crate) block_budget: i64,
    pub(crate) atlas_max_side: u32,
}

/// The quota set that a mesh stream open uses: the preview tier, or the
/// override tier that the `*_override` entry points select.
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

/// Meshing concurrency. Each worker meshes one chunk batch, and the stream
/// holds each finished batch whole until the host pulls it. This cap also
/// bounds the geometry in memory. A deeper cap gave no measurable streaming
/// gain, and `Core/tests/large_builds.rs` pins that.
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
    /// A preview quota refused the mesh. The host can recover with an override
    /// entry point.
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

unsafe fn pack_layer_name(layer: &NQLPackLayer) -> String {
    if layer.name.is_null() || layer.name_len == 0 {
        return String::from("Texture pack");
    }
    String::from_utf8_lossy(slice::from_raw_parts(layer.name, layer.name_len)).into_owned()
}

/// Clones the base once, then moves each validated custom pack into it. Each
/// layer parses against an empty pack through the published core validator, so
/// the limits have one home and the accumulated stack is never cloned.
fn resource_pack_stack(
    base: &NQLResourcePack,
    layers: &[NQLPackLayer],
) -> Result<NQLResourcePack, MeshFailure> {
    let empty = ResourcePackSource::from_resource_pack(Default::default());
    let mut merged = base.0.pack().clone();
    for layer in layers {
        let name = unsafe { pack_layer_name(layer) };
        if layer.len == 0 {
            return Err(MeshFailure::Pack(format!(
                "“{name}” is empty. Choose a Minecraft Java resource-pack ZIP."
            )));
        }
        if layer.data.is_null() {
            return Err(MeshFailure::Pack(format!("“{name}” has no readable data.")));
        }
        let bytes = unsafe { slice::from_raw_parts(layer.data, layer.len) };
        let mut custom =
            litematica_preview_core::resource_pack_overlay(&empty, bytes).map_err(|message| {
                MeshFailure::Pack(format!("“{name}” could not be loaded. {message}"))
            })?;
        merged.overlay(std::mem::take(custom.pack_mut()));
    }
    Ok(NQLResourcePack(ResourcePackSource::from_resource_pack(
        merged,
    )))
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

/// Reads a schematic with no preview quotas. The host shows the limit hit to
/// the user, with an offer to render anyway, before it calls this function.
/// The host then owns every admission decision.
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

/// Cancels meshing for this schematic and keeps the handle valid. Cancellation
/// is monotonic. A cancelled handle cannot publish later geometry.
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
pub unsafe extern "C" fn nql_resource_pack_compose(
    base: *const NQLResourcePack,
    layers: *const NQLPackLayer,
    layer_count: usize,
    out: *mut *mut NQLResourcePack,
    err_out: *mut NQLError,
) -> i32 {
    init_error(err_out);
    if base.is_null() || out.is_null() || (layer_count > 0 && layers.is_null()) {
        return fail(
            err_out,
            status::ERR_NULL,
            "The caller passed a null buffer.",
        );
    }
    *out = std::ptr::null_mut();
    let layers = if layer_count == 0 {
        &[]
    } else {
        slice::from_raw_parts(layers, layer_count)
    };
    match catch_unwind(AssertUnwindSafe(|| resource_pack_stack(&*base, layers))) {
        Ok(Ok(pack)) => {
            *out = Box::into_raw(Box::new(pack));
            status::OK
        }
        Ok(Err(MeshFailure::Pack(error))) => fail(err_out, status::ERR_PACK, &error),
        _ => fail(
            err_out,
            status::ERR_INTERNAL,
            "Something went wrong while composing the texture packs.",
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
            // The probe lifts only the atlas side, to OVERRIDE_ATLAS_SIDE, and
            // keeps the preview block budget. If the probe opens a stream, the
            // tiles did not fit the preview atlas. The host then gets the limit
            // message with the override price. The code drops the probe stream,
            // so it builds nothing at the override cost before the user agrees
            // to it. Any other refusal fails the probe too, and the code then
            // reports the original preview failure with its status and message.
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

/// Lifts the render-stage block budget and the shared texture atlas ceiling
/// for the override tier, under the precondition at
/// [`nql_schematic_open_override`].
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

/// The stream borrows the schematic and the pack, and both must outlive it. A
/// returned batch owns its storage and is valid until [`nql_mesh_batch_free`].
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

/// Thousands separators for the numbers in the user-facing limit messages.
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

/// A byte count in binary units, for the atlas price in an override offer. The
/// host renders its own size strings elsewhere. This type only states the
/// graphics-memory cost of the atlas.
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

/// Names the dimension that hit a limit and states its number, so the host can
/// show figures instead of the engine's terse refusal. An unrecognized message
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

#[cfg(test)]
mod tests {
    use super::*;

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFF_u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut output = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = output.len() as u32;
            let crc = crc32(data);
            let size = data.len() as u32;
            let name_len = name.len() as u16;
            output.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
            output.extend_from_slice(&20_u16.to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(&0x0021_u16.to_le_bytes());
            output.extend_from_slice(&crc.to_le_bytes());
            output.extend_from_slice(&size.to_le_bytes());
            output.extend_from_slice(&size.to_le_bytes());
            output.extend_from_slice(&name_len.to_le_bytes());
            output.extend_from_slice(&0_u16.to_le_bytes());
            output.extend_from_slice(name.as_bytes());
            output.extend_from_slice(data);

            central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&20_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0x0021_u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&name_len.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u16.to_le_bytes());
            central.extend_from_slice(&0_u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let central_offset = output.len() as u32;
        let central_size = central.len() as u32;
        let count = entries.len() as u16;
        output.extend_from_slice(&central);
        output.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output.extend_from_slice(&count.to_le_bytes());
        output.extend_from_slice(&count.to_le_bytes());
        output.extend_from_slice(&central_size.to_le_bytes());
        output.extend_from_slice(&central_offset.to_le_bytes());
        output.extend_from_slice(&0_u16.to_le_bytes());
        output
    }

    fn pack(marker_parent: &str, lower_only: bool) -> Vec<u8> {
        let metadata: &[u8] = br#"{"pack":{"pack_format":34,"description":"test"}}"#;
        let marker = format!(r#"{{"parent":"minecraft:{marker_parent}"}}"#);
        let mut entries = vec![
            ("pack.mcmeta", metadata),
            (
                "assets/minecraft/models/block/stack_marker.json",
                marker.as_bytes(),
            ),
        ];
        let fallback = r#"{"parent":"minecraft:block/lower_only"}"#;
        if lower_only {
            entries.push((
                "assets/minecraft/models/block/lower_only.json",
                fallback.as_bytes(),
            ));
        }
        zip(&entries)
    }

    fn layer(bytes: &[u8], name: &[u8]) -> NQLPackLayer {
        NQLPackLayer {
            data: bytes.as_ptr(),
            len: bytes.len(),
            name: name.as_ptr(),
            name_len: name.len(),
        }
    }

    #[test]
    fn stack_overlays_lowest_first_and_keeps_lower_entries() {
        let lower = pack("block/lower", true);
        let higher = pack("block/higher", false);
        let base = NQLResourcePack(ResourcePackSource::from_resource_pack(Default::default()));
        let low = layer(&lower, b"Lower");
        let high = layer(&higher, b"Higher");

        let composed = resource_pack_stack(&base, &[low, high]).expect("composes");
        let marker = composed
            .0
            .get_model_json("minecraft:block/stack_marker")
            .expect("marker model");
        assert!(marker.contains("block/higher"), "{marker}");
        let fallback = composed
            .0
            .get_model_json("minecraft:block/lower_only")
            .expect("lower model");
        assert!(fallback.contains("block/lower_only"), "{fallback}");

        let reversed = resource_pack_stack(&base, &[high, low]).expect("composes");
        let marker = reversed
            .0
            .get_model_json("minecraft:block/stack_marker")
            .expect("marker model");
        assert!(marker.contains("block/lower"), "{marker}");
    }
}
