// Integration tests for the preview pipeline the Swift bridge calls.
//
// Every bundled demo is decoded and meshed with the shipping configuration,
// one per file format, and every minimal format fixture is decoded.

use std::fs;
use std::path::{Path, PathBuf};

use litematicaql_native::status;
use litematicaql_native::{decode, decode_with_warnings, export_bytes, mesh, NQLMeshInfo};
use nucleation::UniversalSchematic;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn pack_bytes() -> Vec<u8> {
    fs::read(repo_root().join("Renderer/vendor/pack.zip")).expect("bundled resource pack")
}

fn demo_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(repo_root().join("Fixtures/Demos"))
        .expect("demo fixtures")
        .map(|entry| entry.expect("demo entry").path())
        .collect();
    paths.sort();
    assert_eq!(paths.len(), 7, "one demo per supported file format");
    paths
}

fn format_fixture_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(repo_root().join("Fixtures/Formats"))
        .expect("format fixtures")
        .map(|entry| entry.expect("fixture entry").path())
        .collect();
    paths.sort();
    paths
}

fn decode_file(path: &Path) -> UniversalSchematic {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    decode(&bytes).unwrap_or_else(|error| panic!("{}: {error:?}", path.display()))
}

fn mesh_schematic(schematic: &UniversalSchematic) -> (Vec<u8>, NQLMeshInfo) {
    mesh(schematic, &pack_bytes()).expect("the demo should mesh")
}

#[test]
fn demos_decode_and_mesh_with_the_shipping_configuration() {
    for path in demo_paths() {
        let schematic = decode_file(&path);
        assert!(
            schematic.total_blocks() > 0,
            "{} decoded to an empty build",
            path.display()
        );

        let (glb, info) = mesh_schematic(&schematic);
        assert!(!glb.is_empty(), "{} produced an empty GLB", path.display());
        assert!(
            info.triangle_count > 0,
            "{} produced no triangles",
            path.display()
        );
        // The GLB must carry a proper header with the magic, a version of 2,
        // and a total length that matches the allocation.
        assert!(
            glb.starts_with(b"glTF"),
            "{}: bad GLB magic",
            path.display()
        );
        let version = u32::from_le_bytes([glb[4], glb[5], glb[6], glb[7]]);
        assert_eq!(version, 2, "{}: unexpected GLB version", path.display());
        let declared = u32::from_le_bytes([glb[8], glb[9], glb[10], glb[11]]) as usize;
        assert_eq!(
            declared,
            glb.len(),
            "{}: GLB length mismatch",
            path.display()
        );
    }
}

#[test]
fn format_fixtures_decode() {
    for path in format_fixture_paths() {
        decode_file(&path);
    }
}
#[test]
fn mca_fixture_decodes_and_meshes() {
    let path = repo_root().join("Fixtures/Formats/Region.mca");
    let bytes = fs::read(&path).expect("read Region.mca");

    let decoded = decode(&bytes).expect("MCA decode should succeed");
    assert_eq!(decoded.total_blocks(), 4, "all 4 chunks should be represented");
    let (glb, info) = mesh(&decoded, &pack_bytes()).expect("MCA mesh should succeed");
    assert!(!glb.is_empty(), "GLB should not be empty");
    assert!(info.triangle_count > 0, "should produce triangles");
    assert!(glb.starts_with(b"glTF"), "valid GLB magic");
}

#[test]
fn unpadded_mca_decodes_and_meshes() {
    let path = repo_root().join("Fixtures/Formats/Region.mca");
    let mut bytes = fs::read(&path).expect("read Region.mca");
    // Truncate non-essential padding from the end of the file so its length
    // is not a multiple of 4096, mirroring real-world MCA files whose trailing
    // sector padding was omitted by export tools or file transfers.
    bytes.truncate(bytes.len() - 50);
    assert_ne!(bytes.len() % 4096, 0, "test file should not be sector-aligned");

    let decoded = decode(&bytes).expect("unpadded MCA should decode");
    assert_eq!(decoded.total_blocks(), 4);
    let (glb, info) = mesh(&decoded, &pack_bytes()).expect("unpadded MCA should mesh");
    assert!(info.triangle_count > 0);
    assert!(!glb.is_empty());
}

// ─── MCA regions ────────────────────────────────────────────────────────────

/// (location-table slot, record byte offset) for every populated entry.
fn populated_entries(region: &[u8]) -> Vec<(usize, usize)> {
    (0..1024)
        .map(|i| {
            let entry = i * 4;
            let sector_offset = ((region[entry] as usize) << 16)
                | ((region[entry + 1] as usize) << 8)
                | region[entry + 2] as usize;
            (i, sector_offset * 4096)
        })
        .filter(|&(_, byte_offset)| byte_offset >= 2 * 4096)
        .collect()
}

fn zlib_inflate(data: &[u8]) -> Vec<u8> {
    use std::io::Read;

    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data)
        .read_to_end(&mut out)
        .expect("fixture chunk payload should inflate");
    out
}

/// Frames `payload` the way vanilla's lz4-java `LZ4BlockOutputStream` does:
/// per-block `LZ4Block` magic, method token, little-endian compressed and
/// original lengths, a checksum the reader skips, then the data; finally the
/// zero-length raw endmark. Written independently of the decoder under test
/// so the framing is checked against the spec, not against itself.
fn lz4_java_frame(payload: &[u8]) -> Vec<u8> {
    const BLOCK_BYTES: usize = 1 << 16;

    let mut out = Vec::new();
    for chunk in payload.chunks(BLOCK_BYTES) {
        let compressed = lz4_flex::block::compress(chunk);
        out.extend_from_slice(b"LZ4Block");
        out.push(0x20);
        out.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&compressed);
    }
    out.extend_from_slice(b"LZ4Block");
    out.extend_from_slice(&[0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out
}

/// Rebuilds a region where the records of `lz4_slots` are LZ4-framed copies
/// of the fixture payloads and the rest stay zlib. The layout mirrors the
/// fixture's own: two header sectors, then each record at a fresh
/// sector-aligned offset.
fn region_with_lz4_chunks(lz4_slots: &[usize]) -> Vec<u8> {
    let fixture = fs::read(repo_root().join("Fixtures/Formats/Region.mca"))
        .expect("read Region.mca");

    let mut out = vec![0u8; 8192];
    let mut next_sector: u32 = 2;
    for (slot, byte_offset) in populated_entries(&fixture) {
        let record_len = u32::from_be_bytes([
            fixture[byte_offset],
            fixture[byte_offset + 1],
            fixture[byte_offset + 2],
            fixture[byte_offset + 3],
        ]) as usize;
        assert_eq!(fixture[byte_offset + 4], 2, "fixture chunks should be zlib");
        let payload = &fixture[byte_offset + 5..byte_offset + 4 + record_len];

        let record = if lz4_slots.contains(&slot) {
            let inflated = zlib_inflate(payload);
            let mut record = Vec::new();
            let framed = lz4_java_frame(&inflated);
            record.extend_from_slice(&(framed.len() as u32 + 1).to_be_bytes());
            record.push(4);
            record.extend_from_slice(&framed);
            record
        } else {
            fixture[byte_offset..byte_offset + 4 + record_len].to_vec()
        };

        let sector_count = (record.len() as u32).div_ceil(4096);
        let entry = slot * 4;
        out[entry..entry + 3].copy_from_slice(&next_sector.to_be_bytes()[1..4]);
        out[entry + 3] = sector_count as u8;
        let start = out.len();
        out.extend_from_slice(&record);
        out.resize(start + sector_count as usize * 4096, 0);
        next_sector += sector_count;
    }
    out
}

#[test]
fn mixed_lz4_region_decodes_and_meshes() {
    let region = region_with_lz4_chunks(&[0]);
    let (schematic, warnings) =
        decode_with_warnings(&region).expect("mixed-LZ4 region should decode");
    assert_eq!(schematic.total_blocks(), 4, "every chunk should survive");
    assert!(warnings.is_empty(), "no chunk should be skipped: {warnings:?}");

    let (glb, info) = mesh(&schematic, &pack_bytes()).expect("mixed-LZ4 region should mesh");
    assert!(info.triangle_count > 0);
    assert!(glb.starts_with(b"glTF"));
}

#[test]
fn fully_lz4_region_decodes() {
    let region = region_with_lz4_chunks(&[0, 1, 32, 33]);
    let (schematic, warnings) =
        decode_with_warnings(&region).expect("fully-LZ4 region should decode");
    assert_eq!(schematic.total_blocks(), 4);
    assert!(warnings.is_empty(), "no chunk should be skipped: {warnings:?}");
}

#[test]
fn external_chunk_reports_notice() {
    let mut region = fs::read(repo_root().join("Fixtures/Formats/Region.mca"))
        .expect("read Region.mca");
    // The spec's oversized-chunk spelling: length 1, compression value +128;
    // the payload lives in a c.x.z.mcc sibling the preview cannot reach.
    let (_, byte_offset) = populated_entries(&region)[0];
    region[byte_offset..byte_offset + 4].copy_from_slice(&1u32.to_be_bytes());
    region[byte_offset + 4] = 2 + 128;

    let (schematic, warnings) =
        decode_with_warnings(&region).expect("external chunk should skip, not fail");
    assert_eq!(schematic.total_blocks(), 3, "three chunks should remain");
    assert_eq!(warnings.len(), 1, "one notice expected: {warnings:?}");
    assert!(warnings[0].contains("c.0.0.mcc"), "notice names the .mcc: {}", warnings[0]);
}

#[test]
fn corrupt_chunk_reports_notice() {
    let mut region = fs::read(repo_root().join("Fixtures/Formats/Region.mca"))
        .expect("read Region.mca");
    // Point one populated entry past the end of the file: the chunk is in the
    // table but its record is unreadable.
    let (slot, _) = populated_entries(&region)[0];
    let entry = slot * 4;
    region[entry] = 0xff;
    region[entry + 1] = 0xff;
    region[entry + 2] = 0xff;

    let (schematic, warnings) =
        decode_with_warnings(&region).expect("corrupt chunk should skip, not fail");
    assert_eq!(schematic.total_blocks(), 3, "three chunks should remain");
    assert_eq!(warnings.len(), 1, "one notice expected: {warnings:?}");
    assert!(
        warnings[0].contains("could not be read"),
        "notice explains the skip: {}",
        warnings[0]
    );
}

#[test]
fn warnings_cross_the_ffi() {
    use litematicaql_native::{nql_schematic_free, nql_schematic_open, nql_schematic_warnings};

    let mut region = fs::read(repo_root().join("Fixtures/Formats/Region.mca"))
        .expect("read Region.mca");
    let (_, byte_offset) = populated_entries(&region)[0];
    region[byte_offset..byte_offset + 4].copy_from_slice(&1u32.to_be_bytes());
    region[byte_offset + 4] = 2 + 128;

    let mut handle = std::ptr::null_mut();
    let mut failure = litematicaql_native::NQLError {
        message: std::ptr::null_mut(),
        message_len: 0,
    };
    let status = unsafe {
        nql_schematic_open(region.as_ptr(), region.len(), &mut handle, &mut failure)
    };
    assert_eq!(status, status::OK, "the region should decode");
    assert!(!handle.is_null());

    let mut buffer: *mut u8 = std::ptr::null_mut();
    let mut length = 0;
    let status = unsafe { nql_schematic_warnings(handle, &mut buffer, &mut length) };
    assert_eq!(status, status::OK);
    assert!(length > 0, "the external-chunk notice should cross the FFI");
    let text = String::from_utf8(
        unsafe { std::slice::from_raw_parts(buffer, length) }.to_vec(),
    )
    .expect("notices are UTF-8");
    assert!(text.contains("c.0.0.mcc"));

    unsafe { litematicaql_native::nql_buffer_free(buffer, length) };
    unsafe { nql_schematic_free(handle) };
}

#[test]
fn mesh_failures_return_status_codes_not_panics() {
    let schematic = decode_file(
        &demo_paths()
            .iter()
            .find(|p| p.extension().unwrap() == "litematic")
            .expect("litematic demo"),
    );

    // A garbage pack must fail cleanly. The ABI reports a status and message
    // instead of unwinding across the boundary.
    let failure = mesh(&schematic, b"not a zip");
    match failure {
        Err(litematicaql_native::MeshFailure::Pack(_)) => {}
        other => panic!("expected a pack failure, got {other:?}"),
    }

    // An empty schematic cannot mesh. Nucleation refuses it outright.
    let empty = UniversalSchematic::new("empty".to_string());
    assert!(matches!(
        mesh(&empty, &pack_bytes()),
        Err(litematicaql_native::MeshFailure::NoBlocks)
    ));

    // The status module values must stay aligned with the C header.
    assert_eq!(status::OK, 0);
    assert_eq!(status::ERR_INTERNAL, 7);

    // The export helper keeps capacity equal to length so the host can free
    // with just a pointer and a length.
    let bytes = vec![7_u8; 100];
    let (pointer, length) = export_bytes(bytes);
    assert_eq!(length, 100);
    unsafe { litematicaql_native::nql_buffer_free(pointer, length) };
}
