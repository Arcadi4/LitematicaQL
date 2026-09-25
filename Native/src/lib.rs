// Native schematic decoding and meshing for Swift hosts.
//
// The native heap avoids wasm's memory ceiling for large preview meshes. The
// decoding and meshing entry points catch panics and return status codes;
// callers free every returned buffer with `nql_buffer_free`. Swift owns UI
// policy, while this crate enforces decode and mesh budgets.

use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;
use std::sync::{LazyLock, Mutex};

use nucleation::formats::limits::DecodeLimits;
use nucleation::formats::manager::get_manager;
use nucleation::meshing::{MeshConfig, MeshError, ResourcePackSource};
use nucleation::UniversalSchematic;
use regex::Regex;

mod mca;
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
// Sponge `BlockData` uses one padded cell per VarInt; the parser's collection
// allowance must cover both the volume and its per-cell encoding overhead.
const MAX_NBT_COLLECTION_ITEMS: usize = MAX_VOLUME * 2;
const MAX_NBT_NODES: usize = 4_194_304;

// This is a usefulness bound, not an allocation bound: larger GLBs exceed what
// the preview can display.
const MAX_MESH_BLOCKS: i64 = 33_554_432;

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
}

// A decoded schematic. Opaque to the host; free with `nql_schematic_free`.
pub type NQLSchematic = UniversalSchematic;

// `UniversalSchematic` cannot carry host-facing notices, so they remain keyed
// by live handle. Freeing a handle removes its entry before allocator reuse can
// produce the same address.
static DECODE_WARNINGS: LazyLock<Mutex<HashMap<usize, Vec<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[repr(C)]
pub struct NQLSchematicInfo {
    pub block_count: i64,
    pub block_entity_count: i64,
    // Tight content bounds, not the chunk-aligned schematic dimensions.
    pub content_x: i32,
    pub content_y: i32,
    pub content_z: i32,
}

#[repr(C)]
#[derive(Debug)]
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

// Nucleation maps malformed and oversized inputs to the same parse error. A
// second decode with wider structural limits distinguishes them without
// raising the compressed-input cap.
fn diagnostic_limits() -> DecodeLimits {
    DecodeLimits {
        max_dimension: 65_536,
        max_palette_entries: 65_536,
        max_regions: 4_096,
        ..preview_limits()
    }
}

// Greedy merging controls the triangle count; 2048 is the largest atlas the
// preview can resolve.
fn mesh_config() -> MeshConfig {
    MeshConfig::new()
        .with_greedy_meshing(true)
        .with_ambient_occlusion(true)
        .with_atlas_max_size(2_048)
}

// Convert brace-style `state` values such as `axis=y` in
// `minecraft:oak_log{axis=y}` to the bracket syntax Nucleation accepts. Only
// `state` values are rewritten, and `None` skips the one normalization retry.
fn normalize_structure_snbt(bytes: &[u8]) -> Option<Vec<u8>> {
    static BRACE_STATE: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(
            r#"state:\s*"([\w.-]+:[\w/.-]+)\{([\w.-]+[=:][\w.\-/]+(?:,[\w.-]+[=:][\w.\-/]+)*)\}""#,
        )
        .expect("brace-state pattern is static")
    });
    let text = std::str::from_utf8(bytes).ok()?;
    let brace_state = &*BRACE_STATE;

    if !brace_state.is_match(text) {
        return None;
    }
    Some(
        brace_state
            .replace_all(text, r#"state:"$1[$2]""#)
            .into_owned()
            .into_bytes(),
    )
}

#[derive(Debug)]
pub enum DecodeFailure {
    Format(String),
    Limit(String),
}

// Decode schematic `bytes`, refusing anything outside the preview budget.
pub fn decode(bytes: &[u8]) -> Result<NQLSchematic, DecodeFailure> {
    decode_with_warnings(bytes).map(|(schematic, _)| schematic)
}

// Decode under the preview budget and return notices for omitted unreadable or
// externally stored chunks. The C ABI exposes these through
// `nql_schematic_warnings`.
//
// A second decode with wider structural limits distinguishes Nucleation's
// ambiguous parse error for malformed input from a limit rejection.
pub fn decode_with_warnings(bytes: &[u8]) -> Result<(NQLSchematic, Vec<String>), DecodeFailure> {
    let manager = get_manager();
    let guard = manager
        .lock()
        .map_err(|_| DecodeFailure::Format("the format registry is unavailable".to_string()))?;

    if let Ok((_, schematic)) = guard.read_bounded_with_format(bytes, &preview_limits()) {
        return Ok((schematic, Vec::new()));
    }

    if let Some(normalized) = normalize_structure_snbt(bytes) {
        if let Ok((_, schematic)) = guard.read_bounded_with_format(&normalized, &preview_limits())
        {
            return Ok((schematic, Vec::new()));
        }
    }

    if structure_nbt::is_binary_structure(bytes) {
        return structure_nbt::load_structure_nbt(bytes).map(|schematic| (schematic, Vec::new()));
    }

    if mca::is_mca(bytes) {
        return mca::load_mca_preview(bytes);
    }

    match guard.read_bounded_with_format(bytes, &diagnostic_limits()) {
        Ok((_, diagnostic)) => Err(DecodeFailure::Limit(oversized_message(&diagnostic))),
        Err(_) => Err(DecodeFailure::Format(format!(
            "This file is not a readable Minecraft schematic, or it expands beyond the {} MiB preview limit.",
            MAX_DECOMPRESSED_BYTES / 1_048_576
        ))),
    }
}


fn oversized_message(schematic: &NQLSchematic) -> String {
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

#[derive(Debug)]
pub enum MeshFailure {
    Pack(String),
    NoBlocks,
    Mesh(String),
}

pub fn mesh(
    schematic: &NQLSchematic,
    pack_bytes: &[u8],
) -> Result<(Vec<u8>, NQLMeshInfo), MeshFailure> {
    let pack = ResourcePackSource::from_bytes(pack_bytes)
        .map_err(|error| MeshFailure::Pack(error.to_string()))?;

    let output = schematic
        .to_mesh(&pack, &mesh_config())
        .map_err(|error| match error {
            MeshError::ResourcePack(error) => MeshFailure::Pack(error),
            // Nucleation reports an empty build as text rather than a typed
            // error, so preserve that distinction with a guarded match.
            MeshError::Meshing(message) if message.contains("No blocks") => MeshFailure::NoBlocks,
            error => MeshFailure::Mesh(error.to_string()),
        })?;

    let greedy_triangles: usize = output
        .greedy_materials
        .iter()
        .map(|gm| gm.opaque.triangle_count() + gm.transparent.triangle_count())
        .sum();
    let total_triangles = output.total_triangles() + greedy_triangles;

    if total_triangles == 0 {
        return Err(MeshFailure::NoBlocks);
    }

    let glb = output
        .to_glb()
        .map_err(|error| MeshFailure::Mesh(error.to_string()))?;

    let info = NQLMeshInfo {
        triangle_count: total_triangles as i64,
    };
    Ok((glb, info))
}

// Transfer an owning byte buffer to the host. The boxed slice's capacity equals
// its length, allowing `nql_buffer_free` to reconstruct the allocation.
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

/// Decode schematic bytes into an owning handle.
///
/// # Safety
/// `data` must point to `len` readable bytes, and `out` plus any non-null
/// `err_out` must be writable. On success, free the handle exactly once with
/// `nql_schematic_free`. On failure, free any `err_out` message with
/// `nql_buffer_free`.
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

    let bytes = slice::from_raw_parts(data, len);
    match catch_unwind(AssertUnwindSafe(|| decode_with_warnings(bytes))) {
        Ok(Ok((schematic, warnings))) => {
            let handle = Box::into_raw(Box::new(schematic));
            if !warnings.is_empty() {
                if let Ok(mut store) = DECODE_WARNINGS.lock() {
                    store.insert(handle as usize, warnings);
                }
            }
            *out = handle;
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

/// Free a schematic handle.
///
/// # Safety
/// `schematic` must be null or a live handle that has not already been freed.
#[no_mangle]
pub unsafe extern "C" fn nql_schematic_free(schematic: *mut NQLSchematic) {
    if !schematic.is_null() {
        if let Ok(mut store) = DECODE_WARNINGS.lock() {
            store.remove(&(schematic as usize));
        }
        drop(Box::from_raw(schematic));
    }
}

/// Return newline-separated decode notices, or an empty allocation when the
/// schematic has none. The host must free the buffer with `nql_buffer_free`.
///
/// # Safety
/// `schematic` must be live, and `out` and `out_len` must be writable.
#[no_mangle]
pub unsafe extern "C" fn nql_schematic_warnings(
    schematic: *const NQLSchematic,
    out: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if schematic.is_null() || out.is_null() || out_len.is_null() {
        return status::ERR_NULL;
    }

    let text = DECODE_WARNINGS
        .lock()
        .ok()
        .and_then(|store| store.get(&(schematic as usize)).cloned())
        .unwrap_or_default()
        .join("\n");
    let (buffer, len) = export_bytes(text.into_bytes());
    *out = buffer;
    *out_len = len;
    status::OK
}

/// Report decoded schematic statistics.
///
/// # Safety
/// `schematic` must be live and `out` must be writable.
#[no_mangle]
pub unsafe extern "C" fn nql_schematic_info(
    schematic: *const NQLSchematic,
    out: *mut NQLSchematicInfo,
) -> i32 {
    if schematic.is_null() || out.is_null() {
        return status::ERR_NULL;
    }

    let schematic = &*schematic;
    let content = schematic.get_tight_dimensions();
    *out = NQLSchematicInfo {
        block_count: i64::from(schematic.total_blocks()),
        block_entity_count: schematic.get_block_entities_as_list().len() as i64,
        content_x: content.0,
        content_y: content.1,
        content_z: content.2,
    };
    status::OK
}

/// Mesh a schematic into an owned GLB allocation.
///
/// On success, free `glb_out` with `nql_buffer_free`. `info_out` and `err_out`
/// are optional; when non-null they must be writable.
///
/// # Safety
/// `schematic` must be live, and `pack_data` must point to `pack_len` readable
/// bytes. `glb_out` and `glb_len` must be writable.
#[no_mangle]
pub unsafe extern "C" fn nql_schematic_mesh(
    schematic: *const NQLSchematic,
    pack_data: *const u8,
    pack_len: usize,
    glb_out: *mut *mut u8,
    glb_len: *mut usize,
    info_out: *mut NQLMeshInfo,
    err_out: *mut NQLError,
) -> i32 {
    init_error(err_out);
    if schematic.is_null() || pack_data.is_null() || glb_out.is_null() || glb_len.is_null() {
        return fail(
            err_out,
            status::ERR_NULL,
            "The caller passed a null buffer.",
        );
    }

    let pack_bytes = slice::from_raw_parts(pack_data, pack_len);
    let outcome = catch_unwind(AssertUnwindSafe(|| mesh(&*schematic, pack_bytes)));
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

/// Free a buffer returned by this library.
///
/// # Safety
/// `buffer` must be null or a live library allocation paired with the length
/// returned with it.
#[no_mangle]
pub unsafe extern "C" fn nql_buffer_free(buffer: *mut u8, len: usize) {
    if !buffer.is_null() {
        drop(Vec::from_raw_parts(buffer, len, len));
    }
}
