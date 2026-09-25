//! Discover shared atlas tiles through schematic-mesher's public APIs.

use schematic_mesher::atlas::{AtlasBuilder, AtlasRegion, TextureAtlas};
use schematic_mesher::mesher::{element::MeshBuilder, entity};
use schematic_mesher::resource_pack::TextureData;
use schematic_mesher::{BlockPosition, InputBlock, MesherConfig, ResourcePack};
use std::collections::HashSet;

pub(super) fn position_dependent(block: &InputBlock) -> bool {
    matches!(entity::detect_mob(block), Some(entity::MobType::Player))
        || matches!(
            entity::detect_block_entity(block),
            Some(entity::BlockEntityType::Skull(entity::SkullType::Player))
        )
        || block.properties.contains_key("inventory")
        || block
            .properties
            .get("rider")
            .is_some_and(|rider| rider.trim_start_matches("entity:") == "player")
}

pub(super) fn build<'a>(
    pack: &ResourcePack,
    config: &MesherConfig,
    blocks: impl Iterator<Item = (BlockPosition, &'a InputBlock)>,
) -> Result<TextureAtlas, String> {
    let discovery = MesherConfig {
        cull_hidden_faces: false,
        cull_occluded_blocks: false,
        ambient_occlusion: false,
        greedy_meshing: false,
        enable_block_light: false,
        enable_sky_light: false,
        pre_built_atlas: None,
        atlas_max_size: config.atlas_max_size,
        atlas_padding: config.atlas_padding,
        include_air: config.include_air,
        tint_provider: config.tint_provider.clone(),
        ao_intensity: config.ao_intensity,
        sky_light_level: config.sky_light_level,
        enable_particles: config.enable_particles,
    };
    let mut atlas = AtlasBuilder::new(config.atlas_max_size, config.atlas_padding);
    let mut discovered = HashSet::new();

    let (_, _, _, missing, _, _) = MeshBuilder::new(pack, &discovery, None, None, None)
        .build(None)
        .map_err(|error| error.to_string())?;
    collect_tiles(missing, &mut atlas, &mut discovered)?;

    for (pos, block) in blocks {
        if !discovery.include_air && block.is_air() {
            continue;
        }
        let mut representative = MeshBuilder::new(pack, &discovery, None, None, None);
        representative
            .add_block(pos, block)
            .map_err(|error| error.to_string())?;

        let mut needs_local_atlas = false;
        for path in representative.texture_refs() {
            if discovered.contains(path) {
                continue;
            }
            if !path.starts_with('_') {
                if let Some(texture) = pack.get_texture(path) {
                    atlas.add_texture(path.clone(), texture.first_frame());
                    discovered.insert(path.clone());
                    continue;
                }
            }
            needs_local_atlas = true;
        }
        if needs_local_atlas {
            let (_, _, _, local, _, _) = representative
                .build(None)
                .map_err(|error| error.to_string())?;
            collect_tiles(local, &mut atlas, &mut discovered)?;
        }
    }
    atlas.build().map_err(|error| error.to_string())
}

fn collect_tiles(
    local: TextureAtlas,
    atlas: &mut AtlasBuilder,
    discovered: &mut HashSet<String>,
) -> Result<(), String> {
    for (path, region) in &local.regions {
        if !discovered.contains(path) {
            atlas.add_texture(path.clone(), extract_tile(&local, region)?);
            discovered.insert(path.clone());
        }
    }
    Ok(())
}

fn extract_tile(atlas: &TextureAtlas, region: &AtlasRegion) -> Result<TextureData, String> {
    let edge = |uv: f32, extent: u32| -> Result<u32, String> {
        let pixel = uv * extent as f32;
        if !pixel.is_finite() || pixel < 0.0 || pixel > extent as f32 || pixel.fract() != 0.0 {
            return Err("Mesher returned a nonintegral atlas region".into());
        }
        Ok(pixel as u32)
    };
    let x = edge(region.u_min, atlas.width)?;
    let y = edge(region.v_min, atlas.height)?;
    let right = edge(region.u_max, atlas.width)?;
    let bottom = edge(region.v_max, atlas.height)?;
    if right <= x || bottom <= y {
        return Err("Mesher returned an empty atlas region".into());
    }
    let width = right - x;
    let height = bottom - y;
    let stride = atlas.width as usize * 4;
    let row_bytes = width as usize * 4;
    let mut pixels = Vec::with_capacity(row_bytes * height as usize);
    for row in y..bottom {
        let start = row as usize * stride + x as usize * 4;
        let source = atlas
            .pixels
            .get(start..start + row_bytes)
            .ok_or("Mesher returned truncated atlas pixels")?;
        pixels.extend_from_slice(source);
    }
    Ok(TextureData::new(width, height, pixels))
}
