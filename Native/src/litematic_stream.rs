//! Two gzip passes: retain only preview metadata, then decode packed spans in
//! file order. Neither pass owns the expanded document or a BlockStates array.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Cursor, Read};

use flate2::read::GzDecoder;
use nucleation::formats::limits::DecodeLimits;
use nucleation::{BlockState, BoundingBox, Entity};
use quartz_nbt::{io::Flavor, NbtCompound, NbtList, NbtTag};
use schematic_mesher::BlockPosition;

use crate::meshing::{CompactBlocks, CompactBlocksBuilder, PaletteEntry};

const BUFFER_SIZE: usize = 64 * 1024;
/// Checkpoint packed input every 4 MiB. The 64 KiB reader buffer avoids an IPC
/// acknowledgement for every fill.
const CHECKPOINT_BYTES: usize = 4 * 1024 * 1024;

type Position = (i32, i32, i32);
type Tiles = Result<HashSet<Position>, String>;

#[derive(Clone, Copy)]
struct PackedSpan {
    offset: usize,
    longs: usize,
}

struct RegionFields {
    coordinates: NbtCompound,
    palette: Option<Result<Vec<BlockState>, String>>,
    packed: Option<PackedSpan>,
    entities: Vec<Entity>,
    entity_count: usize,
    tiles: Tiles,
    tile_count: usize,
}

impl Default for RegionFields {
    fn default() -> Self {
        Self {
            coordinates: NbtCompound::new(),
            palette: None,
            packed: None,
            entities: Vec::new(),
            entity_count: 0,
            tiles: Ok(HashSet::new()),
            tile_count: 0,
        }
    }
}
struct Regions {
    // The first serialized name is the default even for a non-compound value;
    // repeated names replace values without changing this order.
    entries: Vec<(String, Option<RegionFields>)>,
}

struct PreparedRegion {
    min: Position,
    width: usize,
    length: usize,
    plane: usize,
    volume: usize,
    bits: u32,
    packed: PackedSpan,
    palette: Vec<BlockState>,
    mapping: Vec<PaletteEntry>,
}

struct Input<'a, F> {
    reader: BufReader<GzDecoder<&'a [u8]>>,
    limits: &'a DecodeLimits,
    current: &'a F,
    offset: usize,
    next_check: usize,
    cancelled: bool,
}

impl<'a, F: Fn() -> Result<(), String>> Input<'a, F> {
    fn new(data: &'a [u8], limits: &'a DecodeLimits, current: &'a F) -> Self {
        Self {
            reader: BufReader::with_capacity(BUFFER_SIZE, GzDecoder::new(data)),
            limits,
            current,
            offset: 0,
            next_check: 0,
            cancelled: false,
        }
    }

    fn poll(&mut self) -> Result<(), String> {
        if self.offset >= self.next_check {
            if let Err(error) = (self.current)() {
                self.cancelled = true;
                return Err(error);
            }
            self.next_check = self.offset.saturating_add(CHECKPOINT_BYTES);
        }
        Ok(())
    }

    fn advance(&mut self, count: usize) -> Result<(), String> {
        self.offset = self
            .offset
            .checked_add(count)
            .ok_or("decompressed size overflow")?;
        if self.offset > self.limits.max_decompressed_bytes {
            return Err("decompressed byte limit exceeded".into());
        }
        Ok(())
    }

    fn bytes(&mut self, bytes: &mut [u8]) -> Result<(), String> {
        self.poll()?;
        self.reader
            .read_exact(bytes)
            .map_err(|error| error.to_string())?;
        self.advance(bytes.len())
    }

    #[inline]
    fn number<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let mut bytes = [0; N];
        self.bytes(&mut bytes)?;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.number::<1>()?[0])
    }

    fn skip(&mut self, mut count: usize) -> Result<(), String> {
        while count != 0 {
            self.poll()?;
            let available = self.reader.fill_buf().map_err(|error| error.to_string())?;
            if available.is_empty() {
                return Err("truncated NBT payload".into());
            }
            let take = count.min(available.len());
            self.reader.consume(take);
            self.advance(take)?;
            count -= take;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        // Reading to actual gzip EOF, not merely TAG_End, checks CRC/ISIZE and
        // truncation even when the document has an unused expanded suffix.
        loop {
            self.poll()?;
            let count = self
                .reader
                .fill_buf()
                .map_err(|error| error.to_string())?
                .len();
            if count == 0 {
                return Ok(());
            }
            self.reader.consume(count);
            self.advance(count)?;
        }
    }
}

#[derive(Clone, Copy)]
enum Keep {
    None,
    Triple,
    State,
    Properties,
    Entity,
    EntityData,
    Item,
    Tile,
    String,
    Int,
    SmallInt,
    Byte,
    Position,
    TilePosition,
    Rotation,
    Pose,
    Armor,
}

impl Keep {
    fn field(self, name: &str) -> Self {
        match self {
            Self::Triple if matches!(name, "x" | "y" | "z") => Self::Int,
            Self::State => match name {
                "Name" => Self::String,
                "Properties" => Self::Properties,
                _ => Self::None,
            },
            Self::Properties => Self::String,
            Self::Entity => match name {
                "id" | "Id" => Self::String,
                "Pos" => Self::Position,
                "NBT" => Self::EntityData,
                _ => Self::EntityData.field(name),
            },
            Self::EntityData => match name {
                "Rotation" => Self::Rotation,
                "Item" => Self::Item,
                "IsBaby" | "Color" => Self::Byte,
                "Age" => Self::Int,
                "RightArmPose" | "LeftArmPose" | "RightLegPose" | "LeftLegPose" | "HeadPose"
                | "BodyPose" => Self::Pose,
                "ArmorItems" => Self::Armor,
                _ => Self::None,
            },
            Self::Item if name == "id" => Self::String,
            Self::Tile => match name {
                "Pos" => Self::TilePosition,
                "x" | "y" | "z" => Self::SmallInt,
                _ => Self::None,
            },
            _ => Self::None,
        }
    }
}

struct Scan<'a, F> {
    input: Input<'a, F>,
    nodes: usize,
    recognized: bool,
    // At most one NBT string (u16 length) plus a tiny synthetic root header.
    string: Vec<u8>,
}

impl<'a, F: Fn() -> Result<(), String>> Scan<'a, F> {
    fn node(&mut self, depth: usize) -> Result<(), String> {
        self.nodes = self.nodes.checked_add(1).ok_or("NBT node count overflow")?;
        if depth > self.input.limits.max_nbt_depth || self.nodes > self.input.limits.max_nbt_nodes {
            return Err("NBT depth or node limit exceeded".into());
        }
        Ok(())
    }

    fn count(&mut self) -> Result<usize, String> {
        let count = i32::from_be_bytes(self.input.number()?);
        if count < 0 || count as usize > self.input.limits.max_nbt_collection_items {
            return Err("NBT collection limit exceeded".into());
        }
        Ok(count as usize)
    }

    fn string(&mut self, retain: bool) -> Result<Option<String>, String> {
        let length = u16::from_be_bytes(self.input.number()?) as usize;
        if length > self.input.limits.max_nbt_string_bytes {
            return Err("NBT string limit exceeded".into());
        }
        let needed = length + 4;
        self.string.clear();
        self.string
            .try_reserve(needed)
            .map_err(|error| error.to_string())?;
        self.string.resize(needed, 0);
        self.string[0] = 10;
        self.string[1..3].copy_from_slice(&(length as u16).to_be_bytes());
        self.input.bytes(&mut self.string[3..3 + length])?;
        if let Ok(text) = std::str::from_utf8(&self.string[3..3 + length]) {
            if !retain {
                return Ok(None);
            }
            let mut result = String::new();
            result
                .try_reserve_exact(text.len())
                .map_err(|error| error.to_string())?;
            result.push_str(text);
            return Ok(Some(result));
        }
        // Fall back to Quartz's Java CESU-8 root-name decoder for invalid UTF-8.
        let (_, text) =
            quartz_nbt::io::read_nbt(&mut Cursor::new(&self.string), Flavor::Uncompressed)
                .map_err(|error| error.to_string())?;
        Ok(retain.then_some(text))
    }

    fn next_field(&mut self, count: &mut usize) -> Result<Option<(u8, String)>, String> {
        let tag = self.input.byte()?;
        if tag == 0 {
            return Ok(None);
        }
        *count = count.checked_add(1).ok_or("NBT compound size overflow")?;
        if *count > self.input.limits.max_nbt_collection_items {
            return Err("NBT collection limit exceeded".into());
        }
        let name = self.string(true)?.expect("retained NBT name");
        Ok(Some((tag, name)))
    }

    fn list(&mut self) -> Result<(u8, usize), String> {
        let child = self.input.byte()?;
        let count = self.count()?;
        if child > 12 || (child == 0 && count != 0) {
            return Err("invalid NBT list tag".into());
        }
        if count > self.input.limits.max_nbt_nodes.saturating_sub(self.nodes) {
            return Err("NBT node limit exceeded".into());
        }
        Ok((child, count))
    }

    fn payload(&mut self, tag: u8, depth: usize, keep: Keep) -> Result<Option<NbtTag>, String> {
        self.node(depth)?;
        match tag {
            1 => {
                let value = self.input.byte()? as i8;
                Ok(matches!(keep, Keep::SmallInt | Keep::Byte).then_some(NbtTag::Byte(value)))
            }
            2 => {
                let value = i16::from_be_bytes(self.input.number()?);
                Ok(matches!(keep, Keep::SmallInt).then_some(NbtTag::Short(value)))
            }
            3 => {
                let value = i32::from_be_bytes(self.input.number()?);
                Ok(matches!(keep, Keep::Int | Keep::SmallInt).then_some(NbtTag::Int(value)))
            }
            4 | 6 => {
                self.input.skip(8)?;
                Ok(None)
            }
            5 => {
                self.input.skip(4)?;
                Ok(None)
            }
            7 | 11 | 12 => {
                let count = self.count()?;
                let width = match tag {
                    7 => 1,
                    11 => 4,
                    _ => 8,
                };
                let bytes = count.checked_mul(width).ok_or("NBT array size overflow")?;
                if tag == 11 && matches!(keep, Keep::TilePosition) {
                    let mut values = Vec::new();
                    values
                        .try_reserve_exact(count.min(3))
                        .map_err(|error| error.to_string())?;
                    for _ in 0..count.min(3) {
                        values.push(i32::from_be_bytes(self.input.number()?));
                    }
                    self.input.skip(bytes - values.len() * 4)?;
                    Ok(Some(NbtTag::IntArray(values)))
                } else {
                    self.input.skip(bytes)?;
                    Ok(None)
                }
            }
            8 => Ok(self
                .string(matches!(keep, Keep::String))?
                .map(NbtTag::String)),
            9 => {
                let (child, count) = self.list()?;
                let take = match keep {
                    Keep::Position if child == 6 && count == 3 => 3,
                    Keep::Rotation if child == 5 => count.min(1),
                    Keep::Pose if child == 5 => count,
                    Keep::Armor if child == 10 => count.min(4),
                    _ => 0,
                };
                let mut values = Vec::new();
                for index in 0..count {
                    let value = if index < take && matches!(child, 5 | 6) {
                        self.node(depth + 1)?;
                        Some(if child == 5 {
                            NbtTag::Float(f32::from_be_bytes(self.input.number()?))
                        } else {
                            NbtTag::Double(f64::from_be_bytes(self.input.number()?))
                        })
                    } else {
                        self.payload(
                            child,
                            depth + 1,
                            if index < take { Keep::Item } else { Keep::None },
                        )?
                    };
                    if let Some(value) = value {
                        values.try_reserve(1).map_err(|error| error.to_string())?;
                        values.push(value);
                    }
                }
                Ok((take != 0).then(|| NbtTag::List(NbtList::from(values))))
            }
            10 => {
                let retain = matches!(
                    keep,
                    Keep::Triple
                        | Keep::State
                        | Keep::Properties
                        | Keep::Entity
                        | Keep::EntityData
                        | Keep::Item
                        | Keep::Tile
                );
                let mut compound = NbtCompound::new();
                let mut count = 0;
                while let Some((child, name)) = self.next_field(&mut count)? {
                    let selection = if retain {
                        keep.field(&name)
                    } else {
                        Keep::None
                    };
                    let value = self.payload(child, depth + 1, selection)?;
                    if let Some(value) = value {
                        compound
                            .inner_mut()
                            .try_reserve(1)
                            .map_err(|error| error.to_string())?;
                        compound.insert(name, value);
                    } else if retain {
                        // Later fields replace earlier fields regardless of type.
                        compound.inner_mut().shift_remove(&name);
                    }
                }
                Ok(retain.then_some(NbtTag::Compound(compound)))
            }
            _ => Err("invalid NBT tag".into()),
        }
    }

    fn palette(&mut self, depth: usize) -> Result<Result<Vec<BlockState>, String>, String> {
        self.node(depth)?;
        let (child, count) = self.list()?;
        let mut palette = Vec::new();
        let mut invalid = if count == 0 || count > self.input.limits.max_palette_entries {
            Some("empty palette or palette limit exceeded".to_string())
        } else {
            None
        };
        for _ in 0..count {
            let state = self.payload(child, depth + 1, Keep::State)?;
            if invalid.is_none() {
                match state {
                    Some(NbtTag::Compound(state)) => match BlockState::from_nbt(&state) {
                        Ok(state) => {
                            palette.try_reserve(1).map_err(|error| error.to_string())?;
                            palette.push(state);
                        }
                        Err(error) => invalid = Some(error),
                    },
                    _ => invalid = Some("invalid palette state".into()),
                }
            }
        }
        Ok(match invalid {
            Some(error) => Err(error),
            None => Ok(palette),
        })
    }

    fn entities(
        &mut self,
        depth: usize,
        tiles: bool,
        fields: &mut RegionFields,
    ) -> Result<(), String> {
        self.node(depth)?;
        let (child, count) = self.list()?;
        if tiles {
            fields.tile_count = count;
            fields.tiles = Ok(HashSet::new());
        } else {
            fields.entity_count = count;
            fields.entities.clear();
        }
        for _ in 0..count {
            let value = self.payload(
                child,
                depth + 1,
                if tiles { Keep::Tile } else { Keep::Entity },
            )?;
            if let Some(NbtTag::Compound(entity)) = value {
                if tiles {
                    let position = super::block_entity_position(&entity);
                    match (&mut fields.tiles, position) {
                        (Ok(positions), Ok(position)) => {
                            positions
                                .try_reserve(1)
                                .map_err(|error| error.to_string())?;
                            positions.insert(position);
                        }
                        (result @ Ok(_), Err(error)) => *result = Err(error),
                        _ => {}
                    }
                } else if let Ok(entity) = Entity::from_nbt(&entity) {
                    fields
                        .entities
                        .try_reserve(1)
                        .map_err(|error| error.to_string())?;
                    fields.entities.push(entity);
                }
            }
        }
        Ok(())
    }

    fn region(&mut self, depth: usize) -> Result<RegionFields, String> {
        self.node(depth)?;
        let mut fields = RegionFields {
            tiles: Ok(HashSet::new()),
            ..RegionFields::default()
        };
        let mut count = 0;
        while let Some((tag, name)) = self.next_field(&mut count)? {
            match (name.as_str(), tag) {
                ("Position" | "Size", _) => {
                    if let Some(value) = self.payload(tag, depth + 1, Keep::Triple)? {
                        fields.coordinates.insert(name, value);
                    } else {
                        fields.coordinates.inner_mut().shift_remove(&name);
                    }
                }
                ("BlockStatePalette", 9) => fields.palette = Some(self.palette(depth + 1)?),
                ("BlockStates", 12) => {
                    self.node(depth + 1)?;
                    let longs = self.count()?;
                    fields.packed = Some(PackedSpan {
                        offset: self.input.offset,
                        longs,
                    });
                    self.input
                        .skip(longs.checked_mul(8).ok_or("NBT array size overflow")?)?;
                }
                ("Entities", 9) => self.entities(depth + 1, false, &mut fields)?,
                ("TileEntities", 9) => self.entities(depth + 1, true, &mut fields)?,
                _ => {
                    self.payload(tag, depth + 1, Keep::None)?;
                    match name.as_str() {
                        "BlockStatePalette" => fields.palette = None,
                        "BlockStates" => fields.packed = None,
                        "Entities" => {
                            fields.entities.clear();
                            fields.entity_count = 0;
                        }
                        "TileEntities" => {
                            fields.tiles = Ok(HashSet::new());
                            fields.tile_count = 0;
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(fields)
    }

    fn regions(&mut self, depth: usize) -> Result<Regions, String> {
        self.node(depth)?;
        let mut indices: HashMap<String, usize> = HashMap::new();
        let mut regions = Regions {
            entries: Vec::new(),
        };
        let mut count = 0;
        while let Some((tag, name)) = self.next_field(&mut count)? {
            let region = if tag == 10 {
                Some(self.region(depth + 1)?)
            } else {
                self.payload(tag, depth + 1, Keep::None)?;
                None
            };
            if let Some(&index) = indices.get(&name) {
                regions.entries[index].1 = region;
            } else {
                indices.try_reserve(1).map_err(|error| error.to_string())?;
                indices.insert(name.clone(), regions.entries.len());
                regions
                    .entries
                    .try_reserve(1)
                    .map_err(|error| error.to_string())?;
                regions.entries.push((name, region));
            }
        }
        Ok(regions)
    }

    fn root(&mut self) -> Result<Option<Regions>, String> {
        if self.input.byte()? != 10 {
            return Err("root NBT is not a compound".into());
        }
        self.string(false)?;
        self.node(0)?;
        let mut regions = None;
        let mut metadata = false;
        let mut count = 0;
        while let Some((tag, name)) = self.next_field(&mut count)? {
            match name.as_str() {
                "Regions" => {
                    self.recognized = true;
                    if tag == 10 {
                        regions = Some(self.regions(1)?);
                    } else {
                        regions = None;
                        self.payload(tag, 1, Keep::None)?;
                    }
                }
                "Metadata" => {
                    metadata = tag == 10;
                    self.payload(tag, 1, Keep::None)?;
                }
                _ => {
                    self.payload(tag, 1, Keep::None)?;
                }
            }
        }
        self.input.finish()?;
        if self.recognized {
            if !metadata {
                return Err("missing Litematic Metadata compound".into());
            }
            if regions.is_none() {
                return Err("missing Litematic Regions compound".into());
            }
        }
        Ok(regions)
    }
}

pub(super) fn read(
    data: &[u8],
    limits: &DecodeLimits,
    chunk_size: Option<i32>,
    thread_count: Option<u8>,
    speed_first: bool,
    current: &impl Fn() -> Result<(), String>,
) -> Result<Option<CompactBlocks>, String> {
    current()?;
    limits
        .check_input(data)
        .map_err(|error| error.to_string())?;
    if !data.starts_with(&[0x1f, 0x8b]) {
        return Ok(None);
    }
    let mut scan = Scan {
        input: Input::new(data, limits, current),
        nodes: 0,
        recognized: false,
        string: Vec::new(),
    };
    let regions = match scan.root() {
        Ok(Some(regions)) => regions,
        Ok(None) => return Ok(None),
        Err(error) if scan.recognized || scan.input.cancelled => return Err(error),
        Err(_) => return Ok(None),
    };
    drop(scan);
    current()?;
    let mut builder = CompactBlocksBuilder::new(chunk_size)?;
    let mut prepared = prepare(regions, limits, current, &mut builder)?;
    prepared.sort_unstable_by_key(|region| region.packed.offset);
    let mut input = Input::new(data, limits, current);
    for region in prepared {
        current()?;
        input.skip(
            region
                .packed
                .offset
                .checked_sub(input.offset)
                .ok_or("overlapping packed spans")?,
        )?;
        if let Some(count) = thread_count {
            visit_parallel(
                &mut input,
                &region,
                &mut builder,
                usize::from(count),
                speed_first,
                current,
            )?;
        } else {
            visit(&mut input, &region, &mut builder, current)?;
        }
    }
    input.finish()?;
    current()?;
    Ok(Some(builder.finish()))
}

fn prepare(
    mut regions: Regions,
    limits: &DecodeLimits,
    current: &impl Fn() -> Result<(), String>,
    builder: &mut CompactBlocksBuilder,
) -> Result<Vec<PreparedRegion>, String> {
    if regions.entries.len() > limits.max_regions {
        return Err("region limit exceeded".into());
    }
    let has_default = regions
        .entries
        .first()
        .is_some_and(|(_, fields)| fields.is_some());
    // Count the synthetic default only when the first serialized region is absent
    // or non-compound.
    let mut total_volume = usize::from(!has_default);
    let mut region_count = usize::from(!has_default);
    let mut total_entities = 0usize;
    let mut total_tiles = 0usize;
    if let Some((_, rest)) = regions.entries.split_first_mut() {
        rest.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
    }
    let mut prepared = Vec::new();
    for (_, fields) in regions.entries {
        current()?;
        let Some(fields) = fields else {
            continue;
        };
        region_count = region_count.checked_add(1).ok_or("region count overflow")?;
        let origin = super::triple(&fields.coordinates, "Position")?;
        let size = super::triple(&fields.coordinates, "Size")?;
        let width = i64::from(size.0).abs() as usize;
        let length = i64::from(size.2).abs() as usize;
        let volume = limits
            .check_dimensions((width as i64, i64::from(size.1).abs(), length as i64))
            .map_err(|error| error.to_string())?;
        let min = BoundingBox::try_from_position_and_size(origin, size)?.min;
        total_volume = total_volume
            .checked_add(volume)
            .ok_or("total volume overflow")?;
        if total_volume > limits.max_volume || region_count > limits.max_regions {
            return Err("total region or volume limit exceeded".into());
        }
        let palette = fields
            .palette
            .ok_or("missing Litematic BlockStatePalette")??;
        let bits = (usize::BITS - (palette.len() - 1).leading_zeros()).max(2);
        let packed = fields.packed.ok_or("missing Litematic BlockStates")?;
        let total_bits = volume
            .checked_mul(bits as usize)
            .ok_or("packed state size overflow")?;
        let longs = total_bits
            .checked_add(63)
            .ok_or("packed state size overflow")?
            / 64;
        if packed.longs != longs {
            return Err("packed state length does not match region volume".into());
        }
        total_entities = total_entities
            .checked_add(fields.entity_count)
            .ok_or("entity count overflow")?;
        total_tiles = total_tiles
            .checked_add(fields.tile_count)
            .ok_or("block-entity count overflow")?;
        if total_entities > limits.max_entities || total_tiles > limits.max_block_entities {
            return Err("entity or block-entity limit exceeded".into());
        }
        builder.begin_source()?;
        let mapping = builder.register_palette(&palette)?;
        // Give this region's entities a later source rank than its blocks.
        builder.begin_source()?;
        for mut entity in fields.entities {
            current()?;
            entity.position.0 += f64::from(origin.0);
            entity.position.1 += f64::from(origin.1);
            entity.position.2 += f64::from(origin.2);
            builder.push_entity(&entity)?;
        }
        let tiles = fields.tiles?;
        for &position in &tiles {
            current()?;
            super::offset_position(position, min)?;
        }
        builder.add_block_entities(tiles.len())?;
        prepared.try_reserve(1).map_err(|error| error.to_string())?;
        prepared.push(PreparedRegion {
            min,
            width,
            length,
            plane: width
                .checked_mul(length)
                .ok_or("region plane size overflow")?,
            volume,
            bits,
            packed,
            palette,
            mapping,
        });
    }
    if total_volume > limits.max_volume || region_count > limits.max_regions {
        return Err("default region exceeds region or volume limit".into());
    }
    Ok(prepared)
}

fn visit<F: Fn() -> Result<(), String>>(
    input: &mut Input<'_, F>,
    region: &PreparedRegion,
    builder: &mut CompactBlocksBuilder,
    current: &F,
) -> Result<(), String> {
    let mut word = 0u64;
    let mut remaining = 0u32;
    let mask = 1u64.checked_shl(region.bits).unwrap_or(0).wrapping_sub(1);
    for index in 0..region.volume {
        if index % BUFFER_SIZE == 0 {
            current()?;
        }
        let value = if remaining >= region.bits {
            let value = word;
            word = word.checked_shr(region.bits).unwrap_or(0);
            remaining -= region.bits;
            value
        } else {
            let next = u64::from_be_bytes(input.number()?);
            let value = word | (next << remaining);
            let used = region.bits - remaining;
            word = next.checked_shr(used).unwrap_or(0);
            remaining = 64 - used;
            value
        };
        let palette_index = (value & mask) as usize;
        let state = region
            .palette
            .get(palette_index)
            .ok_or("packed block palette index out of range")?;
        if state.name == "minecraft:air" && state.properties.is_empty() {
            continue;
        }
        let position = BlockPosition::new(
            region.min.0 + (index % region.width) as i32,
            region.min.1 + (index / region.plane) as i32,
            region.min.2 + ((index / region.width) % region.length) as i32,
        );
        builder.push_block(position, &region.mapping[palette_index])?;
    }
    Ok(())
}

fn visit_parallel<F: Fn() -> Result<(), String>>(
    input: &mut Input<'_, F>,
    region: &PreparedRegion,
    builder: &mut CompactBlocksBuilder,
    workers: usize,
    speed_first: bool,
    current: &F,
) -> Result<(), String> {
    // Batches begin on a 64-bit boundary (BATCH_BLOCKS is divisible by 64).
    // Only the last batch may end inside a word; every packed word is read once.
    let jobs = (0..region.volume)
        .step_by(crate::parallel::BATCH_BLOCKS)
        .map(|start| {
            let count = (region.volume - start).min(crate::parallel::BATCH_BLOCKS);
            let word_count = (count * region.bits as usize).div_ceil(64);
            let mut words = Vec::new();
            words
                .try_reserve_exact(word_count)
                .map_err(|e| e.to_string())?;
            for _ in 0..word_count {
                words.push(u64::from_be_bytes(input.number()?));
            }
            Ok((start, count, words))
        });
    crate::parallel::ordered(
        jobs,
        workers,
        speed_first,
        |(start, count, words), cancelled| {
            let mask = 1u64.checked_shl(region.bits).unwrap_or(0).wrapping_sub(1);
            let mut blocks = Vec::new();
            blocks.try_reserve_exact(count).map_err(|e| e.to_string())?;
            for offset in 0..count {
                if offset % 1024 == 0 {
                    crate::parallel::check_cancelled(cancelled)?;
                }
                let bit = offset * region.bits as usize;
                let word = bit / 64;
                let shift = bit % 64;
                let mut value = words[word] >> shift;
                if shift + region.bits as usize > 64 {
                    value |= words[word + 1] << (64 - shift);
                }
                let palette_index = (value & mask) as usize;
                let state = region
                    .palette
                    .get(palette_index)
                    .ok_or("packed block palette index out of range")?;
                if state.name == "minecraft:air" && state.properties.is_empty() {
                    continue;
                }
                let index = start + offset;
                let position = BlockPosition::new(
                    region.min.0 + (index % region.width) as i32,
                    region.min.1 + (index / region.plane) as i32,
                    region.min.2 + ((index / region.width) % region.length) as i32,
                );
                let palette_index = u32::try_from(palette_index)
                    .map_err(|_| "packed block palette index exceeds u32")?;
                blocks.push((position, palette_index));
            }
            Ok(blocks)
        },
        |blocks| {
            for (position, palette_index) in blocks {
                builder.push_block(position, &region.mapping[palette_index as usize])?;
            }
            Ok(())
        },
        current,
    )
}
