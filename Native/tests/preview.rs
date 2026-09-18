// Integration tests for the preview pipeline the Swift bridge calls.
//
// Every bundled demo is decoded and meshed with the shipping configuration,
// one per file format, and every minimal format fixture is decoded. The
// Valkyrie test is opt-in: it needs a large local build and takes minutes.

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

// The build that exposed the wasm ceiling: a 1003 × 259 × 1002 station with
// 4.4 million non-air blocks. Opt-in because it needs the local file and
// takes about a minute.
#[test]
#[ignore = "needs a large local build; run with `cargo test --release -- --ignored`"]
fn valkyrie_meshes_natively() {
    let path = std::env::var("LQL_VALKYRIE")
        .unwrap_or_else(|_| "/Users/skylar/Desktop/valkyrie.litematic".to_string());
    let path = PathBuf::from(path);
    if !path.exists() {
        panic!("set LQL_VALKYRIE to a large .litematic to run this test");
    }

    let started = std::time::Instant::now();
    let schematic = decode_file(&path);
    let decode_seconds = started.elapsed().as_secs_f32();
    assert!(
        schematic.total_blocks() > 4_000_000,
        "region decode silently dropped blocks"
    );

    let started = std::time::Instant::now();
    let (glb, info) = mesh_schematic(&schematic);
    let mesh_seconds = started.elapsed().as_secs_f32();

    assert!(info.triangle_count > 1_000_000, "too few triangles");
    assert!(!glb.is_empty());
    let out = std::env::var("LQL_VALKYRIE_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("valkyrie-native.glb"));
    fs::write(&out, &glb).expect("write GLB");
    println!(
        "decode {decode_seconds:.1}s, mesh {mesh_seconds:.1}s, triangles {}, glb {} MiB, wrote {}",
        info.triangle_count,
        glb.len() / 1_048_576,
        out.display()
    );
}
