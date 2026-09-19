// Integration tests for the preview pipeline the Swift bridge calls.
//
// Every bundled demo is decoded and meshed with the shipping configuration,
// one per file format, and every minimal format fixture is decoded.

use std::fs;
use std::path::{Path, PathBuf};

use litematicaql_native::status;
use litematicaql_native::{decode, export_bytes, mesh, NQLMeshInfo};
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
