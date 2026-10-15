#ifndef LITEMATICAQL_NATIVE_H
#define LITEMATICAQL_NATIVE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct NQLSchematic NQLSchematic;
typedef struct NQLResourcePack NQLResourcePack;

typedef enum {
    NQL_OK = 0,
    NQL_ERR_NULL = 1,
    NQL_ERR_FORMAT = 2,
    NQL_ERR_LIMIT = 3,
    NQL_ERR_NO_BLOCKS = 4,
    NQL_ERR_MESH = 5,
    NQL_ERR_PACK = 6,
    NQL_ERR_INTERNAL = 7,
    NQL_ERR_CANCELLED = 8,
    NQL_DONE = 9
} NQLStatus;
typedef struct NQLMeshStream NQLMeshStream;
typedef struct NQLMeshBatch NQLMeshBatch;

// Vertices are 24 bytes: UNORM16x4 position, float2 UV, SNORM8x4 normal,
// UNORM8x4 color. Indices are uint16 or uint32, local to each part. Texture
// indices name an entry in the batch's texture views below, or zero for the
// shared atlas delivered at stream open. `min` and `max` are this part's
// actual meshed-geometry bounds in block-center coordinates, the same space as
// the batch's `origin`/`extent`; together they form the box a host tests
// against the frustum before encoding a part's draws.
typedef struct {
    size_t vertex_offset;
    size_t index_offset;
    uint32_t vertex_count;
    uint32_t index_count;
    uint32_t index_size;
    uint32_t alpha_mode;
    uint32_t texture_index;
    float min[3];
    float max[3];
} NQLMeshPart;

// One first-use RGBA8 sRGB texture owned by a batch. Texture zero is the
// clamped atlas, delivered once at stream open and never repeated; every
// other texture repeats and may generate mipmaps.
typedef struct {
    uint32_t index;
    uint32_t width;
    uint32_t height;
    bool repeats;
    const uint8_t *pixels;
    size_t length;
} NQLBatchTexture;

typedef struct {
    const uint8_t *geometry;
    size_t geometry_length;
    const NQLMeshPart *parts;
    size_t part_count;
    const NQLBatchTexture *textures;
    size_t texture_count;
    float origin[3];
    float extent[3];
} NQLBatchView;

typedef struct {
    int64_t block_count;
    int64_t block_entity_count;
    int32_t content_x;
    int32_t content_y;
    int32_t content_z;
    // Lower corner of the occupied-block box. Blocks span [min, min + size),
    // so max is not tight_max + 1 until geometry is meshed. Zero when the
    // schematic has no blocks.
    int32_t content_min_x;
    int32_t content_min_y;
    int32_t content_min_z;
} NQLSchematicInfo;

typedef struct {
    uint32_t batch_count;
    int64_t triangle_count;
} NQLMeshInfo;

typedef struct {
    uint32_t width;
    uint32_t height;
} NQLAtlasInfo;

typedef struct {
    uint32_t batch_index;
    uint32_t part_count;
    uint32_t vertex_count;
    uint32_t index_count;
    uint32_t triangle_count;
    uint32_t payload_length;
    float bounds_min[3];
    float bounds_max[3];
} NQLBatchInfo;

typedef struct {
    uint8_t *message;
    size_t message_len;
} NQLError;

NQLStatus nql_schematic_open(const uint8_t *data, size_t len, NQLSchematic **out, NQLError *err);
// Reads a schematic with no preview quotas. Only for limit hits the host has
// already offered to render past; admissions move to the caller from here.
NQLStatus nql_schematic_open_override(const uint8_t *data, size_t len,
                                      NQLSchematic **out, NQLError *err);
void nql_schematic_free(NQLSchematic *schematic);
NQLStatus nql_schematic_cancel(const NQLSchematic *schematic);
NQLStatus nql_schematic_info(const NQLSchematic *schematic, NQLSchematicInfo *out);
NQLStatus nql_schematic_warnings(const NQLSchematic *schematic, uint8_t **out, size_t *out_len);
NQLStatus nql_resource_pack_open(const uint8_t *data, size_t len, NQLResourcePack **out, NQLError *err);
// Creates an immutable custom pack over the base without modifying the base.
NQLStatus nql_resource_pack_overlay(const NQLResourcePack *base, const uint8_t *data, size_t len,
                                    NQLResourcePack **out, NQLError *err);
void nql_resource_pack_free(NQLResourcePack *pack);
NQLStatus nql_mesh_stream_open(const NQLSchematic *schematic,
                               const NQLResourcePack *pack,
                               uint8_t **atlas_rgba_out, size_t *atlas_rgba_len,
                               NQLAtlasInfo *atlas_info_out, NQLMeshInfo *info_out,
                               NQLMeshStream **stream_out, NQLError *err);
// Lifts the render-stage block budget and the shared texture atlas's ceiling;
// companion to nql_schematic_open_override. NQL_ERR_LIMIT from
// nql_mesh_stream_open is the cue to use this after the user has agreed to the
// cost the refusal described.
NQLStatus nql_mesh_stream_open_override(const NQLSchematic *schematic,
                                        const NQLResourcePack *pack,
                                        uint8_t **atlas_rgba_out, size_t *atlas_rgba_len,
                                        NQLAtlasInfo *atlas_info_out, NQLMeshInfo *info_out,
                                        NQLMeshStream **stream_out, NQLError *err);
NQLStatus nql_mesh_stream_next(NQLMeshStream *stream, uint32_t expected_batch,
                               NQLMeshBatch **batch_out,
                               NQLBatchInfo *info_out, NQLError *err);
// Schematic and resource pack must outlive the stream. Returned batch views
// are valid until batch_free and may be released before requesting the next.
NQLBatchView nql_mesh_batch_view(const NQLMeshBatch *batch);
void nql_mesh_batch_free(NQLMeshBatch *batch);
void nql_mesh_stream_free(NQLMeshStream *stream);
void nql_buffer_free(uint8_t *buffer, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* LITEMATICAQL_NATIVE_H */
