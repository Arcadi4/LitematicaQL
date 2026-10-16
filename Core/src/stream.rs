//! Thin C ABI adapter that gives the host ownership of the shared engine's
//! upload-ready batches.
//!
//! Each returned batch owns one core [`MeshBatch`]: packed geometry, part draws,
//! and the RGBA pixels of every texture this stream introduces for the first
//! time. Packing, quantization, bounds, and texture identity belong to
//! litematica-preview-core, so nothing here meshes or assigns texture indices.
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::{MeshFailure, NQLAtlasInfo, NQLBatchInfo, NQLMeshInfo};
use litematica_preview_core::{MeshBatch, MeshStream};

pub use litematica_preview_core::{MeshPart as NQLMeshPart, VERTEX_STRIDE};

/// One first-use RGBA8 sRGB texture. The pixels come from the batch that
/// introduced them and stay valid until the host frees that batch. Texture zero
/// is the clamped atlas, and the open call delivers it once. Every other
/// texture repeats and can generate mipmaps.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NQLBatchTexture {
    pub index: u32,
    pub width: u32,
    pub height: u32,
    pub repeats: bool,
    pub pixels: *const u8,
    pub length: usize,
}

#[repr(C)]
pub struct NQLBatchView {
    pub geometry: *const u8,
    pub geometry_length: usize,
    pub parts: *const NQLMeshPart,
    pub part_count: usize,
    /// Textures that this batch introduces.
    pub textures: *const NQLBatchTexture,
    pub texture_count: usize,
    /// Decode UNORM16 positions in the vertex shader with these actual bounds.
    pub origin: [f32; 3],
    pub extent: [f32; 3],
}

pub struct NQLMeshBatch {
    batch: MeshBatch,
    textures: Vec<NQLBatchTexture>,
    pub(crate) info: NQLBatchInfo,
}

impl NQLMeshBatch {
    pub fn view(&self) -> NQLBatchView {
        NQLBatchView {
            geometry: self.batch.geometry.as_ptr(),
            geometry_length: self.batch.geometry.len(),
            parts: self.batch.parts.as_ptr(),
            part_count: self.batch.parts.len(),
            textures: self.textures.as_ptr(),
            texture_count: self.textures.len(),
            origin: self.batch.origin,
            extent: self.batch.extent,
        }
    }
}

pub struct NQLMeshStream<'a> {
    engine: MeshStream<'a>,
    next_batch: u32,
    cancelled: Arc<AtomicBool>,
}

impl<'a> NQLMeshStream<'a> {
    pub(crate) fn open(
        source: &'a crate::NQLSchematic,
        pack: &'a crate::NQLResourcePack,
        current: &impl Fn() -> Result<(), String>,
        quotas: crate::MeshQuotas,
    ) -> Result<(Self, Vec<u8>, NQLAtlasInfo, NQLMeshInfo), MeshFailure> {
        current().map_err(|_| MeshFailure::Cancelled)?;
        if source.source.block_count() > quotas.block_budget {
            return Err(MeshFailure::Limit(format!(
                "This schematic renders {} blocks; the preview limit is {}.",
                crate::grouped(source.source.block_count() as u64),
                crate::grouped(quotas.block_budget as u64)
            )));
        }
        let mut engine = MeshStream::new(
            &source.source,
            &pack.0,
            &crate::mesh_config(quotas.atlas_max_side),
            crate::worker_count(),
            current,
        )
        .map_err(|error| {
            if source.cancelled.load(Ordering::Relaxed) {
                MeshFailure::Cancelled
            } else {
                MeshFailure::Mesh(error)
            }
        })?;
        // `take_atlas` consumes the atlas delivery. The shared engine then keeps
        // only texture identity, and the host owns the pixels through the buffer
        // that this open call returns.
        let atlas = engine.take_atlas();
        let atlas_info = NQLAtlasInfo {
            width: atlas.width,
            height: atlas.height,
        };
        let batch_count = u32::try_from(engine.batch_count())
            .map_err(|_| MeshFailure::Mesh("Too many mesh batches.".into()))?;
        current().map_err(|_| MeshFailure::Cancelled)?;
        Ok((
            Self {
                engine,
                next_batch: 0,
                cancelled: source.cancelled.clone(),
            },
            atlas.pixels,
            atlas_info,
            NQLMeshInfo {
                batch_count,
                triangle_count: 0,
            },
        ))
    }

    pub(crate) fn next(
        &mut self,
        expected_batch: u32,
    ) -> Result<Option<NQLMeshBatch>, MeshFailure> {
        self.ensure_current()?;
        if expected_batch != self.next_batch {
            return Err(MeshFailure::Mesh(
                "Mesh batches arrived out of order.".into(),
            ));
        }
        let cancelled = &self.cancelled;
        let current = || {
            if cancelled.load(Ordering::Relaxed) {
                Err("Preview cancelled.".into())
            } else {
                Ok(())
            }
        };
        let Some(batch) = self.engine.next(&current).map_err(|error| {
            if cancelled.load(Ordering::Relaxed) {
                MeshFailure::Cancelled
            } else {
                MeshFailure::Mesh(error)
            }
        })?
        else {
            return Ok(None);
        };
        self.ensure_current()?;
        let batch = own(self.next_batch, batch)?;
        self.ensure_current()?;
        self.next_batch += 1;
        Ok(Some(batch))
    }

    fn ensure_current(&self) -> Result<(), MeshFailure> {
        if self.cancelled.load(Ordering::Relaxed) {
            Err(MeshFailure::Cancelled)
        } else {
            Ok(())
        }
    }
}

fn own(index: u32, batch: MeshBatch) -> Result<NQLMeshBatch, MeshFailure> {
    let payload_length = batch
        .geometry
        .len()
        .checked_add(
            batch
                .textures
                .iter()
                .map(|texture| texture.pixels.len())
                .sum(),
        )
        .ok_or_else(|| MeshFailure::Mesh("Mesh batch is too large.".into()))?;
    let info = NQLBatchInfo {
        batch_index: index,
        part_count: batch.info.part_count,
        vertex_count: batch.vertex_count,
        index_count: batch.index_count,
        triangle_count: count(batch.index_count as usize / 3)?,
        payload_length: count(payload_length)?,
        bounds_min: batch.source_min,
        bounds_max: batch.source_max,
    };
    let textures = batch
        .textures
        .iter()
        .map(|texture| NQLBatchTexture {
            index: texture.index,
            width: texture.width,
            height: texture.height,
            repeats: texture.repeat,
            pixels: texture.pixels.as_ptr(),
            length: texture.pixels.len(),
        })
        .collect();
    Ok(NQLMeshBatch {
        batch,
        textures,
        info,
    })
}

fn count(value: usize) -> Result<u32, MeshFailure> {
    u32::try_from(value).map_err(|_| MeshFailure::Mesh("Mesh batch is too large.".into()))
}
