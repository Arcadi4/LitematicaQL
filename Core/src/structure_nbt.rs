// Vanilla Java structure `.nbt` decoding.
//
// Nucleation's format manager recognizes binary structures by content but
// parses only their SNBT spelling. This module decodes the binary container
// into the same schematic model.

use std::io::{Cursor, Read};

use flate2::read::GzDecoder;
use nucleation::block_entity::BlockEntity;
use nucleation::block_position::BlockPosition;
use nucleation::nbt::NbtMap;
use nucleation::{BlockState, Entity, Region, UniversalSchematic};
use quartz_nbt::io::Flavor;
use quartz_nbt::{NbtCompound, NbtList, NbtTag};

use super::{DecodeFailure, MAX_DECOMPRESSED_BYTES, MAX_VOLUME};

// Return whether `bytes` contain binary structure NBT. Requiring both `blocks`
// and `palette` distinguishes structures from other NBT roots such as levels,
// chunks, and saved items.
pub(super) fn is_binary_structure(bytes: &[u8]) -> bool {
    let raw = match maybe_gunzip(bytes) {
        Ok(raw) => raw,
        Err(_) => return false,
    };
    let Ok((root, _)) = quartz_nbt::io::read_nbt(&mut Cursor::new(&raw), Flavor::Uncompressed)
    else {
        return false;
    };
    root.contains_key("blocks") && root.contains_key("palette")
}

pub(super) fn load_structure_nbt(bytes: &[u8]) -> Result<UniversalSchematic, DecodeFailure> {
    let raw = maybe_gunzip(bytes).map_err(DecodeFailure::Format)?;
    let (root, _) = quartz_nbt::io::read_nbt(&mut Cursor::new(&raw), Flavor::Uncompressed)
        .map_err(|error| {
            DecodeFailure::Format(format!("This file is not readable NBT: {error}"))
        })?;

    let unreadable =
        || DecodeFailure::Format("This file is not a Java structure block container.".to_string());

    let size = triple(&root, "size").ok_or_else(unreadable)?;
    if size.iter().any(|axis| *axis <= 0) {
        return Err(unreadable());
    }
    let volume = i64::from(size[0]) * i64::from(size[1]) * i64::from(size[2]);
    if volume > i64::try_from(MAX_VOLUME).expect("volume fits") {
        return Err(DecodeFailure::Limit(format!(
            "This schematic is {size:?} ({volume} cells), beyond the {MAX_VOLUME}-cell preview limit."
        )));
    }

    let data_version = match root.inner().get("DataVersion") {
        Some(NbtTag::Int(value)) => Some(*value),
        _ => None,
    };

    let mut schematic = UniversalSchematic::new("structure".to_string());
    schematic.metadata.name = Some("structure".to_string());
    schematic.metadata.mc_version = data_version;
    schematic.metadata.source_data_version = data_version;
    schematic.default_region = Region::try_new(
        schematic.default_region_name.clone(),
        (0, 0, 0),
        (size[0], size[1], size[2]),
    )
    .map_err(|error| DecodeFailure::Format(format!("{error}")))?;

    let palette = read_palette(&root).ok_or_else(unreadable)?;

    let Some(NbtTag::List(blocks)) = root.inner().get("blocks") else {
        return Err(unreadable());
    };
    if blocks.len() as i64 > volume {
        return Err(unreadable());
    }

    for entry in blocks.iter() {
        let NbtTag::Compound(entry) = entry else {
            return Err(unreadable());
        };
        let Some(position) = triple(entry, "pos") else {
            return Err(unreadable());
        };
        // `Region::try_new` validates only the declared dimensions; each block
        // position still needs an explicit bounds check.
        if position
            .iter()
            .enumerate()
            .any(|(axis, value)| *value < 0 || *value >= size[axis])
        {
            return Err(DecodeFailure::Format(format!(
                "This structure has a block at {position:?}, outside its {size:?} size."
            )));
        }
        let Some(NbtTag::Int(index)) = entry.inner().get("state") else {
            return Err(unreadable());
        };
        let Some(state) = palette.get(*index as usize) else {
            return Err(unreadable());
        };

        let block = BlockState::from_block_string(state)
            .map_err(|error| DecodeFailure::Format(format!("{error}")))?;
        let (x, y, z) = (position[0], position[1], position[2]);
        schematic.set_block(x, y, z, &block);

        if let Some(NbtTag::Compound(nbt)) = entry.inner().get("nbt") {
            let map = NbtMap::from_quartz_nbt(nbt);
            let id = nbt
                .inner()
                .get("id")
                .or_else(|| nbt.inner().get("Id"))
                .and_then(|tag| match tag {
                    NbtTag::String(value) => Some(value.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| block.get_name().to_string());
            let mut block_entity = BlockEntity::new(id, (x, y, z));
            block_entity.set_nbt(map);
            schematic.set_block_entity(BlockPosition { x, y, z }, block_entity);
        }
    }

    if let Some(NbtTag::List(list)) = root.inner().get("entities") {
        for entry in list.iter() {
            let NbtTag::Compound(entry) = entry else {
                continue;
            };
            let Some(NbtTag::Compound(nbt)) = entry.inner().get("nbt") else {
                continue;
            };
            // Match vanilla's deliberate tolerance for entities without a type
            // identifier; one malformed entity must not reject the structure.
            if !nbt.contains_key("id") && !nbt.contains_key("Id") {
                continue;
            }
            let mut nbt = nbt.clone();
            if let Some(position) = double_triple(entry, "pos") {
                let coordinates = position.map(NbtTag::Double);
                nbt.insert("Pos", NbtList::clone_from(&coordinates));
            }
            if let Ok(entity) = Entity::from_nbt(&nbt) {
                schematic.add_entity(entity);
            }
        }
    }

    Ok(schematic)
}

// Return gzip-decoded bytes, or pass through non-gzip input unchanged. Reject
// expanded structures above the shared decompression budget.
fn maybe_gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Ok(bytes.to_vec());
    }
    let mut out = Vec::new();
    GzDecoder::new(Cursor::new(bytes))
        .read_to_end(&mut out)
        .map_err(|error| format!("gzip decompression failed: {error}"))?;
    if out.len() > MAX_DECOMPRESSED_BYTES {
        return Err(format!(
            "the structure expands beyond the {} MiB preview limit.",
            MAX_DECOMPRESSED_BYTES / 1_048_576
        ));
    }
    Ok(out)
}

// Accept an int array or a three-element byte, short, or int list.
fn triple(compound: &NbtCompound, key: &str) -> Option<[i32; 3]> {
    let tag = compound.inner().get(key)?;
    let values: Vec<i32> = match tag {
        NbtTag::IntArray(values) => values.clone(),
        NbtTag::List(list) => {
            let mut out = Vec::with_capacity(list.len());
            for entry in list.iter() {
                out.push(match entry {
                    NbtTag::Byte(v) => i32::from(*v),
                    NbtTag::Short(v) => i32::from(*v),
                    NbtTag::Int(v) => *v,
                    _ => return None,
                });
            }
            out
        }
        _ => return None,
    };
    let [x, y, z] = values.as_slice() else {
        return None;
    };
    Some([*x, *y, *z])
}

// Accept a three-element float or double list.
fn double_triple(compound: &NbtCompound, key: &str) -> Option<[f64; 3]> {
    let NbtTag::List(list) = compound.inner().get(key)? else {
        return None;
    };
    let mut out = Vec::with_capacity(list.len());
    for entry in list.iter() {
        out.push(match entry {
            NbtTag::Float(v) => f64::from(*v),
            NbtTag::Double(v) => *v,
            _ => return None,
        });
    }
    let [x, y, z] = out.as_slice() else {
        return None;
    };
    Some([*x, *y, *z])
}

// Convert palette entries to canonical `id[property=value,…]` block-state
// strings.
fn read_palette(root: &NbtCompound) -> Option<Vec<String>> {
    let NbtTag::List(palette) = root.inner().get("palette")? else {
        return None;
    };
    let mut states = Vec::with_capacity(palette.len());
    for entry in palette.iter() {
        let NbtTag::Compound(entry) = entry else {
            return None;
        };
        let name = match entry.inner().get("Name") {
            Some(NbtTag::String(name)) => name.clone(),
            _ => return None,
        };
        let mut properties: Vec<(String, String)> = Vec::new();
        if let Some(NbtTag::Compound(properties_tag)) = entry.inner().get("Properties") {
            for (key, value) in properties_tag.inner().iter() {
                let NbtTag::String(value) = value else {
                    return None;
                };
                properties.push((key.clone(), value.clone()));
            }
        }
        // Canonical property order keeps serialization-order differences from
        // changing the decoded block state.
        properties.sort();
        states.push(if properties.is_empty() {
            name
        } else {
            let body = properties
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join(",");
            format!("{name}[{body}]")
        });
    }
    Some(states)
}
