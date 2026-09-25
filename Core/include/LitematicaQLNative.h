#ifndef LITEMATICAQL_NATIVE_H
#define LITEMATICAQL_NATIVE_H

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
    NQL_ERR_CANCELLED = 8
} NQLStatus;

typedef struct {
    int64_t block_count;
    int64_t block_entity_count;
    int32_t content_x;
    int32_t content_y;
    int32_t content_z;
} NQLSchematicInfo;

typedef struct {
    int64_t triangle_count;
} NQLMeshInfo;

typedef struct {
    uint8_t *message;
    size_t message_len;
} NQLError;

NQLStatus nql_schematic_open(const uint8_t *data, size_t len, NQLSchematic **out, NQLError *err);
void nql_schematic_free(NQLSchematic *schematic);
NQLStatus nql_schematic_cancel(const NQLSchematic *schematic);
NQLStatus nql_schematic_info(const NQLSchematic *schematic, NQLSchematicInfo *out);
NQLStatus nql_schematic_warnings(const NQLSchematic *schematic, uint8_t **out, size_t *out_len);
NQLStatus nql_resource_pack_open(const uint8_t *data, size_t len, NQLResourcePack **out, NQLError *err);
void nql_resource_pack_free(NQLResourcePack *pack);
NQLStatus nql_schematic_mesh(const NQLSchematic *schematic, const NQLResourcePack *pack,
                             uint8_t **glb_out, size_t *glb_len,
                             NQLMeshInfo *info_out, NQLError *err);
void nql_buffer_free(uint8_t *buffer, size_t len);

#ifdef __cplusplus
}
#endif

#endif /* LITEMATICAQL_NATIVE_H */
