// Minimal Slang fixture for the engine-shader oracle (Phase 4 PR 4).
// Three entry points cover every stage we route through slangc.

struct VsOut {
    float4 pos : SV_Position;
    float2 uv  : TEXCOORD0;
};

[shader("vertex")]
VsOut vs_main(uint vid : SV_VertexID) {
    // Procedural fullscreen triangle — no vertex buffer required.
    float2 p = float2((vid << 1) & 2, vid & 2);
    VsOut o;
    o.pos = float4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = p;
    return o;
}

[shader("fragment")]
float4 fs_main(VsOut i) : SV_Target {
    return float4(i.uv, 0.0, 1.0);
}

[shader("compute")]
[numthreads(8, 8, 1)]
void cs_main(uint3 tid : SV_DispatchThreadID) {
    // Intentionally no resource declarations; PR 4 only needs the
    // entry-point + stage handshake to verify slangc round-trips.
    let _ = tid.x + tid.y;
}
