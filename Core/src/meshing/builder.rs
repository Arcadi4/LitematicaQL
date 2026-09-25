//! Bounded geometry emission through schematic-mesher's public API.

use schematic_mesher::mesh_output::GreedyMaterialOutput;
use schematic_mesher::mesher::{
    element::MeshBuilder, face_culler::FaceCuller, liquid, AnimatedTextureExport, Mesh,
};
use schematic_mesher::resolver::{resolve_block, ModelResolver};
use schematic_mesher::{
    BlockPosition, BoundingBox, InputBlock, MeshLayer, MeshOutput, MesherConfig, ResourcePack,
    TextureAtlas,
};
use std::collections::HashSet;

pub(super) fn atlas_only(pack: &ResourcePack, block: &InputBlock) -> bool {
    let Ok(models) = resolve_block(pack, block) else {
        return true;
    };
    let resolver = ModelResolver::new(pack);
    models.iter().any(|resolved| {
        let textures = resolver.resolve_textures(&resolved.model);
        resolved.model.elements.iter().any(|element| {
            element.faces.values().any(|face| {
                let path = face
                    .texture
                    .strip_prefix('#')
                    .map_or(face.texture.as_str(), |key| {
                        textures.get(key).map_or("block/missing", String::as_str)
                    });
                pack.get_texture(path)
                    .is_none_or(|texture| texture.has_transparency() && !texture.has_translucency())
            })
        })
    })
}

pub(super) fn mesh(
    pack: &ResourcePack,
    config: &MesherConfig,
    shared_atlas: &TextureAtlas,
    palette: &[InputBlock],
    atlas_only: &[bool],
    blocks: &[(BlockPosition, u32)],
    context: &[(BlockPosition, &InputBlock)],
    bounds: BoundingBox,
) -> Result<MeshOutput, String> {
    if config.cull_hidden_faces {
        validate_culler_grid(context)?;
    }
    let culler = config
        .cull_hidden_faces
        .then(|| FaceCuller::new(pack, context));
    let liquids: rustc_hash::FxHashMap<_, _> = context
        .iter()
        .copied()
        .filter(|(_, block)| {
            liquid::FluidState::from_block(block).is_some() || liquid::is_waterlogged(block)
        })
        .collect();
    let liquid_map = (!liquids.is_empty()).then_some(&liquids);
    let mut output = MeshOutput {
        opaque: MeshLayer::new(),
        cutout: MeshLayer::new(),
        transparent: MeshLayer::new(),
        atlas: shared_atlas.clone(),
        greedy_materials: Vec::new(),
        animated_textures: Vec::new(),
        bounds,
        chunk_coord: None,
        lod_level: 0,
    };
    let mut atlas_config = config.clone();
    atlas_config.greedy_meshing = false;
    for use_atlas in [false, true] {
        if use_atlas && !config.greedy_meshing {
            break;
        }
        let selected = |state: u32| config.greedy_meshing && atlas_only[state as usize];
        if !blocks
            .iter()
            .any(|&(_, state)| selected(state) == use_atlas)
        {
            continue;
        }
        let active_config = if use_atlas { &atlas_config } else { config };
        let mut builder = MeshBuilder::new(pack, active_config, culler.as_ref(), liquid_map, None);
        for &(position, state) in blocks {
            if selected(state) != use_atlas {
                continue;
            }
            let block = &palette[state as usize];
            if config.cull_occluded_blocks
                && culler
                    .as_ref()
                    .is_some_and(|c| c.is_fully_occluded(position))
            {
                continue;
            }
            builder
                .add_block(position, block)
                .map_err(|error| error.to_string())?;
        }
        collect_pack_animations(
            pack,
            shared_atlas,
            builder.texture_refs(),
            &mut output.animated_textures,
        )?;
        let supplied = std::mem::replace(
            &mut output.atlas,
            TextureAtlas {
                width: 0,
                height: 0,
                pixels: Vec::new(),
                regions: Default::default(),
            },
        );
        let pixel_storage = supplied.pixels.as_ptr();
        let (opaque, cutout, transparent, atlas, materials, animated) = builder
            .build(Some(supplied))
            .map_err(|error| error.to_string())?;
        validate_atlas(shared_atlas, &atlas, pixel_storage)?;
        output.atlas = atlas;
        append_layer(&mut output.opaque, opaque)?;
        append_layer(&mut output.cutout, cutout)?;
        append_layer(&mut output.transparent, transparent)?;
        for material in materials {
            if material.texture_png.is_empty() {
                return Err(format!(
                    "Greedy material {} has no texture.",
                    material.texture_path
                ));
            }
            output.greedy_materials.push(GreedyMaterialOutput {
                texture_path: material.texture_path,
                opaque: into_layer(material.opaque_mesh),
                transparent: into_layer(material.transparent_mesh),
                texture_png: material.texture_png,
            });
        }
        for animation in animated {
            if !output.animated_textures.iter().any(|existing| {
                existing.atlas_x == animation.atlas_x && existing.atlas_y == animation.atlas_y
            }) {
                output.animated_textures.push(animation);
            }
        }
    }
    Ok(output)
}

fn validate_culler_grid(context: &[(BlockPosition, &InputBlock)]) -> Result<(), String> {
    if context.is_empty() {
        return Ok(());
    }
    let mut min = [i32::MAX; 3];
    let mut max = [i32::MIN; 3];
    for (pos, _) in context {
        for (axis, value) in [pos.x, pos.y, pos.z].into_iter().enumerate() {
            min[axis] = min[axis].min(value);
            max[axis] = max[axis].max(value);
        }
    }
    let mut volume = 1usize;
    for axis in 0..3 {
        let padded_min = min[axis]
            .checked_sub(1)
            .ok_or("Culling bounds exceed i32 coordinates.")?;
        let padded_max = max[axis]
            .checked_add(1)
            .ok_or("Culling bounds exceed i32 coordinates.")?;
        let size = padded_max
            .checked_sub(padded_min)
            .and_then(|span| span.checked_add(1))
            .ok_or("Culling dimensions exceed i32 representation.")?;
        volume = volume
            .checked_mul(size as usize)
            .filter(|&value| value <= isize::MAX as usize)
            .ok_or("Culling grid exceeds addressable memory.")?;
    }
    Ok(())
}

fn collect_pack_animations(
    pack: &ResourcePack,
    atlas: &TextureAtlas,
    references: &HashSet<String>,
    output: &mut Vec<AnimatedTextureExport>,
) -> Result<(), String> {
    let mut references: Vec<_> = references.iter().collect();
    references.sort_unstable();
    for path in references {
        let Some(texture) = pack
            .get_texture(path)
            .filter(|t| t.is_animated && t.frame_count > 1)
        else {
            continue;
        };
        let Some(region) = atlas.get_region(path) else {
            return Err(format!(
                "Animated texture {path} is absent from the shared atlas."
            ));
        };
        let atlas_x = (region.u_min * atlas.width as f32).round() as u32;
        let atlas_y = (region.v_min * atlas.height as f32).round() as u32;
        if output
            .iter()
            .any(|existing| existing.atlas_x == atlas_x && existing.atlas_y == atlas_y)
        {
            continue;
        }
        let animation = texture.animation.as_ref();
        let frame_width = animation
            .and_then(|a| a.frame_width)
            .unwrap_or(texture.width);
        output.push(AnimatedTextureExport {
            sprite_sheet_png: texture.to_png().map_err(|error| error.to_string())?,
            frame_count: texture.frame_count,
            frametime: animation.map_or(1, |a| a.frametime),
            interpolate: animation.is_some_and(|a| a.interpolate),
            frames: animation
                .and_then(|a| a.frames.as_ref())
                .map(|frames| frames.iter().map(|frame| frame.index).collect()),
            frame_width,
            frame_height: animation
                .and_then(|a| a.frame_height)
                .unwrap_or(frame_width),
            atlas_x,
            atlas_y,
        });
    }
    Ok(())
}

fn validate_atlas(
    shared: &TextureAtlas,
    actual: &TextureAtlas,
    pixel_storage: *const u8,
) -> Result<(), String> {
    if actual.pixels.as_ptr() != pixel_storage
        || actual.width != shared.width
        || actual.height != shared.height
        || actual.pixels.len() != shared.pixels.len()
        || actual.regions.len() != shared.regions.len()
        || shared.regions.iter().any(|(path, region)| {
            actual.get_region(path).is_none_or(|actual| {
                [actual.u_min, actual.v_min, actual.u_max, actual.v_max]
                    != [region.u_min, region.v_min, region.u_max, region.v_max]
            })
        })
    {
        return Err(
            "A mesh chunk changed the shared texture atlas; texture discovery is incomplete."
                .into(),
        );
    }
    Ok(())
}

fn append_layer(target: &mut MeshLayer, mut source: MeshLayer) -> Result<(), String> {
    if target.positions.is_empty() {
        *target = source;
        return Ok(());
    }
    let offset =
        u32::try_from(target.positions.len()).map_err(|_| "Too many vertices in a mesh chunk.")?;
    let total = target
        .positions
        .len()
        .checked_add(source.positions.len())
        .ok_or("Too many vertices in a mesh chunk.")?;
    u32::try_from(total).map_err(|_| "Too many vertices in a mesh chunk.")?;
    target.positions.append(&mut source.positions);
    target.normals.append(&mut source.normals);
    target.uvs.append(&mut source.uvs);
    target.colors.append(&mut source.colors);
    target
        .indices
        .extend(source.indices.into_iter().map(|index| index + offset));
    Ok(())
}

fn into_layer(mesh: Mesh) -> MeshLayer {
    let count = mesh.vertices.len();
    let mut layer = MeshLayer {
        positions: Vec::with_capacity(count),
        normals: Vec::with_capacity(count),
        uvs: Vec::with_capacity(count),
        colors: Vec::with_capacity(count),
        indices: mesh.indices,
    };
    for vertex in mesh.vertices {
        layer.positions.push(vertex.position);
        layer.normals.push(vertex.normal);
        layer.uvs.push(vertex.uv);
        layer.colors.push(vertex.color);
    }
    layer
}
