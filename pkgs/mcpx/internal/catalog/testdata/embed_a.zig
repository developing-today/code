// embed_a.zig — synthetic deterministic embedding guest, model A (#13616).
//
// Built with: zig build-exe embed_a.zig -target wasm32-freestanding -O ReleaseSmall -rdynamic -fno-entry -femit-bin=../embed_a.wasm
// No imports: the module instantiates under plain wazero+WASI with nothing granted.
// It exports `allocate` and `embed` per the catalog.WasmEmbedder protocol:
// allocate(textLen) returns a writable scratch pointer, embed(ptr, len)
// writes a 384-dimensional f32LE vector and returns its pointer.
// Model A output: vec[0]=1.0, vec[1]=0.25, rest 0 — sparse on purpose, so a
// test can tell it apart from model B and from the dense subword fallback.

var input_buf: [4096]u8 align(8) = undefined;
var out_vec: [384]f32 align(8) = undefined;

export fn allocate(len: u32) u32 {
    _ = len;
    return @intFromPtr(&input_buf);
}

export fn embed(ptr: u32, len: u32) u32 {
    _ = ptr;
    _ = len;
    out_vec[0] = 1.0;
    out_vec[1] = 0.25;
    var i: usize = 2;
    while (i < 384) : (i += 1) {
        out_vec[i] = 0.0;
    }
    return @intFromPtr(&out_vec);
}
