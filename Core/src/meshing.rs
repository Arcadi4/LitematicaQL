//! Application-owned compact chunk scheduling over Nucleation's public data model.
//!
//! Nucleation 0.10.14 block/entity and chunk scheduling; see NOTICE and
//! ThirdParty/Nucleation-LICENSE.txt.
use nucleation::meshing::{MeshConfig, MeshOutput, ResourcePackSource};
use nucleation::{BlockState, Entity, NbtValue, UniversalSchematic};
use schematic_mesher::{
    BlockPosition, BoundingBox, InputBlock, MesherConfig, ResourcePack, TextureAtlas,
};
use std::collections::{hash_map::Entry, HashMap};

mod atlas;
mod builder;

pub(crate) type ChunkCoord = (i32, i32, i32);
type IndexedBlock = (BlockPosition, u32);

pub(crate) struct CompactBlocks {
    pub(crate) chunks: Vec<(ChunkCoord, Vec<IndexedBlock>)>,
    palette: Vec<InputBlock>,
    block_count: i64,
    block_entity_count: i64,
    tight_min: [i32; 3],
    tight_max: [i32; 3],
    has_blocks: bool,
}

pub(crate) struct PaletteEntry {
    index: Option<u32>,
    counted: bool,
}

pub(crate) struct CompactBlocksBuilder {
    chunks: HashMap<ChunkCoord, Vec<IndexedBlock>>,
    palette: Vec<InputBlock>,
    states: HashMap<BlockState, u32>,
    chunk_size: Option<i32>,
    block_count: i64,
    block_entity_count: i64,
    source_order: Option<Vec<(usize, u32)>>,
    source: usize,
    tight_min: [i32; 3],
    tight_max: [i32; 3],
    has_blocks: bool,
}

impl CompactBlocksBuilder {
    pub(crate) fn new(chunk_size: Option<i32>) -> Result<Self, String> {
        if chunk_size.is_some_and(|size| size <= 0) {
            return Err("The mesh chunk size must be positive.".into());
        }
        Ok(Self {
            chunks: HashMap::new(),
            palette: Vec::new(),
            states: HashMap::new(),
            chunk_size,
            block_count: 0,
            block_entity_count: 0,
            source_order: None,
            source: 0,
            tight_min: [i32::MAX; 3],
            tight_max: [i32::MIN; 3],
            has_blocks: false,
        })
    }

    pub(crate) fn begin_source(&mut self) -> Result<(), String> {
        self.source = self.source.checked_add(1).ok_or("source order overflow")?;
        self.source_order.get_or_insert_with(Vec::new);
        Ok(())
    }

    fn source_index(&mut self, index: u32) -> Result<u32, String> {
        let Some(order) = &mut self.source_order else {
            return Ok(index);
        };
        let alias = u32::try_from(order.len())
            .map_err(|_| "The schematic has too many source block states.")?;
        order.try_reserve(1).map_err(|error| error.to_string())?;
        order.push((self.source, index));
        Ok(alias)
    }

    pub(crate) fn register_palette(
        &mut self,
        palette: &[BlockState],
    ) -> Result<Vec<PaletteEntry>, String> {
        let mut remap = Vec::new();
        remap
            .try_reserve_exact(palette.len())
            .map_err(|e| e.to_string())?;
        for state in palette {
            let counted = state.name != "minecraft:air" || !state.properties.is_empty();
            let index = if is_air(&state.name) {
                None
            } else {
                let index = match self.states.entry(state.clone()) {
                    Entry::Occupied(entry) => *entry.get(),
                    Entry::Vacant(entry) => {
                        let index = u32::try_from(self.palette.len())
                            .map_err(|_| "The schematic has too many block states.")?;
                        self.palette.try_reserve(1).map_err(|e| e.to_string())?;
                        self.palette.push(block_state_to_input_block(entry.key()));
                        entry.insert(index);
                        index
                    }
                };
                Some(self.source_index(index)?)
            };
            remap.push(PaletteEntry { index, counted });
        }
        Ok(remap)
    }

    pub(crate) fn push_block(
        &mut self,
        position: BlockPosition,
        entry: &PaletteEntry,
    ) -> Result<(), String> {
        if entry.counted {
            self.block_count = self
                .block_count
                .checked_add(1)
                .ok_or("Block count exceeds i64.")?;
        }
        if let Some(index) = entry.index {
            self.push_index(position, index)?;
        }
        Ok(())
    }

    pub(crate) fn push_entity(&mut self, entity: &Entity) -> Result<(), String> {
        let position = BlockPosition::new(
            entity.position.0.floor() as i32,
            entity.position.1.floor() as i32,
            entity.position.2.floor() as i32,
        );
        let index = u32::try_from(self.palette.len())
            .map_err(|_| "The schematic has too many block states.")?;
        self.palette.try_reserve(1).map_err(|e| e.to_string())?;
        self.palette.push(entity_to_input_block(entity));
        let index = self.source_index(index)?;
        self.push_index(position, index)
    }

    pub(crate) fn add_block_entities(&mut self, count: usize) -> Result<(), String> {
        self.block_entity_count = self
            .block_entity_count
            .checked_add(i64::try_from(count).map_err(|_| "Block entity count exceeds i64.")?)
            .ok_or("Block entity count exceeds i64.")?;
        Ok(())
    }

    fn push_index(&mut self, position: BlockPosition, index: u32) -> Result<(), String> {
        let coord = self
            .chunk_size
            .map_or((0, 0, 0), |size| chunk_coord(position, size));
        let blocks = self.chunks.entry(coord).or_default();
        blocks.try_reserve(1).map_err(|e| e.to_string())?;
        blocks.push((position, index));
        for (axis, value) in [position.x, position.y, position.z].into_iter().enumerate() {
            self.tight_min[axis] = self.tight_min[axis].min(value);
            self.tight_max[axis] = self.tight_max[axis].max(value);
        }
        self.has_blocks = true;
        Ok(())
    }

    pub(crate) fn finish(self) -> CompactBlocks {
        let mut chunks: Vec<_> = self.chunks.into_iter().collect();
        chunks.sort_unstable_by_key(|(coord, _)| *coord);
        for (_, blocks) in &mut chunks {
            if let Some(order) = &self.source_order {
                blocks.sort_by_key(|(pos, index)| (pos.y, pos.z, pos.x, order[*index as usize].0));
                for (_, index) in blocks {
                    *index = order[*index as usize].1;
                }
            } else {
                blocks.sort_by_key(|(pos, _)| (pos.y, pos.z, pos.x));
            }
        }
        CompactBlocks {
            chunks,
            palette: self.palette,
            block_count: self.block_count,
            block_entity_count: self.block_entity_count,
            tight_min: self.tight_min,
            tight_max: self.tight_max,
            has_blocks: self.has_blocks,
        }
    }
}

impl CompactBlocks {
    pub(crate) fn from_schematic(
        schematic: UniversalSchematic,
        chunk_size: Option<i32>,
        current: &impl Fn() -> Result<(), String>,
    ) -> Result<Self, String> {
        let mut builder = CompactBlocksBuilder::new(chunk_size)?;
        let UniversalSchematic {
            default_region,
            other_regions,
            ..
        } = schematic;
        let mut regions: Vec<_> = other_regions.into_iter().collect();
        regions.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
        for region in std::iter::once(default_region).chain(regions.into_iter().map(|(_, r)| r)) {
            current()?;
            builder.block_count = builder
                .block_count
                .checked_add(
                    i64::try_from(region.count_blocks()).map_err(|_| "Block count exceeds i64.")?,
                )
                .ok_or("Block count exceeds i64.")?;
            let remap = builder.register_palette(&region.get_palette())?;
            for (index, &state) in region.blocks.iter().enumerate() {
                let entry = remap
                    .get(state)
                    .ok_or("The schematic contains an invalid palette index.")?;
                if let Some(state) = entry.index {
                    let (x, y, z) = region.index_to_coords(index);
                    builder.push_index(BlockPosition::new(x, y, z), state)?;
                }
            }
            for entity in &region.entities {
                builder.push_entity(entity)?;
            }
            builder.add_block_entities(region.block_entities.len())?;
        }
        Ok(builder.finish())
    }

    pub(crate) fn block_count(&self) -> i64 {
        self.block_count
    }

    pub(crate) fn block_entity_count(&self) -> i64 {
        self.block_entity_count
    }

    pub(crate) fn content_dimensions(&self) -> (i32, i32, i32) {
        if self.has_blocks {
            let dimensions =
                [0usize, 1, 2].map(|axis| self.tight_max[axis] - self.tight_min[axis] + 1);
            (dimensions[0], dimensions[1], dimensions[2])
        } else {
            (0, 0, 0)
        }
    }

    fn atlas(
        &self,
        pack: &ResourcePackSource,
        config: &MesherConfig,
    ) -> Result<TextureAtlas, String> {
        let positioned: Vec<_> = self
            .palette
            .iter()
            .map(|block| block.name.starts_with("entity:") || atlas::position_dependent(block))
            .collect();
        let representatives = self
            .palette
            .iter()
            .zip(&positioned)
            .filter(|(_, positioned)| !**positioned)
            .map(|(block, _)| (BlockPosition::new(0, 0, 0), block));
        let chunks = if positioned.iter().any(|&value| value) {
            self.chunks.as_slice()
        } else {
            &[]
        };
        let actual_positions = chunks.iter().flat_map(|(_, blocks)| {
            let positioned = &positioned;
            blocks.iter().filter_map(move |&(pos, index)| {
                positioned[index as usize].then_some((pos, &self.palette[index as usize]))
            })
        });
        atlas::build(pack.pack(), config, representatives.chain(actual_positions))
            .map_err(|error| format!("Unable to prepare schematic textures: {error}"))
    }

    fn context(
        &self,
        coord: ChunkCoord,
        chunk_size: Option<i32>,
    ) -> Vec<(BlockPosition, &InputBlock)> {
        let (min, max) = chunk_bounds(coord, chunk_size.unwrap_or(1));
        let capacity = self
            .chunks
            .binary_search_by_key(&coord, |(coord, _)| *coord)
            .map_or(0, |index| self.chunks[index].1.len());
        let mut context = Vec::with_capacity(capacity);
        for dx in -1i32..=1 {
            for dy in -1i32..=1 {
                for dz in -1i32..=1 {
                    let (Some(x), Some(y), Some(z)) = (
                        coord.0.checked_add(dx),
                        coord.1.checked_add(dy),
                        coord.2.checked_add(dz),
                    ) else {
                        continue;
                    };
                    let Ok(index) = self
                        .chunks
                        .binary_search_by_key(&(x, y, z), |(coord, _)| *coord)
                    else {
                        continue;
                    };
                    let blocks = &self.chunks[index].1;
                    let start = blocks.partition_point(|(pos, _)| i64::from(pos.y) < min[1] - 1);
                    let end = blocks.partition_point(|(pos, _)| i64::from(pos.y) < max[1] + 1);
                    for &(pos, state) in &blocks[start..end] {
                        if i64::from(pos.x) >= min[0] - 1
                            && i64::from(pos.x) < max[0] + 1
                            && i64::from(pos.z) >= min[2] - 1
                            && i64::from(pos.z) < max[2] + 1
                        {
                            context.push((pos, &self.palette[state as usize]));
                        }
                    }
                }
            }
        }
        context
    }
}

fn chunk_coord(pos: BlockPosition, size: i32) -> ChunkCoord {
    (
        pos.x.div_euclid(size),
        pos.y.div_euclid(size),
        pos.z.div_euclid(size),
    )
}

fn chunk_bounds(coord: ChunkCoord, size: i32) -> ([i64; 3], [i64; 3]) {
    let size = i64::from(size);
    let min = [coord.0, coord.1, coord.2].map(|value| i64::from(value) * size);
    (min, min.map(|value| value + size))
}

pub(super) struct ChunkMeshes<'a> {
    source: &'a CompactBlocks,
    index: usize,
    chunk_size: i32,
    pack: &'a ResourcePack,
    config: MesherConfig,
    atlas: TextureAtlas,
    atlas_only: Vec<bool>,
}

impl<'a> ChunkMeshes<'a> {
    pub(super) fn from_source(
        source: &'a CompactBlocks,
        pack: &'a ResourcePackSource,
        config: &MeshConfig,
        current: &impl Fn() -> Result<(), String>,
    ) -> Result<Self, String> {
        current()?;
        let config = mesher_config(config);
        let atlas = source.atlas(pack, &config)?;
        let atlas_only = source
            .palette
            .iter()
            .map(|block| config.greedy_meshing && builder::atlas_only(pack.pack(), block))
            .collect();
        current()?;
        Ok(Self {
            source,
            index: 0,
            chunk_size: 64,
            pack: pack.pack(),
            config,
            atlas,
            atlas_only,
        })
    }

    pub(super) fn consume(
        &mut self,
        workers: usize,
        consume: impl FnMut(MeshOutput) -> Result<(), String>,
        current: &impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        if self.source.chunks.is_empty() {
            return Ok(());
        }
        crate::parallel::ordered(
            (self.index..self.source.chunks.len()).map(Ok),
            workers,
            true,
            |index, cancelled| {
                crate::parallel::check_cancelled(cancelled)?;
                let mesh = self.mesh_at(index).map_err(|error| {
                    format!("This schematic is too detailed to preview: {error}")
                })?;
                crate::parallel::check_cancelled(cancelled)?;
                Ok(mesh)
            },
            consume,
            current,
        )?;
        self.index = self.source.chunks.len();
        Ok(())
    }

    fn mesh_at(&self, index: usize) -> Result<MeshOutput, String> {
        let (coord, blocks) = &self.source.chunks[index];
        let (min, max) = chunk_bounds(*coord, self.chunk_size);
        let bounds = BoundingBox::new(min.map(|value| value as f32), max.map(|value| value as f32));
        let context = self.source.context(*coord, Some(self.chunk_size));
        let mut mesh = builder::mesh(
            self.pack,
            &self.config,
            &self.atlas,
            &self.source.palette,
            &self.atlas_only,
            blocks,
            &context,
            bounds,
        )?;
        mesh.chunk_coord = Some(*coord);
        Ok(mesh)
    }
}

fn mesher_config(config: &MeshConfig) -> MesherConfig {
    let mut result = MesherConfig {
        cull_hidden_faces: config.cull_hidden_faces,
        ambient_occlusion: config.ambient_occlusion,
        ao_intensity: config.ao_intensity,
        atlas_max_size: config.atlas_max_size,
        cull_occluded_blocks: config.cull_occluded_blocks,
        greedy_meshing: config.greedy_meshing,
        ..MesherConfig::default()
    };
    if let Some(biome) = &config.biome {
        result = result.with_biome(biome);
    }
    result
}

fn is_air(name: &str) -> bool {
    matches!(
        name,
        "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
    )
}

fn block_state_to_input_block(state: &BlockState) -> InputBlock {
    let mut input = InputBlock::new(state.name.to_string());
    if let Some(facts) = nucleation::blockpedia::get_block(&state.name) {
        for (key, value) in facts.default_state {
            input.properties.insert(key.to_string(), value.to_string());
        }
    }
    for (key, value) in &state.properties {
        input.properties.insert(key.to_string(), value.to_string());
    }
    input
}

fn entity_to_input_block(entity: &Entity) -> InputBlock {
    let entity_id = entity.id.strip_prefix("minecraft:").unwrap_or(&entity.id);
    let mesher_id = match entity_id {
        "furnace_minecart"
        | "chest_minecart"
        | "tnt_minecart"
        | "hopper_minecart"
        | "spawner_minecart"
        | "command_block_minecart" => "minecart",
        id if id.ends_with("_chest_boat") || id.ends_with("_chest_raft") => "chest_boat",
        id if id.ends_with("_boat") || id.ends_with("_raft") => "boat",
        id => id,
    };
    let mut input = InputBlock::new(format!("entity:{mesher_id}"));
    if let Some(NbtValue::List(rotation)) = entity.nbt.get("Rotation") {
        if let Some(NbtValue::Float(yaw)) = rotation.first() {
            input
                .properties
                .insert("facing".into(), yaw_to_facing(*yaw).into());
        }
    }
    if let Some(NbtValue::Compound(item)) = entity.nbt.get("Item") {
        if let Some(NbtValue::String(id)) = item.get("id") {
            input.properties.insert("item".into(), id.clone());
        }
    }
    if matches!(entity.nbt.get("IsBaby"), Some(NbtValue::Byte(1)))
        || matches!(entity.nbt.get("Age"), Some(NbtValue::Int(age)) if *age < 0)
    {
        input.properties.insert("is_baby".into(), "true".into());
    }
    if let Some(NbtValue::Byte(color)) = entity.nbt.get("Color") {
        input
            .properties
            .insert("color".into(), dye_color_name(*color as u8).into());
    }
    for pose_key in [
        "RightArmPose",
        "LeftArmPose",
        "RightLegPose",
        "LeftLegPose",
        "HeadPose",
        "BodyPose",
    ] {
        if let Some(NbtValue::List(angles)) = entity.nbt.get(pose_key) {
            use std::fmt::Write;
            let mut pose = String::new();
            for angle in angles {
                if let NbtValue::Float(angle) = angle {
                    if !pose.is_empty() {
                        pose.push(',');
                    }
                    write!(&mut pose, "{angle}").expect("writing to a String cannot fail");
                }
            }
            if !pose.is_empty() {
                input.properties.insert(pose_key.into(), pose);
            }
        }
    }
    if let Some(NbtValue::List(items)) = entity.nbt.get("ArmorItems") {
        for (index, property) in ["boots", "leggings", "chestplate", "helmet"]
            .iter()
            .enumerate()
        {
            if let Some(NbtValue::Compound(item)) = items.get(index) {
                if let Some(NbtValue::String(id)) = item.get("id") {
                    input.properties.insert((*property).into(), id.clone());
                }
            }
        }
    }
    input
}

fn yaw_to_facing(yaw: f32) -> &'static str {
    let normalized = ((yaw % 360.0) + 360.0) % 360.0;
    if !(45.0..315.0).contains(&normalized) {
        "south"
    } else if (45.0..135.0).contains(&normalized) {
        "west"
    } else if (135.0..225.0).contains(&normalized) {
        "north"
    } else {
        "east"
    }
}

fn dye_color_name(color: u8) -> &'static str {
    match color {
        0 => "white",
        1 => "orange",
        2 => "magenta",
        3 => "light_blue",
        4 => "yellow",
        5 => "lime",
        6 => "pink",
        7 => "gray",
        8 => "light_gray",
        9 => "cyan",
        10 => "purple",
        11 => "blue",
        12 => "brown",
        13 => "green",
        14 => "red",
        15 => "black",
        _ => "white",
    }
}
