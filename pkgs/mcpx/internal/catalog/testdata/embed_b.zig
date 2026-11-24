// embed_b.zig — synthetic deterministic embedding guest, model B (#13616).
//
// Built with: zig build-exe embed_b.zig -target wasm32-freestanding -O ReleaseSmall -rdynamic -fno-entry -femit-bin=../embed_b.wasm
// Same protocol as embed_a.zig. Model B output: vec[0]=0.25, vec[1]=1.0,
// rest 0 — the mirror of model A, so identical inputs give clearly distinct
// vectors before and after a hot swap.

var input_buf: [4096]u8 align(8) = undefined;
var out_vec: [384]f32 align(8) = undefined;

export fn allocate(len: u32) u32 {
    _ = len;
    return @intFromPtr(&input_buf);
}

export fn embed(ptr: u32, len: u32) u32 {
    _ = ptr;
    _ = len;
    out_vec[0] = 0.25;
    out_vec[1] = 1.0;
    var i: usize = 2;
    while (i < 384) : (i += 1) {
        out_vec[i] = 0.0;
    }
    return @intFromPtr(&out_vec);
}
