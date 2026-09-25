use std::fs;
use std::path::PathBuf;

use litematicaql_native::{
    nql_buffer_free, nql_resource_pack_free, nql_resource_pack_open, nql_schematic_cancel,
    nql_schematic_free, nql_schematic_info, nql_schematic_mesh, nql_schematic_open,
    nql_schematic_warnings, status, NQLError, NQLMeshInfo, NQLSchematicInfo,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn pack_bytes() -> Vec<u8> {
    fs::read(repo_root().join("Renderer/vendor/pack.zip")).expect("bundled resource pack")
}

fn fixture_paths(directory: &str) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(repo_root().join(directory))
        .expect("fixture directory")
        .map(|entry| entry.expect("fixture entry").path())
        .collect();
    paths.sort();
    paths
}

fn error() -> NQLError {
    NQLError {
        message: std::ptr::null_mut(),
        message_len: 0,
    }
}

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

fn open(bytes: &[u8]) -> (*mut litematicaql_native::NQLSchematic, String) {
    let mut handle = std::ptr::null_mut();
    let mut failure = error();
    let status =
        unsafe { nql_schematic_open(bytes.as_ptr(), bytes.len(), &mut handle, &mut failure) };
    let message = take_error(&mut failure);
    assert_eq!(status, status::OK, "{message}");
    assert!(!handle.is_null());
    (handle, message)
}

fn open_result(bytes: &[u8]) -> Result<(*mut litematicaql_native::NQLSchematic, String), String> {
    let mut handle = std::ptr::null_mut();
    let mut failure = error();
    let status =
        unsafe { nql_schematic_open(bytes.as_ptr(), bytes.len(), &mut handle, &mut failure) };
    let message = take_error(&mut failure);
    if status == status::OK && !handle.is_null() {
        Ok((handle, message))
    } else {
        Err(message)
    }
}

fn open_pack(bytes: &[u8]) -> *mut litematicaql_native::NQLResourcePack {
    let mut handle = std::ptr::null_mut();
    let mut failure = error();
    let status =
        unsafe { nql_resource_pack_open(bytes.as_ptr(), bytes.len(), &mut handle, &mut failure) };
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
    assert_eq!(unsafe { nql_schematic_info(handle, &mut info) }, status::OK);
    info
}

fn warnings(handle: *const litematicaql_native::NQLSchematic) -> Vec<String> {
    let mut buffer = std::ptr::null_mut();
    let mut length = 0;
    assert_eq!(
        unsafe { nql_schematic_warnings(handle, &mut buffer, &mut length) },
        status::OK
    );
    if length == 0 {
        defer_free(buffer, length);
        return Vec::new();
    }
    let text = String::from_utf8(unsafe { std::slice::from_raw_parts(buffer, length) }.to_vec())
        .expect("notices are UTF-8");
    defer_free(buffer, length);
    text.lines().map(str::to_owned).collect()
}

fn defer_free(buffer: *mut u8, length: usize) {
    if !buffer.is_null() {
        unsafe { nql_buffer_free(buffer, length) };
    }
}

fn mesh(
    handle: *const litematicaql_native::NQLSchematic,
    pack: *const litematicaql_native::NQLResourcePack,
) -> (Vec<u8>, NQLMeshInfo) {
    let mut glb = std::ptr::null_mut();
    let mut length = 0;
    let mut mesh_info = NQLMeshInfo { triangle_count: 0 };
    let mut failure = error();
    let status = unsafe {
        nql_schematic_mesh(
            handle,
            pack,
            &mut glb,
            &mut length,
            &mut mesh_info,
            &mut failure,
        )
    };
    let message = take_error(&mut failure);
    assert_eq!(status, status::OK, "{message}");
    assert!(!glb.is_null());
    let bytes = unsafe { std::slice::from_raw_parts(glb, length) }.to_vec();
    unsafe { nql_buffer_free(glb, length) };
    (bytes, mesh_info)
}
fn assert_valid_glb(bytes: &[u8]) {
    assert_eq!(&bytes[..4], b"glTF");
    assert_eq!(u32::from_le_bytes(bytes[4..8].try_into().unwrap()), 2);
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
        bytes.len()
    );
}

fn deterministic_fixture() -> Vec<u8> {
    fs::read(repo_root().join("Fixtures/Demos/Cottage.litematic")).expect("litematic demo")
}

#[test]
fn ordered_chunk_output_is_deterministic() {
    let bytes = deterministic_fixture();
    let pack = open_pack(&pack_bytes());
    let (handle, _) = open(&bytes);
    let first = mesh(handle, pack);
    let second = mesh(handle, pack);
    assert_eq!(first.1, second.1);
    assert_valid_glb(&first.0);
    assert_valid_glb(&second.0);
    unsafe {
        nql_schematic_free(handle);
        nql_resource_pack_free(pack);
    }
}

#[test]
fn cancellation_returns_no_geometry_through_the_c_abi() {
    let bytes = deterministic_fixture();
    let pack = open_pack(&pack_bytes());
    let (handle, _) = open(&bytes);
    assert_eq!(unsafe { nql_schematic_cancel(handle) }, status::OK);
    let mut glb = std::ptr::null_mut();
    let mut length = 1;
    let mut mesh_info = NQLMeshInfo { triangle_count: 99 };
    let mut failure = error();
    let status = unsafe {
        nql_schematic_mesh(
            handle,
            pack,
            &mut glb,
            &mut length,
            &mut mesh_info,
            &mut failure,
        )
    };
    assert_eq!(status, status::ERR_CANCELLED);
    assert!(glb.is_null());
    assert_eq!(length, 0);
    take_error(&mut failure);
    unsafe {
        nql_schematic_free(handle);
        nql_resource_pack_free(pack);
    }
}

#[test]
fn every_format_and_demo_fixture_uses_the_c_abi() {
    let pack = open_pack(&pack_bytes());
    for directory in ["Fixtures/Formats", "Fixtures/Demos"] {
        for path in fixture_paths(directory) {
            let bytes = fs::read(&path).expect("fixture bytes");
            let (handle, message) =
                open_result(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(message.is_empty());
            let facts = info(handle);
            if path.extension().is_some_and(|extension| extension == "mca") {
                assert_eq!(facts.block_count, 4, "{}", path.display());
            } else {
                assert!(facts.block_count > 0, "{}", path.display());
            }
            let (glb, mesh_info) = mesh(handle, pack);
            assert!(mesh_info.triangle_count > 0, "{}", path.display());
            assert_valid_glb(&glb);
        }
    }
    unsafe { nql_resource_pack_free(pack) };
}

fn populated_entries(region: &[u8]) -> Vec<(usize, usize)> {
    (0..1024)
        .map(|index| {
            let entry = index * 4;
            let sector = ((region[entry] as usize) << 16)
                | ((region[entry + 1] as usize) << 8)
                | region[entry + 2] as usize;
            (index, sector * 4096)
        })
        .filter(|&(_, offset)| offset >= 8192)
        .collect()
}

#[test]
fn mca_notices_and_custom_lz4_survive_the_c_abi() {
    use std::io::Read;

    let fixture = fs::read(repo_root().join("Fixtures/Formats/Region.mca")).expect("MCA fixture");
    let mut external = fixture.clone();
    let (_, offset) = populated_entries(&external)[0];
    external[offset..offset + 4].copy_from_slice(&1u32.to_be_bytes());
    external[offset + 4] = 2 + 128;
    let (handle, _) = open(&external);
    let notices = warnings(handle);
    assert_eq!(info(handle).block_count, 3);
    assert!(notices.iter().any(|notice| notice.contains("c.0.0.mcc")));
    unsafe { nql_schematic_free(handle) };

    let mut lz4 = vec![0u8; 8192];
    let mut next_sector = 2u32;
    for (slot, source_offset) in populated_entries(&fixture) {
        let record_len = u32::from_be_bytes(
            fixture[source_offset..source_offset + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let mut inflated = Vec::new();
        flate2::read::ZlibDecoder::new(&fixture[source_offset + 5..source_offset + 4 + record_len])
            .read_to_end(&mut inflated)
            .expect("inflate MCA fixture");
        let mut framed = Vec::new();
        for block in inflated.chunks(1 << 16) {
            let compressed = lz4_flex::block::compress(block);
            framed.extend_from_slice(b"LZ4Block");
            framed.push(0x20);
            framed.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
            framed.extend_from_slice(&(block.len() as u32).to_le_bytes());
            framed.extend_from_slice(&0u32.to_le_bytes());
            framed.extend_from_slice(&compressed);
        }
        framed.extend_from_slice(b"LZ4Block");
        framed.extend_from_slice(&[0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut record = ((framed.len() + 1) as u32).to_be_bytes().to_vec();
        record.push(4);
        record.extend_from_slice(&framed);
        let sectors = record.len().div_ceil(4096) as u32;
        let entry = slot * 4;
        lz4[entry..entry + 3].copy_from_slice(&next_sector.to_be_bytes()[1..4]);
        lz4[entry + 3] = sectors as u8;
        let start = lz4.len();
        lz4.extend_from_slice(&record);
        lz4.resize(start + sectors as usize * 4096, 0);
        next_sector += sectors;
    }
    let pack = open_pack(&pack_bytes());
    let (handle, _) = open(&lz4);
    assert_eq!(info(handle).block_count, 4);
    assert!(warnings(handle).is_empty());
    assert_valid_glb(&mesh(handle, pack).0);
    unsafe {
        nql_schematic_free(handle);
        nql_resource_pack_free(pack);
    }
}

#[test]
fn pack_failures_keep_stable_status_codes() {
    let bad_pack = b"not a zip";
    let mut pack = std::ptr::null_mut();
    let mut failure = error();
    let result = unsafe {
        nql_resource_pack_open(bad_pack.as_ptr(), bad_pack.len(), &mut pack, &mut failure)
    };
    assert_eq!(result, status::ERR_PACK);
    take_error(&mut failure);
}
