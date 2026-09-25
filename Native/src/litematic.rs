//! Direct compact Litematic import.
//!
//! Nucleation 0.10.14 format handling; see NOTICE and
//! ThirdParty/Nucleation-LICENSE.txt.
use nucleation::formats::limits::DecodeLimits;
use quartz_nbt::{NbtCompound, NbtTag};

use crate::meshing::CompactBlocks;

#[path = "litematic_stream.rs"]
mod stream;

pub(super) fn read_compact(
    data: &[u8],
    limits: &DecodeLimits,
    chunk_size: Option<i32>,
    thread_count: Option<u8>,
    speed_first: bool,
    current: &impl Fn() -> Result<(), String>,
) -> Result<Option<CompactBlocks>, String> {
    stream::read(data, limits, chunk_size, thread_count, speed_first, current)
}

fn triple(nbt: &NbtCompound, key: &str) -> Result<(i32, i32, i32), String> {
    let value = nbt
        .get::<_, &NbtCompound>(key)
        .map_err(|error| error.to_string())?;
    Ok((
        value
            .get::<_, i32>("x")
            .map_err(|error| error.to_string())?,
        value
            .get::<_, i32>("y")
            .map_err(|error| error.to_string())?,
        value
            .get::<_, i32>("z")
            .map_err(|error| error.to_string())?,
    ))
}

fn block_entity_position(nbt: &NbtCompound) -> Result<(i32, i32, i32), String> {
    if let Ok(position) = nbt.get::<_, &[i32]>("Pos") {
        if position.len() < 3 {
            return Err("truncated block entity position".into());
        }
        return Ok((position[0], position[1], position[2]));
    }
    let integer = |key| match nbt.inner().get(key) {
        Some(NbtTag::Byte(value)) => Some(i32::from(*value)),
        Some(NbtTag::Short(value)) => Some(i32::from(*value)),
        Some(NbtTag::Int(value)) => Some(*value),
        _ => None,
    };
    Ok(match (integer("x"), integer("y"), integer("z")) {
        (Some(x), Some(y), Some(z)) => (x, y, z),
        _ => (0, 0, 0),
    })
}

fn offset_position(
    position: (i32, i32, i32),
    offset: (i32, i32, i32),
) -> Result<(i32, i32, i32), String> {
    Ok((
        position
            .0
            .checked_add(offset.0)
            .ok_or("block entity X overflow")?,
        position
            .1
            .checked_add(offset.1)
            .ok_or("block entity Y overflow")?,
        position
            .2
            .checked_add(offset.2)
            .ok_or("block entity Z overflow")?,
    ))
}
