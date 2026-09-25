use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use litematicaql_native::{
    nql_schematic_free, nql_schematic_info, nql_schematic_open, status, NQLError, NQLSchematicInfo,
};
use quartz_nbt::{NbtCompound, NbtList, NbtTag};

struct AllocationMeter;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn allocated(bytes: usize) {
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for AllocationMeter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            allocated(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(pointer, layout, size) };
        if !next.is_null() {
            if size >= layout.size() {
                allocated(size - layout.size());
            } else {
                LIVE.fetch_sub(layout.size() - size, Ordering::Relaxed);
            }
        }
        next
    }
}

#[global_allocator]
static ALLOCATOR: AllocationMeter = AllocationMeter;

fn sparse_litematic(width: usize) -> Vec<u8> {
    let mut size = NbtCompound::new();
    size.insert("x", width as i32);
    size.insert("y", 32i32);
    size.insert("z", width as i32);
    let mut position = NbtCompound::new();
    for axis in ["x", "y", "z"] {
        position.insert(axis, 0i32);
    }
    let palette: Vec<_> = ["minecraft:air", "minecraft:stone"]
        .into_iter()
        .map(|name| {
            let mut state = NbtCompound::new();
            state.insert("Name", name);
            NbtTag::Compound(state)
        })
        .collect();
    let mut packed = vec![0i64; (width * 32 * width * 2).div_ceil(64)];
    for (x, y, z) in [(1, 2, 3), (73, 17, 131), (width - 6, 29, width - 7)] {
        let index = x + z * width + y * width * width;
        packed[index / 32] |= 1i64 << ((index % 32) * 2);
    }
    let mut region = NbtCompound::new();
    region.insert("Size", size);
    region.insert("Position", position);
    region.insert("BlockStatePalette", NbtList::from(palette));
    region.insert("BlockStates", NbtTag::LongArray(packed));
    let mut regions = NbtCompound::new();
    regions.insert("Main", region);
    let mut root = NbtCompound::new();
    root.insert("Version", 6i32);
    root.insert("Metadata", NbtCompound::new());
    root.insert("Regions", regions);
    let mut bytes = Vec::new();
    quartz_nbt::io::write_nbt(
        &mut bytes,
        None,
        &root,
        quartz_nbt::io::Flavor::GzCompressed,
    )
    .unwrap();
    bytes
}

#[test]
fn sparse_litematic_never_materializes_its_dense_volume() {
    for width in [256, 512] {
        let bytes = sparse_litematic(width);
        let baseline = LIVE.load(Ordering::Relaxed);
        PEAK.store(baseline, Ordering::Relaxed);
        let mut handle = std::ptr::null_mut();
        let mut failure = NQLError {
            message: std::ptr::null_mut(),
            message_len: 0,
        };
        let result =
            unsafe { nql_schematic_open(bytes.as_ptr(), bytes.len(), &mut handle, &mut failure) };
        assert_eq!(result, status::OK);
        let mut info = NQLSchematicInfo {
            block_count: 0,
            block_entity_count: 0,
            content_x: 0,
            content_y: 0,
            content_z: 0,
        };
        assert_eq!(unsafe { nql_schematic_info(handle, &mut info) }, status::OK);
        assert_eq!(info.block_count, 3);
        let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
        unsafe { nql_schematic_free(handle) };
        assert!(
            peak < 1024 * 1024,
            "sparse {width} decode retained {peak} bytes"
        );
    }
}
