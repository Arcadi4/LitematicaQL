//! Bounded mesh production in Metal's final vertex/index layout.
//!
//! Each returned batch owns one compact geometry allocation and first-use
//! greedy texture PNGs. No serialization or whole-model geometry is retained.
use std::collections::{HashMap, VecDeque};
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};

use schematic_mesher::{MeshLayer, MeshOutput};
use crate::meshing::ChunkMeshes;
use crate::{MeshFailure, NQLAtlasInfo, NQLBatchInfo, NQLMeshInfo};

pub const VERTEX_STRIDE: usize = 24;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NQLMeshPart {
    pub vertex_offset: usize,
    pub index_offset: usize,
    pub vertex_count: u32,
    pub index_count: u32,
    pub index_size: u32,
    /// 0 = opaque, 1 = alpha test, 2 = blend. Nonzero textures repeat.
    pub alpha_mode: u32,
    pub texture_index: u32,
    pub texture_png: *const u8,
    pub texture_length: usize,
}

#[repr(C)]
pub struct NQLBatchView {
    pub geometry: *const u8,
    pub geometry_length: usize,
    pub parts: *const NQLMeshPart,
    pub part_count: usize,
    /// Decode UNORM16 positions in the vertex shader using these actual bounds.
    pub origin: [f32; 3],
    pub extent: [f32; 3],
}

pub struct NQLMeshBatch {
    geometry: Vec<u8>,
    parts: Vec<NQLMeshPart>,
    _textures: Vec<Vec<u8>>,
    origin: [f32; 3],
    extent: [f32; 3],
    pub(crate) info: NQLBatchInfo,
}

impl NQLMeshBatch {
    pub fn view(&self) -> NQLBatchView {
        NQLBatchView {
            geometry: self.geometry.as_ptr(), geometry_length: self.geometry.len(),
            parts: self.parts.as_ptr(), part_count: self.parts.len(),
            origin: self.origin, extent: self.extent,
        }
    }
}

pub struct NQLMeshStream<'a> {
    chunks: ChunkMeshes<'a>,
    next_batch: u32,
    pending: VecDeque<MeshOutput>,
    textures: HashMap<String, u32>,
    cancelled: Arc<AtomicBool>,
}

impl<'a> NQLMeshStream<'a> {
    pub(crate) fn open(
        source: &'a crate::NQLSchematic,
        pack: &'a crate::NQLResourcePack,
        current: &impl Fn() -> Result<(), String>,
    ) -> Result<(Self, Vec<u8>, NQLAtlasInfo, NQLMeshInfo), MeshFailure> {
        let block_count = source.source.block_count();
        if block_count > crate::MAX_MESH_BLOCKS {
            return Err(MeshFailure::Mesh(format!("This schematic renders {block_count} blocks, beyond the preview limit.")));
        }
        let mut chunks = ChunkMeshes::from_source(&source.source, &pack.0, &crate::mesh_config(), current)
            .map_err(MeshFailure::Mesh)?;
        let atlas_info = NQLAtlasInfo { width: chunks.atlas_width(), height: chunks.atlas_height() };
        // Transfer the existing pixels. Metal uploads them once; PNG encoding
        // and immediately decoding a generated atlas would be wasted work.
        let atlas = chunks.take_atlas_pixels();
        let batch_count = u32::try_from(chunks.batch_count())
            .map_err(|_| MeshFailure::Mesh("Too many mesh batches.".into()))?;
        current().map_err(|_| MeshFailure::Cancelled)?;
        Ok((Self {
            chunks, next_batch: 0, pending: VecDeque::new(),
            textures: HashMap::new(), cancelled: source.cancelled.clone(),
        }, atlas, atlas_info, NQLMeshInfo { batch_count, triangle_count: 0 }))
    }

    pub(crate) fn next(&mut self, expected_batch: u32) -> Result<Option<NQLMeshBatch>, MeshFailure> {
        self.ensure_current()?;
        if expected_batch != self.next_batch {
            return Err(MeshFailure::Mesh("Mesh batches arrived out of order.".into()));
        }
        if self.next_batch as usize >= self.chunks.batch_count() { return Ok(None); }
        if self.pending.is_empty() {
            let end = self.next_batch.saturating_add(crate::worker_count() as u32)
                .min(self.chunks.batch_count() as u32);
            let mut generated = VecDeque::new();
            crate::parallel::ordered(
                (self.next_batch..end).map(Ok), crate::worker_count(), true,
                |index, cancelled| {
                    crate::parallel::check_cancelled(cancelled)?;
                    if self.cancelled.load(Ordering::Relaxed) { return Err("Preview cancelled.".into()); }
                    self.chunks.mesh_at(index as usize)
                },
                |output| { generated.push_back(output); Ok(()) },
                &|| if self.cancelled.load(Ordering::Relaxed) { Err("Preview cancelled.".into()) } else { Ok(()) },
            ).map_err(|error| if self.cancelled.load(Ordering::Relaxed) { MeshFailure::Cancelled } else { MeshFailure::Mesh(error) })?;
            self.pending = generated;
        }
        let output = self.pending.pop_front().expect("nonempty mesh window");
        self.ensure_current()?;
        let batch = pack_batch(self.next_batch, output, &mut self.textures)?;
        self.ensure_current()?;
        self.next_batch += 1;
        Ok(Some(batch))
    }

    fn ensure_current(&self) -> Result<(), MeshFailure> {
        if self.cancelled.load(Ordering::Relaxed) { Err(MeshFailure::Cancelled) } else { Ok(()) }
    }
}

fn pack_batch(index: u32, mut output: MeshOutput, textures: &mut HashMap<String, u32>) -> Result<NQLMeshBatch, MeshFailure> {
    output.greedy_materials.sort_by(|a, b| a.texture_path.cmp(&b.texture_path).then_with(|| a.texture_png.cmp(&b.texture_png)));
    let layers = [&output.opaque, &output.cutout, &output.transparent].into_iter()
        .chain(output.greedy_materials.iter().flat_map(|m| [&m.opaque, &m.transparent]));
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    let mut geometry_length = 0;
    let mut vertex_count = 0;
    let mut index_count = 0;
    for layer in layers.filter(|l| !l.is_empty()) {
        for position in &layer.positions {
            for axis in 0..3 {
                if !position[axis].is_finite() { return Err(MeshFailure::Mesh("Nonfinite mesh position.".into())); }
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
        vertex_count += layer.vertex_count();
        index_count += layer.indices.len();
        geometry_length += layer.vertex_count() * VERTEX_STRIDE + layer.indices.len() * index_size(layer);
        geometry_length = (geometry_length + 3) & !3;
    }
    if vertex_count == 0 { min = [0.0; 3]; max = [0.0; 3]; }
    let extent = std::array::from_fn(|axis| max[axis] - min[axis]);
    let mut batch = NQLMeshBatch {
        geometry: Vec::with_capacity(geometry_length), parts: Vec::new(), _textures: Vec::new(),
        origin: min, extent,
        info: NQLBatchInfo {
            batch_index: index, part_count: 0,
            vertex_count: count(vertex_count)?, index_count: count(index_count)?,
            triangle_count: count(index_count / 3)?, payload_length: 0,
            bounds_min: output.bounds.min, bounds_max: output.bounds.max,
        },
    };
    append_layer(&mut batch, output.opaque, 0, 0, None)?;
    append_layer(&mut batch, output.cutout, 0, 1, None)?;
    append_layer(&mut batch, output.transparent, 0, 2, None)?;
    for material in output.greedy_materials {
        if material.opaque.is_empty() && material.transparent.is_empty() { continue; }
        let next = count(textures.len() + 1)?;
        let first_use = !textures.contains_key(&material.texture_path);
        let texture = *textures.entry(material.texture_path).or_insert(next);
        let png = first_use.then_some(material.texture_png);
        if material.opaque.is_empty() {
            append_layer(&mut batch, material.transparent, texture, 2, png)?;
        } else {
            append_layer(&mut batch, material.opaque, texture, 0, png)?;
            append_layer(&mut batch, material.transparent, texture, 2, None)?;
        }
    }
    batch.info.part_count = count(batch.parts.len())?;
    batch.info.payload_length = count(batch.geometry.len() + batch._textures.iter().map(Vec::len).sum::<usize>())?;
    Ok(batch)
}

fn count(value: usize) -> Result<u32, MeshFailure> {
    u32::try_from(value).map_err(|_| MeshFailure::Mesh("Mesh batch is too large.".into()))
}

fn index_size(layer: &MeshLayer) -> usize { if layer.vertex_count() <= 65536 { 2 } else { 4 } }

fn append_layer(batch: &mut NQLMeshBatch, layer: MeshLayer, texture: u32, alpha: u32, png: Option<Vec<u8>>) -> Result<(), MeshFailure> {
    if layer.is_empty() { return Ok(()); }
    let vertex_offset = batch.geometry.len();
    // 24-byte interleaved vertices: UNORM16x4 position, float2 UV,
    // SNORM8x4 normal, UNORM8x4 color. Position precision is relative to this
    // chunk's actual geometry, including models extending past block bounds.
    for i in 0..layer.vertex_count() {
        for axis in 0..3 {
            let extent = batch.extent[axis];
            let value = if extent > 0.0 { (layer.positions[i][axis] - batch.origin[axis]) / extent } else { 0.0 };
            batch.geometry.extend_from_slice(&((value.clamp(0.0, 1.0) * 65535.0).round() as u16).to_le_bytes());
        }
        batch.geometry.extend_from_slice(&0u16.to_le_bytes());
        for uv in layer.uvs[i] { batch.geometry.extend_from_slice(&uv.to_le_bytes()); }
        for normal in layer.normals[i] { batch.geometry.push((normal.clamp(-1.0, 1.0) * 127.0).round() as i8 as u8); }
        batch.geometry.push(0);
        for color in layer.colors[i] { batch.geometry.push((color.clamp(0.0, 1.0) * 255.0).round() as u8); }
    }
    let index_offset = batch.geometry.len();
    let index_size = index_size(&layer);
    for &index in &layer.indices {
        if index_size == 2 { batch.geometry.extend_from_slice(&(index as u16).to_le_bytes()); }
        else { batch.geometry.extend_from_slice(&index.to_le_bytes()); }
    }
    batch.geometry.resize((batch.geometry.len() + 3) & !3, 0);
    let (texture_png, texture_length) = if let Some(png) = png {
        let result = (png.as_ptr(), png.len());
        batch._textures.push(png);
        result
    } else { (std::ptr::null(), 0) };
    batch.parts.push(NQLMeshPart {
        vertex_offset, index_offset, vertex_count: count(layer.vertex_count())?, index_count: count(layer.indices.len())?,
        index_size: index_size as u32, alpha_mode: alpha, texture_index: texture, texture_png, texture_length,
    });
    Ok(())
}
