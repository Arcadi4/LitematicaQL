// Preview decoding for Minecraft Anvil `.mca` region files.
//
// A region can span far beyond preview limits, so this loader reads at most a
// cohesive four-chunk parcel, preferring a populated 2x2 window near the
// populated-chunk centroid, then normalizes chunk coordinates into one
// schematic.

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


/// Compression byte for `region-file-compression=lz4` regions (since 24w04a).
const COMPRESSION_LZ4: u8 = 4;

/// A populated location-table entry whose record header is readable.
struct LocatedChunk {
    index: usize,
    byte_offset: usize,
    record_len: usize,
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

/// Decode vanilla's lz4-java `LZ4BlockInputStream` framing.
///
/// Each 21-byte header contains the `LZ4Block` magic, a method token, little-
/// endian compressed and uncompressed lengths, and an XXHash32 checksum. A
/// zero-length block ends the stream. The checksum is not required for
/// decoding. `MAX_BLOCK_BYTES` mirrors lz4-java's ceiling so corrupt headers
/// cannot force an oversized allocation for a single block.
fn lz4_java_block_stream_decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    const MAGIC: &[u8; 8] = b"LZ4Block";
    const HEADER_LEN: usize = 21;
    const METHOD_RAW: u8 = 0x10;
    const METHOD_LZ4: u8 = 0x20;
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

/// Rebuild a region with every LZ4 chunk record re-emitted as zlib.
///
/// Compression types 1-3 are copied verbatim. Records that cannot be read or
/// re-emitted within the 255-sector limit are omitted from the table and later
/// surface as skipped-chunk warnings. `None` means no populated record used
/// LZ4, so the caller should decode `data` directly.
fn transcode_lz4_chunks(data: &[u8]) -> Option<Vec<u8>> {
    const SECTOR_BYTES: usize = 4096;
    // The location table's one-byte sector count caps records below 255 sectors.
    const MAX_RECORD_BYTES: usize = 255 * SECTOR_BYTES - 4;

    let located = located_chunks(data);
    if !located
        .iter()
        .any(|chunk| chunk.compression == COMPRESSION_LZ4)
    {
        return None;
    }

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

/// Select at most four populated chunks, preferring a 2x2 window with the most
/// entries. Ties favor the window nearest the populated-chunk centroid; if the
/// best window is incomplete, nearest remaining chunks fill the vacancies.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Independent vanilla-compatible framing for exercising the decoder.
    fn frame_lz4_java_stream(payload: &[u8], block_size: usize, method: u8) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in payload.chunks(block_size.max(1)) {
            out.extend_from_slice(b"LZ4Block");
            let (token, data): (u8, Vec<u8>) = match method {
                0x10 => (0x10, chunk.to_vec()),
                _ => (0x20, block::compress(chunk)),
            };
            out.push(token);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&data);
        }
        out.extend_from_slice(b"LZ4Block");
        out.extend_from_slice(&[0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        out
    }

    /// Deterministic incompressible data exercises real LZ4 encoding across
    /// multiple blocks.
    fn pseudo_random(len: usize) -> Vec<u8> {
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    #[test]
    fn lz4_stream_round_trips_multi_block() {
        let payload = pseudo_random(300_000);
        let framed = frame_lz4_java_stream(&payload, 1 << 16, 0x20);
        assert_eq!(
            lz4_java_block_stream_decompress(&framed).expect("stream should decompress"),
            payload
        );
    }

    #[test]
    fn lz4_stream_accepts_stored_blocks() {
        let payload = b"stored block payloads pass through verbatim".to_vec();
        let framed = frame_lz4_java_stream(&payload, 8, 0x10);
        assert_eq!(
            lz4_java_block_stream_decompress(&framed).expect("stream should decompress"),
            payload
        );
    }

    #[test]
    fn lz4_stream_tolerates_missing_endmark() {
        let payload = pseudo_random(50_000);
        let framed = frame_lz4_java_stream(&payload, 1 << 12, 0x20);
        let truncated_at_endmark = &framed[..framed.len() - 21];
        assert_eq!(
            lz4_java_block_stream_decompress(truncated_at_endmark).expect("stream should decompress"),
            payload
        );
    }

    #[test]
    fn lz4_stream_rejects_bad_magic_and_truncation() {
        assert!(lz4_java_block_stream_decompress(b"NOTBLOCK").is_err());
        assert!(lz4_java_block_stream_decompress(b"LZ4Block\x20").is_err());
        let payload = pseudo_random(10_000);
        let framed = frame_lz4_java_stream(&payload, 1 << 12, 0x20);
        let truncated_payload = &framed[..framed.len() - 100];
        assert!(lz4_java_block_stream_decompress(truncated_payload).is_err());
    }

    #[test]
    fn regions_without_lz4_are_not_transcoded() {
        let region = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../Fixtures/Formats/Region.mca"),
        )
        .expect("read Region.mca");
        assert!(transcode_lz4_chunks(&region).is_none());
    }
}
