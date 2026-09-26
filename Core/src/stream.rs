//! Bounded native mesh streaming.
//!
//! A stream owns the mesher's shared atlas and emits one geometry payload at a
//! time. The atlas is returned by `open`; batch payloads contain only typed
//! vertex/index data and first-use textures for greedy materials. Batch
//! generation uses a small ordered worker window before payloads are encoded.
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use schematic_mesher::mesh_output::GreedyMaterialOutput;
use schematic_mesher::{MeshLayer, MeshOutput};

use crate::meshing::ChunkMeshes;
use crate::{MeshFailure, NQLAtlasInfo, NQLBatchInfo, NQLMeshInfo};

const BATCH_MAGIC: &[u8; 4] = b"LQMB";
const BATCH_VERSION: u16 = 1;
const BATCH_HEADER_BYTES: usize = 64;
const PART_HEADER_BYTES: usize = 24;
const ALPHA_OPAQUE: u32 = 0;
const ALPHA_MASK: u32 = 1;
const ALPHA_BLEND: u32 = 2;
const REPEAT_TEXTURE: u32 = 0x100;

pub struct NQLMeshStream<'a> {
    chunks: ChunkMeshes<'a>,
    next_batch: u32,
    pending: Vec<MeshOutput>,
    textures: HashMap<String, u32>,
    next_texture: u32,
    cancelled: Arc<AtomicBool>,
}

pub(crate) struct BatchPayload {
    pub bytes: Vec<u8>,
    pub info: NQLBatchInfo,
}

impl<'a> NQLMeshStream<'a> {
    pub(crate) fn open(
        source: &'a crate::NQLSchematic,
        pack: &'a crate::NQLResourcePack,
        current: &impl Fn() -> Result<(), String>,
    ) -> Result<(Self, Vec<u8>, NQLAtlasInfo, NQLMeshInfo), MeshFailure> {
        let block_count = source.source.block_count();
        if block_count > crate::MAX_MESH_BLOCKS {
            return Err(MeshFailure::Mesh(format!(
                "This schematic renders {block_count} blocks, beyond the {}-block preview limit.",
                crate::MAX_MESH_BLOCKS
            )));
        }
        let chunks =
            ChunkMeshes::from_source(&source.source, &pack.0, &crate::mesh_config(), current)
                .map_err(MeshFailure::Mesh)?;
        let atlas = chunks.atlas_png().map_err(MeshFailure::Mesh)?;
        let atlas_info = NQLAtlasInfo {
            width: chunks.atlas_width(),
            height: chunks.atlas_height(),
        };
        let batch_count = chunks.batch_count();
        current().map_err(|_| MeshFailure::Cancelled)?;
        let stream = Self {
            chunks,
            next_batch: 0,
            pending: Vec::new(),
            textures: HashMap::new(),
            next_texture: 1,
            cancelled: source.cancelled.clone(),
        };
        let info = NQLMeshInfo {
            batch_count: u32::try_from(batch_count).map_err(|_| {
                MeshFailure::Mesh("This schematic has too many mesh batches.".into())
            })?,
            triangle_count: 0,
        };
        Ok((stream, atlas, atlas_info, info))
    }

    pub(crate) fn next(
        &mut self,
        expected_batch: u32,
    ) -> Result<Option<BatchPayload>, MeshFailure> {
        self.ensure_current()?;
        if expected_batch != self.next_batch {
            return Err(MeshFailure::Mesh(
                "Mesh batches arrived out of order.".into(),
            ));
        }
        if usize::try_from(self.next_batch).unwrap_or(usize::MAX) >= self.chunks.batch_count() {
            return Ok(None);
        }
        if self.pending.is_empty() {
            let start = self.next_batch;
            let end = start
                .saturating_add(crate::worker_count() as u32)
                .min(self.chunks.batch_count() as u32);
            if start >= end {
                return Ok(None);
            }
            let mut generated = Vec::new();
            crate::parallel::ordered(
                (start..end).map(Ok),
                crate::worker_count(),
                true,
                |index, cancelled| {
                    crate::parallel::check_cancelled(cancelled)?;
                    self.chunks.mesh_at(index as usize)
                },
                |output| {
                    generated.push(output);
                    Ok(())
                },
                &|| Ok(()),
            )
            .map_err(MeshFailure::Mesh)?;
            self.pending = generated;
        }
        let mut output = self.pending.remove(0);
        output.greedy_materials.sort_by(|left, right| {
            left.texture_path
                .cmp(&right.texture_path)
                .then_with(|| left.texture_png.cmp(&right.texture_png))
        });
        self.ensure_current()?;
        let bytes = encode_batch(
            self.next_batch,
            &output,
            &mut self.textures,
            &mut self.next_texture,
        )?;
        self.ensure_current()?;
        let info = batch_info(self.next_batch, &bytes, &output)?;
        self.next_batch = self
            .next_batch
            .checked_add(1)
            .ok_or_else(|| MeshFailure::Mesh("This schematic has too many mesh batches.".into()))?;
        Ok(Some(BatchPayload { bytes, info }))
    }

    fn ensure_current(&self) -> Result<(), MeshFailure> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(MeshFailure::Cancelled)
        } else {
            Ok(())
        }
    }
}

fn encode_batch(
    batch_index: u32,
    output: &MeshOutput,
    texture_indices: &mut HashMap<String, u32>,
    next_texture: &mut u32,
) -> Result<Vec<u8>, MeshFailure> {
    let mut materials: Vec<_> = output
        .greedy_materials
        .iter()
        .filter(|material| !material.opaque.is_empty() || !material.transparent.is_empty())
        .collect();
    materials.sort_by(|left, right| {
        left.texture_path
            .cmp(&right.texture_path)
            .then_with(|| left.texture_png.cmp(&right.texture_png))
    });
    let part_count = layer_part_count(&output.opaque)
        + layer_part_count(&output.cutout)
        + layer_part_count(&output.transparent)
        + materials
            .iter()
            .map(|material| {
                layer_part_count(&material.opaque) + layer_part_count(&material.transparent)
            })
            .sum::<usize>();
    let vertex_count = output
        .opaque
        .vertex_count()
        .checked_add(output.cutout.vertex_count())
        .and_then(|count| count.checked_add(output.transparent.vertex_count()))
        .and_then(|count| {
            materials.iter().try_fold(count, |count, material| {
                count
                    .checked_add(material.opaque.vertex_count())
                    .and_then(|count| count.checked_add(material.transparent.vertex_count()))
            })
        })
        .ok_or_else(|| MeshFailure::Mesh("This schematic has too many vertices.".into()))?;
    let index_count = output
        .opaque
        .indices
        .len()
        .checked_add(output.cutout.indices.len())
        .and_then(|count| count.checked_add(output.transparent.indices.len()))
        .and_then(|count| {
            materials.iter().try_fold(count, |count, material| {
                count
                    .checked_add(material.opaque.indices.len())
                    .and_then(|count| count.checked_add(material.transparent.indices.len()))
            })
        })
        .ok_or_else(|| MeshFailure::Mesh("This schematic has too many indices.".into()))?;
    let triangle_count = index_count / 3;
    let mut bytes = Vec::with_capacity(BATCH_HEADER_BYTES + part_count * PART_HEADER_BYTES);
    bytes.extend_from_slice(BATCH_MAGIC);
    push_u16(&mut bytes, BATCH_VERSION);
    push_u16(&mut bytes, BATCH_HEADER_BYTES as u16);
    push_u32(&mut bytes, batch_index);
    push_u32(
        &mut bytes,
        u32::try_from(part_count).map_err(|_| MeshFailure::Mesh("Too many mesh parts.".into()))?,
    );
    push_u32(
        &mut bytes,
        u32::try_from(vertex_count).map_err(|_| MeshFailure::Mesh("Too many vertices.".into()))?,
    );
    push_u32(
        &mut bytes,
        u32::try_from(index_count).map_err(|_| MeshFailure::Mesh("Too many indices.".into()))?,
    );
    push_u32(
        &mut bytes,
        u32::try_from(triangle_count)
            .map_err(|_| MeshFailure::Mesh("Too many triangles.".into()))?,
    );
    for value in output.bounds.min {
        push_f32(&mut bytes, value);
    }
    for value in output.bounds.max {
        push_f32(&mut bytes, value);
    }
    push_u32(&mut bytes, *next_texture);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);

    append_layer(&mut bytes, &output.opaque, 0, ALPHA_OPAQUE, None)?;
    append_layer(&mut bytes, &output.cutout, 0, ALPHA_MASK, None)?;
    append_layer(&mut bytes, &output.transparent, 0, ALPHA_BLEND, None)?;
    for material in materials {
        let (texture_index, first_use) = texture_index(material, texture_indices, next_texture);
        let png = first_use.then_some(material.texture_png.as_slice());
        append_layer(
            &mut bytes,
            &material.opaque,
            texture_index,
            ALPHA_OPAQUE,
            if material.opaque.is_empty() {
                None
            } else {
                png
            },
        )?;
        append_layer(
            &mut bytes,
            &material.transparent,
            texture_index,
            ALPHA_BLEND,
            if material.transparent.is_empty() || !material.opaque.is_empty() {
                None
            } else {
                png
            },
        )?;
    }
    Ok(bytes)
}

fn layer_part_count(layer: &MeshLayer) -> usize {
    usize::from(!layer.is_empty())
}

fn append_layer(
    bytes: &mut Vec<u8>,
    layer: &MeshLayer,
    texture_index: u32,
    alpha_mode: u32,
    texture_png: Option<&[u8]>,
) -> Result<(), MeshFailure> {
    if layer.is_empty() {
        return Ok(());
    }
    let repeat = if texture_index == 0 {
        0
    } else {
        REPEAT_TEXTURE
    };
    let texture_len = texture_png.map_or(0, |png| u32::try_from(png.len()).unwrap_or(u32::MAX));
    push_u32(bytes, texture_index);
    push_u32(bytes, alpha_mode | repeat);
    push_u32(
        bytes,
        u32::try_from(layer.vertex_count())
            .map_err(|_| MeshFailure::Mesh("Too many vertices.".into()))?,
    );
    push_u32(
        bytes,
        u32::try_from(layer.indices.len())
            .map_err(|_| MeshFailure::Mesh("Too many indices.".into()))?,
    );
    push_u32(bytes, texture_len);
    push_u32(bytes, 0);
    bytes.extend_from_slice(layer.positions_bytes());
    bytes.extend_from_slice(layer.normals_bytes());
    bytes.extend_from_slice(layer.uvs_bytes());
    bytes.extend_from_slice(layer.colors_bytes());
    bytes.extend_from_slice(layer.indices_bytes());
    if let Some(png) = texture_png {
        if texture_len != png.len() as u32 {
            return Err(MeshFailure::Mesh("Texture is too large.".into()));
        }
        bytes.extend_from_slice(png);
        bytes.resize((bytes.len() + 3) & !3, 0);
    }
    Ok(())
}

fn texture_index(
    material: &GreedyMaterialOutput,
    texture_indices: &mut HashMap<String, u32>,
    next_texture: &mut u32,
) -> (u32, bool) {
    if let Some(index) = texture_indices.get(&material.texture_path).copied() {
        return (index, false);
    }
    let index = *next_texture;
    *next_texture = next_texture.saturating_add(1);
    texture_indices.insert(material.texture_path.clone(), index);
    (index, true)
}

fn batch_info(index: u32, bytes: &[u8], output: &MeshOutput) -> Result<NQLBatchInfo, MeshFailure> {
    let part_count = layer_part_count(&output.opaque)
        + layer_part_count(&output.cutout)
        + layer_part_count(&output.transparent)
        + output
            .greedy_materials
            .iter()
            .map(|material| {
                layer_part_count(&material.opaque) + layer_part_count(&material.transparent)
            })
            .sum::<usize>();
    let vertex_count = output
        .opaque
        .vertex_count()
        .checked_add(output.cutout.vertex_count())
        .and_then(|count| count.checked_add(output.transparent.vertex_count()))
        .and_then(|count| {
            output
                .greedy_materials
                .iter()
                .try_fold(count, |count, material| {
                    count
                        .checked_add(material.opaque.vertex_count())
                        .and_then(|count| count.checked_add(material.transparent.vertex_count()))
                })
        })
        .ok_or_else(|| MeshFailure::Mesh("This schematic has too many vertices.".into()))?;
    let index_count = output
        .opaque
        .indices
        .len()
        .checked_add(output.cutout.indices.len())
        .and_then(|count| count.checked_add(output.transparent.indices.len()))
        .and_then(|count| {
            output
                .greedy_materials
                .iter()
                .try_fold(count, |count, material| {
                    count
                        .checked_add(material.opaque.indices.len())
                        .and_then(|count| count.checked_add(material.transparent.indices.len()))
                })
        })
        .ok_or_else(|| MeshFailure::Mesh("This schematic has too many indices.".into()))?;
    Ok(NQLBatchInfo {
        batch_index: index,
        part_count: u32::try_from(part_count)
            .map_err(|_| MeshFailure::Mesh("Too many mesh parts.".into()))?,
        vertex_count: u32::try_from(vertex_count)
            .map_err(|_| MeshFailure::Mesh("Too many vertices.".into()))?,
        index_count: u32::try_from(index_count)
            .map_err(|_| MeshFailure::Mesh("Too many indices.".into()))?,
        triangle_count: u32::try_from(index_count / 3)
            .map_err(|_| MeshFailure::Mesh("Too many triangles.".into()))?,
        payload_length: u32::try_from(bytes.len())
            .map_err(|_| MeshFailure::Mesh("Mesh batch is too large.".into()))?,
        bounds_min: output.bounds.min,
        bounds_max: output.bounds.max,
    })
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
