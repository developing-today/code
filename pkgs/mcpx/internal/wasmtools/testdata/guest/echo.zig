// echo.zig — a well-behaved core-WASM tool source for the #1970 spike.
//
// Built with: zig build-exe echo.zig -target wasm32-freestanding -O ReleaseSmall -rdynamic -fno-entry -femit-bin=../echo.wasm
// No WASI, no ambient imports: the only import is the granted host
// capability `mcpx.log`. If the host does not grant it, this module
// cannot even be instantiated — that is the capability test's point.

extern "mcpx" fn log(ptr: [*]const u8, len: u32) void;

var input_buf: [8192]u8 align(8) = undefined;
var output_buf: [8192]u8 align(8) = undefined;

export fn mcpx_input_ptr() [*]u8 {
    return &input_buf;
}

export fn mcpx_output_ptr() [*]const u8 {
    return &output_buf;
}

fn writeOutput(s: []const u8) u32 {
    @memcpy(output_buf[0..s.len], s);
    return @intCast(s.len);
}

const tools_json = "[{\"name\":\"echo\",\"description\":\"echoes its arguments back\",\"inputSchema\":{\"type\":\"object\"}}]";

export fn mcpx_list_tools() u32 {
    return writeOutput(tools_json);
}

export fn mcpx_call_tool(name_len: u32, args_len: u32) u32 {
    const name = input_buf[0..name_len];
    const args = input_buf[name_len .. name_len + args_len];
    // Use the granted capability: log the call through the host.
    log(name.ptr, name_len);
    // Result JSON: {"tool":"<name>","arguments":<args raw>}
    var i: usize = 0;
    const pre = "{\"tool\":\"";
    @memcpy(output_buf[i .. i + pre.len], pre);
    i += pre.len;
    @memcpy(output_buf[i .. i + name.len], name);
    i += name.len;
    const mid = "\",\"arguments\":";
    @memcpy(output_buf[i .. i + mid.len], mid);
    i += mid.len;
    @memcpy(output_buf[i .. i + args.len], args);
    i += args.len;
    output_buf[i] = '}';
    i += 1;
    return @intCast(i);
}
