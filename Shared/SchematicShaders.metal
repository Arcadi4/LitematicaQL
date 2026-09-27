#include <metal_stdlib>
using namespace metal;

struct Vertex {
    float4 position [[attribute(0)]];
    float2 uv [[attribute(1)]];
    float4 normal [[attribute(2)]];
    float4 color [[attribute(3)]];
};
struct Frame { float4x4 viewProjection; };
struct Chunk { float4 origin; float4 extent; };
struct Varying {
    float4 position [[position]];
    float2 uv;
    half4 color;
    half light;
};

vertex Varying schematicVertex(Vertex in [[stage_in]], constant Frame &frame [[buffer(1)]],
                                constant Chunk &chunk [[buffer(2)]]) {
    Varying out;
    float3 world = chunk.origin.xyz + in.position.xyz * chunk.extent.xyz;
    out.position = frame.viewProjection * float4(world, 1.0);
    out.uv = in.uv;
    out.color = half4(in.color);
    // Directional light plus a hemisphere term; the mesher's vertex colors
    // already include biome tint and ambient occlusion.
    float3 normal = normalize(in.normal.xyz);
    out.light = half(0.58 + 0.22 * max(normal.y, 0.0) +
                     0.20 * max(dot(normal, normalize(float3(-0.4, 0.8, 0.5))), 0.0));
    return out;
}

fragment half4 schematicOpaque(Varying in [[stage_in]], texture2d<half> texture [[texture(0)]],
                                sampler filter [[sampler(0)]]) {
    half4 color = texture.sample(filter, in.uv) * in.color;
    return half4(color.rgb * in.light, 1.0h);
}
fragment half4 schematicCutout(Varying in [[stage_in]], texture2d<half> texture [[texture(0)]],
                                sampler filter [[sampler(0)]]) {
    half4 color = texture.sample(filter, in.uv) * in.color;
    if (color.a < 0.5h) discard_fragment();
    return half4(color.rgb * in.light, 1.0h);
}
fragment half4 schematicBlend(Varying in [[stage_in]], texture2d<half> texture [[texture(0)]],
                               sampler filter [[sampler(0)]]) {
    half4 color = texture.sample(filter, in.uv) * in.color;
    return half4(color.rgb * in.light, color.a);
}
