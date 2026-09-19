// Minecraft Anvil `.mca` region decoding.
//
// An MCA region file stores up to 1024 chunks in a 32x32 grid. A full region
// can easily span 100+ million blocks, which is far beyond the preview limit.
// This loader scans the region location table, selects a cohesive 4-chunk
// parcel (preferring a 2x2 chunk window centered near the populated chunks),
// reads only those chunks via `RegionReader`, and normalizes their coordinates
// into a single previewable schematic.

use std::collections::HashSet;
use std::io::Cursor;

use nucleation::block_position::BlockPosition;
use nucleation::formats::anvil::{ChunkData, RegionReader};
use nucleation::{Region, UniversalSchematic};

use super::DecodeFailure;

pub(super) fn is_mca(data: &[u8]) -> bool {
    const SECTOR_BYTES: usize = 4096;
    const HEADER_BYTES: usize = 2 * SECTOR_BYTES;

    if data.len() < HEADER_BYTES || data.starts_with(b"PK\x03\x04") {
        return false;
    }

    for i in 0..1024 {
        let offset = i * 4;
        let sector_offset = ((data[offset] as usize) << 16)
            | ((data[offset + 1] as usize) << 8)
            | data[offset + 2] as usize;
        let sector_count = data[offset + 3] as usize;

        if sector_offset == 0 && sector_count == 0 {
            continue;
        }
        if sector_offset < 2 || sector_count == 0 {
            return false;
        }

        let Some(byte_offset) = sector_offset.checked_mul(SECTOR_BYTES) else {
            return false;
        };
        if byte_offset + 5 > data.len() {
            continue;
        }

        let chunk_len = u32::from_be_bytes([
            data[byte_offset],
            data[byte_offset + 1],
            data[byte_offset + 2],
            data[byte_offset + 3],
        ]) as usize;

        if chunk_len <= 1 || !matches!(data[byte_offset + 4], 1..=4) {
            continue;
        }

        if byte_offset + 4 + chunk_len <= data.len() {
            return true;
        }
    }

    false
}

fn is_air(name: &str) -> bool {
    matches!(
        name,
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    )
}

pub(super) fn load_mca_preview(bytes: &[u8]) -> Result<UniversalSchematic, DecodeFailure> {
    let mut reader = RegionReader::new_auto(Cursor::new(bytes)).map_err(|error| {
        DecodeFailure::Format(format!("This file is not a readable MCA region: {error}"))
    })?;

    let populated_positions = reader.chunk_positions();
    if populated_positions.is_empty() {
        return Err(DecodeFailure::Format(
            "This MCA file contains no chunk data.".to_string(),
        ));
    }

    let (region_x, region_z) = reader.region_position();
    let selected_coords = select_preview_chunks(&populated_positions, region_x, region_z);

    let mut chunks: Vec<ChunkData> = Vec::with_capacity(selected_coords.len());
    for (cx, cz) in selected_coords {
        match reader.read_chunk(cx, cz) {
            Ok(Some(chunk)) => chunks.push(chunk),
            Ok(None) => {}
            Err(error) => {
                return Err(DecodeFailure::Format(format!(
                    "Failed to read chunk ({cx}, {cz}) from MCA: {error}"
                )));
            }
        }
    }

    if chunks.is_empty() {
        return Err(DecodeFailure::Format(
            "Failed to decode any populated chunks from this MCA file.".to_string(),
        ));
    }

    // Determine region bounds directly from chunk and non-air section coordinates in O(1).
    let min_chunk_x = chunks.iter().map(|c| c.x).min().unwrap();
    let max_chunk_x = chunks.iter().map(|c| c.x).max().unwrap();
    let min_chunk_z = chunks.iter().map(|c| c.z).min().unwrap();
    let max_chunk_z = chunks.iter().map(|c| c.z).max().unwrap();

    let mut min_section_y = i8::MAX;
    let mut max_section_y = i8::MIN;
    let mut has_non_air_sections = false;

    for chunk in &chunks {
        for section in &chunk.sections {
            if !section.palette.iter().all(|b| is_air(&b.name)) {
                min_section_y = min_section_y.min(section.y);
                max_section_y = max_section_y.max(section.y);
                has_non_air_sections = true;
            }
        }
    }

    if !has_non_air_sections {
        return Err(DecodeFailure::Format(
            "The selected chunks in this MCA file contain no blocks.".to_string(),
        ));
    }

    let min_x = min_chunk_x * 16;
    let min_z = min_chunk_z * 16;
    let min_y = (min_section_y as i32) * 16;

    let width = (max_chunk_x - min_chunk_x + 1) * 16;
    let length = (max_chunk_z - min_chunk_z + 1) * 16;
    let height = ((max_section_y - min_section_y + 1) as i32) * 16;

    let mut schematic = UniversalSchematic::new("mca".to_string());
    let data_version = chunks.iter().map(|c| c.data_version).max();
    schematic.metadata.name = Some("mca".to_string());
    schematic.metadata.mc_version = data_version;
    schematic.metadata.source_data_version = data_version;

    schematic.default_region = Region::try_new(
        schematic.default_region_name.clone(),
        (0, 0, 0),
        (width, height, length),
    )
    .map_err(|error| DecodeFailure::Format(error.to_string()))?;

    // Populate the region with blocks in a single pass.
    for chunk in &chunks {
        let chunk_base_x = (chunk.x - min_chunk_x) * 16;
        let chunk_base_z = (chunk.z - min_chunk_z) * 16;

        for section in &chunk.sections {
            if section.palette.iter().all(|b| is_air(&b.name)) {
                continue;
            }
            let section_base_y = (section.y as i32 - min_section_y as i32) * 16;

            for local_y in 0..16 {
                for local_z in 0..16 {
                    for local_x in 0..16 {
                        let idx = (local_y * 256 + local_z * 16 + local_x) as usize;
                        if idx >= section.block_states.len() {
                            continue;
                        }
                        let palette_idx = section.block_states[idx] as usize;
                        if palette_idx >= section.palette.len() {
                            continue;
                        }
                        let block = &section.palette[palette_idx];
                        if is_air(&block.name) {
                            continue;
                        }

                        let rx = chunk_base_x + local_x;
                        let ry = section_base_y + local_y;
                        let rz = chunk_base_z + local_z;
                        schematic.default_region.set_block(rx, ry, rz, block);
                    }
                }
            }
        }

        for be in &chunk.block_entities {
            let (bx, by, bz) = be.position;
            let rx = bx - min_x;
            let ry = by - min_y;
            let rz = bz - min_z;
            if rx >= 0 && rx < width && ry >= 0 && ry < height && rz >= 0 && rz < length {
                let mut be_clone = be.clone();
                be_clone.position = (rx, ry, rz);
                schematic.default_region.set_block_entity(
                    BlockPosition {
                        x: rx,
                        y: ry,
                        z: rz,
                    },
                    be_clone,
                );
            }
        }

        for entity in &chunk.entities {
            let rx = entity.position.0 - min_x as f64;
            let ry = entity.position.1 - min_y as f64;
            let rz = entity.position.2 - min_z as f64;
            if rx >= 0.0
                && rx < width as f64
                && ry >= 0.0
                && ry < height as f64
                && rz >= 0.0
                && rz < length as f64
            {
                let mut entity_clone = entity.clone();
                entity_clone.position = (rx, ry, rz);
                schematic.default_region.add_entity(entity_clone);
            }
        }
    }

    if schematic.total_blocks() == 0 {
        return Err(DecodeFailure::Format(
            "The selected chunks in this MCA file contain no blocks.".to_string(),
        ));
    }

    Ok(schematic)
}

/// Select up to 4 chunks from the populated chunk positions.
///
/// Prefers a 2x2 chunk window with the highest number of populated chunks,
/// breaking ties towards the centroid of all populated chunks. If the best
/// 2x2 window contains fewer than 4 chunks, remaining slots are filled from
/// the nearest populated chunks.
fn select_preview_chunks(
    populated: &[(i32, i32)],
    region_x: i32,
    region_z: i32,
) -> Vec<(i32, i32)> {
    if populated.len() <= 4 {
        let mut sorted = populated.to_vec();
        sorted.sort_by_key(|&(cx, cz)| (cz, cx));
        return sorted;
    }

    let populated_set: HashSet<(i32, i32)> = populated.iter().cloned().collect();

    let centroid_x = populated.iter().map(|&(x, _)| x as f64).sum::<f64>() / (populated.len() as f64);
    let centroid_z = populated.iter().map(|&(_, z)| z as f64).sum::<f64>() / (populated.len() as f64);

    let base_cx = region_x * 32;
    let base_cz = region_z * 32;

    let mut best_window: Option<(i32, i32)> = None;
    let mut best_count = 0;
    let mut best_dist_sq = f64::MAX;

    for lz in 0..31 {
        for lx in 0..31 {
            let cx = base_cx + lx;
            let cz = base_cz + lz;

            let count = [
                (cx, cz),
                (cx + 1, cz),
                (cx, cz + 1),
                (cx + 1, cz + 1),
            ]
            .iter()
            .filter(|pos| populated_set.contains(pos))
            .count();

            if count == 0 {
                continue;
            }

            let center_x = cx as f64 + 0.5;
            let center_z = cz as f64 + 0.5;
            let dx = center_x - centroid_x;
            let dz = center_z - centroid_z;
            let dist_sq = dx * dx + dz * dz;

            if count > best_count || (count == best_count && dist_sq < best_dist_sq) {
                best_count = count;
                best_dist_sq = dist_sq;
                best_window = Some((cx, cz));
            }
        }
    }

    let mut selected = Vec::new();
    let window_center = if let Some((wx, wz)) = best_window {
        for pos in [
            (wx, wz),
            (wx + 1, wz),
            (wx, wz + 1),
            (wx + 1, wz + 1),
        ] {
            if populated_set.contains(&pos) {
                selected.push(pos);
            }
        }
        (wx as f64 + 0.5, wz as f64 + 0.5)
    } else {
        (centroid_x, centroid_z)
    };

    if selected.len() < 4 {
        let selected_set: HashSet<(i32, i32)> = selected.iter().cloned().collect();
        let mut remaining: Vec<(i32, i32)> = populated
            .iter()
            .cloned()
            .filter(|pos| !selected_set.contains(pos))
            .collect();

        remaining.sort_by(|&(ax, az), &(bx, bz)| {
            let da = (ax as f64 - window_center.0).powi(2) + (az as f64 - window_center.1).powi(2);
            let db = (bx as f64 - window_center.0).powi(2) + (bz as f64 - window_center.1).powi(2);
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        });

        for pos in remaining {
            if selected.len() >= 4 {
                break;
            }
            selected.push(pos);
        }
    }

    selected.sort_by_key(|&(cx, cz)| (cz, cx));
    selected
}
