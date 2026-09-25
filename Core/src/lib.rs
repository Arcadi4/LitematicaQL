// Native schematic decoding and bounded, chunked preview meshing for Swift hosts.
//
// Decoding retains occupied blocks in 64-block spatial groups. Meshing builds
// every chunk against one shared texture atlas, consumes worker results in
// deterministic order, aggregates them into one MeshOutput, and exports one GLB.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nucleation::formats::limits::DecodeLimits;
use nucleation::formats::manager::get_manager;
use nucleation::meshing::{MeshConfig, MeshOutput, ResourcePackSource};
use regex::Regex;
use schematic_mesher::MeshLayer;

mod litematic;
mod mca;
mod meshing;
mod parallel;
mod structure_nbt;

const MAX_INPUT_BYTES: usize = 1_024 * 1_024 * 1_024;
const MAX_DECOMPRESSED_BYTES: usize = 1_024 * 1_024 * 1_024;
const MAX_AXIS_LENGTH: usize = 4_096;
const MAX_VOLUME: usize = 536_870_912;
const MAX_REGIONS: usize = 64;
const MAX_PALETTE_ENTRIES: usize = 4_096;
const MAX_ENTITIES: usize = 100_000;
const MAX_BLOCK_ENTITIES: usize = 100_000;
const MAX_NBT_DEPTH: usize = 64;
const MAX_NBT_STRING_BYTES: usize = 1_000_000;
const MAX_NBT_COLLECTION_ITEMS: usize = MAX_VOLUME * 2;
const MAX_NBT_NODES: usize = 4_194_304;
const MAX_MESH_BLOCKS: i64 = 33_554_432;
const CHUNK_SIZE: i32 = 64;
const MAX_WORKERS: usize = 8;
// Two workers leave the available CPU budget unused on large previews. Four
// keeps the bounded output window while allowing chunk meshing to scale.
const DEFAULT_WORKERS: usize = 4;

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
}

/// An application-owned decoded preview. Opaque to C callers.
pub struct NQLSchematic {
    source: meshing::CompactBlocks,
    warnings: Vec<String>,
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
}

#[repr(C)]
#[derive(Debug, PartialEq, Eq)]
pub struct NQLMeshInfo {
    pub triangle_count: i64,
}

// A UTF-8 failure message. The host frees `message` with `nql_buffer_free`.
#[repr(C)]
pub struct NQLError {
    pub message: *mut u8,
    pub message_len: usize,
}

pub fn preview_limits() -> DecodeLimits {
    DecodeLimits {
        max_input_bytes: MAX_INPUT_BYTES,
        max_decompressed_bytes: MAX_DECOMPRESSED_BYTES,
        max_dimension: MAX_AXIS_LENGTH,
        max_volume: MAX_VOLUME,
        max_regions: MAX_REGIONS,
        max_palette_entries: MAX_PALETTE_ENTRIES,
        max_entities: MAX_ENTITIES,
        max_block_entities: MAX_BLOCK_ENTITIES,
        max_nbt_depth: MAX_NBT_DEPTH,
        max_nbt_string_bytes: MAX_NBT_STRING_BYTES,
        max_nbt_collection_items: MAX_NBT_COLLECTION_ITEMS,
        max_nbt_nodes: MAX_NBT_NODES,
    }
}

fn diagnostic_limits() -> DecodeLimits {
    DecodeLimits {
        max_dimension: 65_536,
        max_palette_entries: 65_536,
        max_regions: 4_096,
        ..preview_limits()
    }
}

fn mesh_config() -> MeshConfig {
    MeshConfig::new()
        .with_greedy_meshing(true)
        .with_ambient_occlusion(true)
        .with_atlas_max_size(2_048)
}

fn decode_worker_count() -> usize {
    std::thread::available_parallelism().map_or(2, |count| {
        count.get().min(2).max(1)
    })
}

fn mesh_worker_count() -> usize {
    std::thread::available_parallelism().map_or(DEFAULT_WORKERS, |count| {
        count.get().min(MAX_WORKERS).max(1)
    })
}

fn normalize_structure_snbt(bytes: &[u8]) -> Option<Vec<u8>> {
    static BRACE_STATE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#"state:\s*"([\w./:-]+)\{([^{}"]+)\}""#).expect("brace-state pattern is static")
    });
    let text = std::str::from_utf8(bytes).ok()?;
    let brace_state = &*BRACE_STATE;
    if !brace_state.is_match(text) {
        return None;
    }
    let mut normalized = String::with_capacity(text.len());
    let mut last = 0;
    for captures in brace_state.captures_iter(text) {
        let matched = captures.get(0).expect("capture zero always exists");
        normalized.push_str(&text[last..matched.start()]);
        normalized.push_str("state:\"");
        normalized.push_str(&captures[1]);
        normalized.push('[');
        normalized.push_str(&captures[2]);
        normalized.push_str("]\"");
        last = matched.end();
    }
    normalized.push_str(&text[last..]);
    Some(normalized.into_bytes())
}

#[derive(Debug)]
pub enum DecodeFailure {
    Format(String),
    Limit(String),
}

#[derive(Debug)]
pub enum MeshFailure {
    Pack(String),
    NoBlocks,
    Mesh(String),
    Cancelled,
}

/// Decode all supported schematic formats into compact 64-block groups.
pub fn decode(bytes: &[u8]) -> Result<NQLSchematic, DecodeFailure> {
    decode_with_warnings(bytes)
}

pub fn decode_with_warnings(bytes: &[u8]) -> Result<NQLSchematic, DecodeFailure> {
    preview_limits()
        .check_input(bytes)
        .map_err(|error| DecodeFailure::Limit(error.to_string()))?;
    match litematic::read_compact(
        bytes,
        &preview_limits(),
        Some(CHUNK_SIZE),
        Some(decode_worker_count() as u8),
        true,
        &|| Ok(()),
    ) {
        Ok(Some(source)) => return Ok(preview(source, Vec::new())),
        Ok(None) => {}
        Err(error) => return Err(compact_decode_failure(&error)),
    }

    let manager = get_manager();
    let guard = manager
        .lock()
        .map_err(|_| DecodeFailure::Format("the format registry is unavailable".to_string()))?;

    if let Some(normalized) = normalize_structure_snbt(bytes) {
        if let Ok((_, schematic)) = guard.read_bounded_with_format(&normalized, &preview_limits()) {
            return convert_dense(schematic, Vec::new());
        }
    }

    if let Ok((_, schematic)) = guard.read_bounded_with_format(bytes, &preview_limits()) {
        return convert_dense(schematic, Vec::new());
    }
    drop(guard);

    if structure_nbt::is_binary_structure(bytes) {
        return structure_nbt::load_structure_nbt(bytes)
            .and_then(|schematic| convert_dense(schematic, Vec::new()));
    }
    if mca::is_mca(bytes) {
        let (schematic, warnings) = mca::load_mca_preview(bytes)?;
        return convert_dense(schematic, warnings);
    }

    let manager = get_manager();
    let guard = manager
        .lock()
        .map_err(|_| DecodeFailure::Format("the format registry is unavailable".to_string()))?;
    match guard.read_bounded_with_format(bytes, &diagnostic_limits()) {
        Ok((_, diagnostic)) => Err(DecodeFailure::Limit(oversized_message(&diagnostic))),
        Err(_) => Err(DecodeFailure::Format(format!(
            "This file is not a readable Minecraft schematic, or it expands beyond the {} MiB preview limit.",
            MAX_DECOMPRESSED_BYTES / 1_048_576
        ))),
    }
}

fn convert_dense(
    schematic: nucleation::UniversalSchematic,
    warnings: Vec<String>,
) -> Result<NQLSchematic, DecodeFailure> {
    let source = meshing::CompactBlocks::from_schematic(schematic, Some(CHUNK_SIZE), &|| Ok(()))
        .map_err(|_| {
            DecodeFailure::Format(
                "This schematic could not be converted for previewing.".to_string(),
            )
        })?;
    Ok(preview(source, warnings))
}

fn preview(source: meshing::CompactBlocks, warnings: Vec<String>) -> NQLSchematic {
    NQLSchematic {
        source,
        warnings,
        cancelled: Arc::new(AtomicBool::new(false)),
    }
}

fn compact_decode_failure(error: &str) -> DecodeFailure {
    if error.contains("limit")
        || error.contains("volume")
        || error.contains("dimensions")
        || error.contains("too many")
    {
        DecodeFailure::Limit("This schematic is too large to preview.".to_string())
    } else {
        DecodeFailure::Format(
            "This file is not a readable Minecraft schematic, or it expands beyond the 1024 MiB preview limit."
                .to_string(),
        )
    }
}

fn oversized_message(schematic: &nucleation::UniversalSchematic) -> String {
    let block_count = i64::from(schematic.total_blocks());
    if block_count > MAX_MESH_BLOCKS {
        format!(
            "This schematic renders {block_count} blocks, beyond the {MAX_MESH_BLOCKS}-block preview limit."
        )
    } else {
        let (x, y, z) = schematic.get_dimensions();
        format!(
            "This schematic is {x} × {y} × {z}, beyond the {MAX_AXIS_LENGTH}-block per-axis and {MAX_VOLUME}-block preview limits."
        )
    }
}

pub fn resource_pack(bytes: &[u8]) -> Result<NQLResourcePack, MeshFailure> {
    ResourcePackSource::from_bytes(bytes)
        .map(NQLResourcePack)
        .map_err(|error| MeshFailure::Pack(error.to_string()))
}

pub fn mesh(
    schematic: &NQLSchematic,
    pack: &NQLResourcePack,
) -> Result<(Vec<u8>, NQLMeshInfo), MeshFailure> {
    let cancelled = schematic.cancelled.clone();
    let current = || check_cancelled(&cancelled);
    current().map_err(|_| MeshFailure::Cancelled)?;

    let block_count = schematic.source.block_count();
    if block_count > MAX_MESH_BLOCKS {
        return Err(MeshFailure::Mesh(format!(
            "This schematic renders {block_count} blocks, beyond the {MAX_MESH_BLOCKS}-block preview limit."
        )));
    }
    let mut chunks =
        meshing::ChunkMeshes::from_source(&schematic.source, &pack.0, &mesh_config(), &current)
            .map_err(MeshFailure::Mesh)?;
    let mut aggregate: Option<MeshOutput> = None;
    chunks
        .consume(
            mesh_worker_count(),
            |output| {
                current()?;
                append_mesh(&mut aggregate, output);
                Ok(())
            },
            &current,
        )
        .map_err(|error| {
            if error == "Parallel preview cancelled." {
                MeshFailure::Cancelled
            } else {
                MeshFailure::Mesh(error)
            }
        })?;

    let Some(mut output) = aggregate else {
        return Err(MeshFailure::NoBlocks);
    };
    output.greedy_materials.sort_by(|left, right| {
        left.texture_path
            .cmp(&right.texture_path)
            .then_with(|| left.texture_png.cmp(&right.texture_png))
    });
    current().map_err(|_| MeshFailure::Cancelled)?;
    let greedy_triangles: usize = output
        .greedy_materials
        .iter()
        .map(|material| material.opaque.triangle_count() + material.transparent.triangle_count())
        .sum();
    let total_triangles = output
        .total_triangles()
        .checked_add(greedy_triangles)
        .ok_or_else(|| MeshFailure::Mesh("This schematic has too many triangles.".to_string()))?;
    if total_triangles == 0 {
        return Err(MeshFailure::NoBlocks);
    }
    let glb = output
        .to_glb()
        .map_err(|error| MeshFailure::Mesh(error.to_string()))?;
    current().map_err(|_| MeshFailure::Cancelled)?;
    let triangle_count = i64::try_from(total_triangles)
        .map_err(|_| MeshFailure::Mesh("This schematic has too many triangles.".to_string()))?;
    Ok((glb, NQLMeshInfo { triangle_count }))
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), String> {
    if cancelled.load(Ordering::Relaxed) {
        Err("Parallel preview cancelled.".to_string())
    } else {
        Ok(())
    }
}

fn append_mesh(aggregate: &mut Option<MeshOutput>, mut output: MeshOutput) {
    let Some(target) = aggregate else {
        output.chunk_coord = None;
        *aggregate = Some(output);
        return;
    };
    append_layer(&mut target.opaque, output.opaque);
    append_layer(&mut target.cutout, output.cutout);
    append_layer(&mut target.transparent, output.transparent);
    for animation in output.animated_textures {
        if !target.animated_textures.iter().any(|existing| {
            existing.atlas_x == animation.atlas_x && existing.atlas_y == animation.atlas_y
        }) {
            target.animated_textures.push(animation);
        }
    }
    target.greedy_materials.append(&mut output.greedy_materials);
    for axis in 0..3 {
        target.bounds.min[axis] = target.bounds.min[axis].min(output.bounds.min[axis]);
        target.bounds.max[axis] = target.bounds.max[axis].max(output.bounds.max[axis]);
    }
}

fn append_layer(target: &mut MeshLayer, mut source: MeshLayer) {
    let offset = target.positions.len() as u32;
    target.positions.reserve(source.positions.len());
    target.normals.reserve(source.normals.len());
    target.uvs.reserve(source.uvs.len());
    target.colors.reserve(source.colors.len());
    target.indices.reserve(source.indices.len());
    target.positions.append(&mut source.positions);
    target.normals.append(&mut source.normals);
    target.uvs.append(&mut source.uvs);
    target.colors.append(&mut source.colors);
    target
        .indices
        .extend(source.indices.into_iter().map(|index| index + offset));
}

pub fn export_bytes(bytes: Vec<u8>) -> (*mut u8, usize) {
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
    let (buffer, len) = export_bytes((*schematic).warnings.join("\n").into_bytes());
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
    *out = NQLSchematicInfo {
        block_count: source.block_count(),
        block_entity_count: source.block_entity_count(),
        content_x: content.0,
        content_y: content.1,
        content_z: content.2,
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
pub unsafe extern "C" fn nql_schematic_mesh(
    schematic: *const NQLSchematic,
    pack: *const NQLResourcePack,
    glb_out: *mut *mut u8,
    glb_len: *mut usize,
    info_out: *mut NQLMeshInfo,
    err_out: *mut NQLError,
) -> i32 {
    init_error(err_out);
    if schematic.is_null() || pack.is_null() || glb_out.is_null() || glb_len.is_null() {
        return fail(
            err_out,
            status::ERR_NULL,
            "The caller passed a null buffer.",
        );
    }
    *glb_out = std::ptr::null_mut();
    *glb_len = 0;
    let outcome = catch_unwind(AssertUnwindSafe(|| mesh(&*schematic, &*pack)));
    match outcome {
        Err(_) => fail(
            err_out,
            status::ERR_MESH,
            "This schematic has too much visible surface to preview. Its block geometry exceeds what the renderer can build.",
        ),
        Ok(Err(MeshFailure::Pack(error))) => fail(
            err_out,
            status::ERR_PACK,
            &format!("The bundled block resources are invalid: {error}"),
        ),
        Ok(Err(MeshFailure::NoBlocks)) => fail(
            err_out,
            status::ERR_NO_BLOCKS,
            "This schematic contains no blocks to render.",
        ),
        Ok(Err(MeshFailure::Cancelled)) => {
            fail(err_out, status::ERR_CANCELLED, "Preview cancelled.")
        }
        Ok(Err(MeshFailure::Mesh(_))) => fail(
            err_out,
            status::ERR_MESH,
            "This schematic has too much visible surface to preview. Its block geometry exceeds what the renderer can build.",
        ),
        Ok(Ok((glb, info))) => {
            let (data, len) = export_bytes(glb);
            *glb_out = data;
            *glb_len = len;
            if !info_out.is_null() {
                *info_out = info;
            }
            status::OK
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn nql_buffer_free(buffer: *mut u8, len: usize) {
    if !buffer.is_null() {
        drop(Vec::from_raw_parts(buffer, len, len));
    }
}
