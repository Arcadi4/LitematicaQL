// Schematic decoding and meshing over a small C ABI.
//
// The Quick Look extension and the app call these entry points instead of the
// WebAssembly bridge. The native heap has no wasm memory ceiling, so builds
// like a 4.4-million-block station mesh here where the wasm mesher aborts.
// Nucleation panics must never cross the FFI boundary, so every entry point
// runs inside `catch_unwind` and reports failures as status codes plus a
// UTF-8 message the host frees with `nql_buffer_free`.
//
// Policy lives on the Swift side; this crate only enforces decode limits and
// the mesh block budget.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::slice;

use nucleation::formats::limits::DecodeLimits;
use nucleation::formats::manager::get_manager;
use nucleation::meshing::{MeshConfig, MeshError, ResourcePackSource};
use nucleation::UniversalSchematic;
use regex::Regex;

mod structure_nbt;

// Compressed schematic bytes accepted from the native bridge.
const MAX_INPUT_BYTES: usize = 1_024 * 1_024 * 1_024;
// Inflated NBT bytes Nucleation may allocate while decoding.
const MAX_DECOMPRESSED_BYTES: usize = 1_024 * 1_024 * 1_024;
const MAX_AXIS_LENGTH: usize = 4_096;
// Cells Nucleation may decode per schematic; mirrors the renderer budget.
const MAX_VOLUME: usize = 536_870_912;
const MAX_REGIONS: usize = 64;
const MAX_PALETTE_ENTRIES: usize = 4_096;
const MAX_ENTITIES: usize = 100_000;
const MAX_BLOCK_ENTITIES: usize = 100_000;
const MAX_NBT_DEPTH: usize = 64;
const MAX_NBT_STRING_BYTES: usize = 1_000_000;
// A Sponge `BlockData` array declares one VarInt per padded cell, so the
// collection allowance is the volume widened by a VarInt's width.
const MAX_NBT_COLLECTION_ITEMS: usize = MAX_VOLUME * 2;
const MAX_NBT_NODES: usize = 4_194_304;

// Blocks the preview is willing to mesh: a usefulness bound, not a memory
// one. Past this many blocks the GLB is too large for any preview to show.
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

// Build facts reported right after a successful decode.
#[repr(C)]
pub struct NQLSchematicInfo {
    pub block_count: i64,
    pub block_entity_count: i64,
    // Dimensions of the region the blocks actually occupy. Region padding is
    // a whole-chunk affair, so the declared size overstates what is drawn
    // and nothing user-facing reports it.
    pub content_x: i32,
    pub content_y: i32,
    pub content_z: i32,
}

// Facts about a finished mesh.
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

// Decode limits bounding what a preview accepts.
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

// Widened allowances for a diagnostic second decode: Nucleation reports
// oversized and unreadable inputs as the same parse error, and this is the
// only way to tell them apart. The input cap stays at the preview value.
fn diagnostic_limits() -> DecodeLimits {
    DecodeLimits {
        max_dimension: 65_536,
        max_palette_entries: 65_536,
        max_regions: 4_096,
        ..preview_limits()
    }
}

// The mesher configuration the preview ships with: greedy merging for the
// triangle count, a 2048 atlas because the preview cannot resolve more.
fn mesh_config() -> MeshConfig {
    MeshConfig::new()
        .with_greedy_meshing(true)
        .with_ambient_occlusion(true)
        .with_atlas_max_size(2_048)
}

// Rewrites the brace block-state spelling some structure SNBT uses (for
// example `state: "minecraft:oak_log{axis=y}"`) into the bracket form
// Nucleation's reader accepts, for one retry after a failed import.
// Restricted to `state` values so nothing else in the document is touched.
// `None` skips the retry.
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
//
// Nucleation reports every rejection as the same opaque parse error, so a
// second decode with the diagnostic allowances decides whether the input was
// oversized or unreadable and the refusal says which.
pub fn decode(bytes: &[u8]) -> Result<NQLSchematic, DecodeFailure> {
    let manager = get_manager();
    let guard = manager
        .lock()
        .map_err(|_| DecodeFailure::Format("the format registry is unavailable".to_string()))?;

    if let Ok((_, schematic)) = guard.read_bounded_with_format(bytes, &preview_limits()) {
        return Ok(schematic);
    }

    // A readable document that only trips on Nucleation's brace block-state
    // spelling gets one normalized retry.
    if let Some(normalized) = normalize_structure_snbt(bytes) {
        if let Ok((_, schematic)) = guard.read_bounded_with_format(&normalized, &preview_limits()) {
            return Ok(schematic);
        }
    }

    if structure_nbt::is_binary_structure(bytes) {
        return structure_nbt::load_structure_nbt(bytes);
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

// Mesh the schematic into GLB bytes with the shipping configuration.
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
            // Nucleation reports an empty build as this string, not as a typed
            // variant, so the refusal has to match on the message.
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

// Move `bytes` onto the heap for the host and hand back its location.
//
// `shrink_to_fit` plus `into_boxed_slice` guarantees capacity equals length,
// which is what `nql_buffer_free` reconstructs the allocation from.
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

// Decode schematic bytes into an owning handle.
//
// # Safety
// `data` must point at `len` readable bytes; `out` and `err` (if non-null)
// must be writable. On success the handle must be freed exactly once with
// `nql_schematic_free`. On failure `err` receives a message to free with
// `nql_buffer_free`.
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

// Free a handle returned by `nql_schematic_open`.
//
// # Safety
// `schematic` must be null or a live handle that has not been freed already.
#[no_mangle]
pub unsafe extern "C" fn nql_schematic_free(schematic: *mut NQLSchematic) {
    if !schematic.is_null() {
        drop(Box::from_raw(schematic));
    }
}

// Report build facts about a decoded schematic.
//
// # Safety
// `schematic` must be a live handle and `out` must be writable.
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

// Mesh a decoded schematic into GLB bytes.
//
// The schematic handle stays valid; the pack is read from `pack_data` on
// every call. On success `glb_out`/`glb_len` receive an allocation to free
// with `nql_buffer_free` and `info_out` receives mesh facts.
//
// # Safety
// Pointer arguments must be valid and, where marked out, writable.
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

// Free an allocation handed out by this library.
//
// # Safety
// `buffer` must be null or a live allocation from `nql_schematic_open`'s
// error message or `nql_schematic_mesh`'s GLB output, with its exact length.
#[no_mangle]
pub unsafe extern "C" fn nql_buffer_free(buffer: *mut u8, len: usize) {
    if !buffer.is_null() {
        drop(Vec::from_raw_parts(buffer, len, len));
    }
}
