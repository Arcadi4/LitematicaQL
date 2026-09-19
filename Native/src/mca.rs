// Minecraft Anvil `.mca` region decoding.
//
// An MCA region file stores up to 1024 chunks in a 32x32 grid. A full region
// can easily span 100+ million blocks, which is far beyond the preview limit.
// This loader scans the region location table, selects a cohesive 4-chunk
// parcel (preferring a 2x2 chunk window centered near the populated chunks),
// reads only those chunks via `RegionReader`, and normalizes their coordinates
// into a single previewable schematic.

use std::collections::HashSet;
use std::io::{Cursor, Write};

use flate2::write::ZlibEncoder;
use flate2::Compression;
use lz4_flex::block;
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

// Vanilla writes region chunks with `region-file-compression=lz4` (since
// 24w04a) as lz4-java block streams, and Nucleation's region reader refuses
// that compression byte. Decoding them here means one new dependency and no
// duplicated chunk NBT parsing: the region is rebuilt byte-for-byte with the
// LZ4 records re-emitted as zlib before the reader ever sees it.

/// Compression byte for `region-file-compression=lz4` regions (since 24w04a).
const COMPRESSION_LZ4: u8 = 4;

/// A populated location-table entry whose record header lies within `data`.
struct LocatedChunk {
    index: usize,
    /// Byte offset of the record: a 4-byte big-endian length, then the
    /// compression byte, then `length - 1` payload bytes.
    byte_offset: usize,
    /// The declared record length, compression byte included.
    record_len: usize,
    /// The record's compression byte.
    compression: u8,
}

/// Scans the location table for populated entries with a readable record
/// header. Entries that point past the end of the file are skipped; chunks
/// lost that way surface later as skipped-chunk warnings.
fn located_chunks(data: &[u8]) -> Vec<LocatedChunk> {
    const SECTOR_BYTES: usize = 4096;

    let mut located = Vec::new();
    for index in 0..1024 {
        let entry = index * 4;
        let sector_offset = ((data[entry] as usize) << 16)
            | ((data[entry + 1] as usize) << 8)
            | data[entry + 2] as usize;
        let sector_count = data[entry + 3] as usize;
        if sector_offset < 2 || sector_count == 0 {
            continue;
        }
        let Some(byte_offset) = sector_offset.checked_mul(SECTOR_BYTES) else {
            continue;
        };
        if byte_offset + 5 > data.len() {
            continue;
        }
        let record_len = u32::from_be_bytes([
            data[byte_offset],
            data[byte_offset + 1],
            data[byte_offset + 2],
            data[byte_offset + 3],
        ]) as usize;
        located.push(LocatedChunk {
            index,
            byte_offset,
            record_len,
            compression: data[byte_offset + 4],
        });
    }
    located
}

/// Decompresses the lz4-java block stream vanilla writes for LZ4 regions
/// (`LZ4BlockInputStream`).
///
/// The stream is a sequence of blocks, each a 21-byte header — the magic
/// `LZ4Block`, a token whose high nibble is the method (0x10 stored raw,
/// 0x20 LZ4), little-endian compressed and original lengths, and an XXHash32
/// checksum of the original bytes — followed by the compressed data. The
/// writer terminates with a zero-length raw block; a truncated stream is an
/// error. The checksum is not verified because decoding does not depend on
/// it, and block sizes are capped at lz4-java's own maximum so a corrupt
/// header cannot demand an absurd allocation.
fn lz4_java_block_stream_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    const MAGIC: &[u8; 8] = b"LZ4Block";
    const HEADER_LEN: usize = 21;
    const METHOD_RAW: u8 = 0x10;
    const METHOD_LZ4: u8 = 0x20;
    // lz4-java refuses block sizes past `1 << (10 + 15)`; mirror the cap so
    // corrupt headers cannot resize the output into the gigabytes.
    const MAX_BLOCK_BYTES: usize = 1 << 25;

    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let header = data
            .get(pos..pos + HEADER_LEN)
            .ok_or("truncated LZ4 block header")?;
        if &header[..8] != MAGIC {
            return Err("this is not an LZ4 block stream".to_string());
        }
        let method = header[8] & 0xf0;
        let compressed_len = u32::from_le_bytes(header[9..13].try_into().unwrap()) as usize;
        let original_len = u32::from_le_bytes(header[13..17].try_into().unwrap()) as usize;
        let payload = data
            .get(pos + HEADER_LEN..pos + HEADER_LEN + compressed_len)
            .ok_or("truncated LZ4 block payload")?;

        if original_len == 0 {
            break;
        }
        if original_len > MAX_BLOCK_BYTES {
            return Err("LZ4 block exceeds lz4-java's maximum block size".to_string());
        }
        match method {
            METHOD_RAW => {
                if payload.len() != original_len {
                    return Err("stored LZ4 block length mismatch".to_string());
                }
                out.extend_from_slice(payload);
            }
            METHOD_LZ4 => {
                let start = out.len();
                out.resize(start + original_len, 0);
                block::decompress_into(payload, &mut out[start..])
                    .map_err(|error| format!("LZ4 block failed to decompress: {error}"))?;
            }
            other => return Err(format!("unknown LZ4 block method 0x{other:02x}")),
        }
        pos += HEADER_LEN + compressed_len;
    }
    Ok(out)
}

/// Rebuilds a region so every LZ4 chunk record arrives as zlib.
///
/// Records with compression types 1-3 are copied verbatim, type-4 records are
/// decompressed from the lz4-java block stream and re-emitted as zlib, and
/// anything else — unreadable records, unknown compression bytes, results
/// that no longer fit the 255-sector cap — is dropped from the table.
/// `load_mca_preview` reports dropped chunks as skipped. `None` means no
/// populated record used LZ4 and the caller should decode the original bytes.
fn transcode_lz4_chunks(data: &[u8]) -> Option<Vec<u8>> {
    const SECTOR_BYTES: usize = 4096;
    // The location entry stores the sector count in one byte.
    const MAX_RECORD_BYTES: usize = 255 * SECTOR_BYTES - 4;

    let located = located_chunks(data);
    if !located
        .iter()
        .any(|chunk| chunk.compression == COMPRESSION_LZ4)
    {
        return None;
    }

    // Two header sectors, then each surviving chunk at a fresh sector-aligned
    // offset with zeroed timestamps: a spec-shaped file any region reader
    // could open.
    let mut out = vec![0u8; 2 * SECTOR_BYTES];
    let mut next_sector: u32 = 2;

    for chunk in located {
        let record: Vec<u8> = match chunk.compression {
            1..=3 => {
                let end = chunk.byte_offset + 4 + chunk.record_len;
                if end > data.len() || 4 + chunk.record_len > MAX_RECORD_BYTES {
                    continue;
                }
                data[chunk.byte_offset..end].to_vec()
            }
            COMPRESSION_LZ4 => {
                if chunk.record_len < 2 {
                    continue;
                }
                let payload_end = chunk.byte_offset + 4 + chunk.record_len;
                if payload_end > data.len() {
                    continue;
                }
                let payload = &data[chunk.byte_offset + 5..payload_end];
                let Ok(decompressed) = lz4_java_block_stream_decompress(payload) else {
                    continue;
                };
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
                if encoder.write_all(&decompressed).is_err() {
                    continue;
                }
                let Ok(compressed) = encoder.finish() else {
                    continue;
                };
                if 5 + compressed.len() > MAX_RECORD_BYTES {
                    continue;
                }
                let mut record = Vec::with_capacity(5 + compressed.len());
                record.extend_from_slice(&(compressed.len() as u32 + 1).to_be_bytes());
                record.push(2);
                record.extend_from_slice(&compressed);
                record
            }
            _ => continue,
        };

        let sector_count = (record.len() as u32).div_ceil(SECTOR_BYTES as u32);
        let entry = chunk.index * 4;
        out[entry..entry + 3].copy_from_slice(&next_sector.to_be_bytes()[1..4]);
        out[entry + 3] = sector_count as u8;
        let start = out.len();
        out.extend_from_slice(&record);
        out.resize(start + sector_count as usize * SECTOR_BYTES, 0);
        next_sector += sector_count;
    }

    Some(out)
}

/// The compression byte recorded for one location-table index, when its
/// record header is readable.
fn chunk_record_compression(data: &[u8], index: usize) -> Option<u8> {
    const SECTOR_BYTES: usize = 4096;

    let entry = data.get(index * 4..index * 4 + 4)?;
    let sector_offset = ((entry[0] as usize) << 16)
        | ((entry[1] as usize) << 8)
        | entry[2] as usize;
    if sector_offset < 2 {
        return None;
    }
    let byte_offset = sector_offset.checked_mul(SECTOR_BYTES)?;
    data.get(byte_offset + 4).copied()
}

/// Why a selected chunk produced no data. Distinguishes the spec's external
/// `.mcc` spelling (length 1, compression value +128) from other empty
/// records so the preview can say which chunks live outside the file.
fn missing_chunk_notice(bytes: &[u8], region: (i32, i32), cx: i32, cz: i32) -> String {
    let local_x = cx - region.0 * 32;
    let local_z = cz - region.1 * 32;
    let external = (0..32).contains(&local_x)
        && (0..32).contains(&local_z)
        && chunk_record_compression(bytes, (local_z * 32 + local_x) as usize)
            .is_some_and(|compression| compression >= 128);
    if external {
        format!("Chunk ({cx}, {cz}) is stored in the external c.{cx}.{cz}.mcc file and is not shown.")
    } else {
        format!("Chunk ({cx}, {cz}) has an empty record and is not shown.")
    }
}

pub(super) fn load_mca_preview(
    bytes: &[u8],
) -> Result<(UniversalSchematic, Vec<String>), DecodeFailure> {
    // Regions written with `region-file-compression=lz4` carry chunk records
    // Nucleation cannot decompress; rebuild those records as zlib first. The
    // original bytes stay authoritative for header questions such as whether
    // a chunk is stored externally.
    let transcoded = transcode_lz4_chunks(bytes);
    let region_bytes: &[u8] = transcoded.as_deref().unwrap_or(bytes);

    let mut warnings = Vec::new();
    let mut reader = RegionReader::new_auto(Cursor::new(region_bytes)).map_err(|error| {
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
            Ok(None) => warnings.push(missing_chunk_notice(bytes, (region_x, region_z), cx, cz)),
            Err(error) => warnings.push(format!(
                "Chunk ({cx}, {cz}) could not be read and is not shown ({}).",
                error.to_string().replace('\n', " ")
            )),
        }
    }

    if chunks.is_empty() {
        let mut message = "Failed to decode any populated chunks from this MCA file.".to_string();
        if !warnings.is_empty() {
            message.push(' ');
            message.push_str(&warnings.join(" "));
        }
        return Err(DecodeFailure::Format(message));
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

    Ok((schematic, warnings))
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
