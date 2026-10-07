//! Prototype wasm32 host: Elm-style init/update/view over Str boundaries.
const std = @import("std");
const abi = @import("roc_platform_abi.zig");

var env: abi.RocEnv = undefined;
var roc_host: abi.RocHost = undefined;
var ready: bool = false;

fn ensure() void {
    if (ready) return;
    env = .{ .allocator = std.heap.wasm_allocator, .roc_io = abi.RocIo.freestanding() };
    roc_host = abi.makeRocHost(&env);
    ready = true;
}

fn hostAlloc(length: usize, alignment: usize) callconv(.c) *anyopaque {
    ensure();
    return abi.DefaultAllocators.rocAlloc(&roc_host, length, alignment);
}
fn hostDealloc(ptr: *anyopaque, alignment: usize) callconv(.c) void {
    ensure();
    abi.DefaultAllocators.rocDealloc(&roc_host, ptr, alignment);
}
fn hostRealloc(ptr: *anyopaque, new_length: usize, alignment: usize) callconv(.c) *anyopaque {
    ensure();
    return abi.DefaultAllocators.rocRealloc(&roc_host, ptr, new_length, alignment);
}
fn hostDbg(_: [*]const u8, _: usize) callconv(.c) void {}
fn hostExpectFailed(_: [*]const u8, _: usize) callconv(.c) void {}
var last_crash: [4096]u8 = undefined;
var last_crash_len: usize = 0;

fn hostCrashed(bytes: [*]const u8, len: usize) callconv(.c) void {
    last_crash_len = @min(len, last_crash.len);
    @memcpy(last_crash[0..last_crash_len], bytes[0..last_crash_len]);
    @trap();
}

export fn plaza_error_ptr() usize {
    return @intFromPtr(&last_crash);
}

export fn plaza_error_len() usize {
    return last_crash_len;
}

comptime {
    @export(&hostAlloc, .{ .name = "roc_alloc", .visibility = .hidden });
    @export(&hostDealloc, .{ .name = "roc_dealloc", .visibility = .hidden });
    @export(&hostRealloc, .{ .name = "roc_realloc", .visibility = .hidden });
    @export(&hostDbg, .{ .name = "roc_dbg", .visibility = .hidden });
    @export(&hostExpectFailed, .{ .name = "roc_expect_failed", .visibility = .hidden });
    @export(&hostCrashed, .{ .name = "roc_crashed", .visibility = .hidden });
}

var last_out: abi.RocStr = abi.RocStr.empty();

export fn plaza_alloc(len: usize) usize {
    ensure();
    const slice = std.heap.wasm_allocator.alloc(u8, len) catch @trap();
    return @intFromPtr(slice.ptr);
}

export fn plaza_free(ptr: usize, len: usize) void {
    const p: [*]u8 = @ptrFromInt(ptr);
    std.heap.wasm_allocator.free(p[0..len]);
}

export fn plaza_init(seed: u64) usize {
    ensure();
    return @intFromPtr(abi.roc_init(seed));
}

export fn plaza_update(model: usize, participant: usize, ptr: usize, len: usize) usize {
    ensure();
    const p: [*]const u8 = @ptrFromInt(ptr);
    const s = abi.RocStr.fromSlice(p[0..len], &roc_host);
    return @intFromPtr(abi.roc_update(@ptrFromInt(model), @intCast(participant), s));
}

export fn plaza_view(model: usize, ptr: usize, len: usize) usize {
    ensure();
    last_out.decref(&roc_host);
    const p: [*]const u8 = @ptrFromInt(ptr);
    const s = abi.RocStr.fromSlice(p[0..len], &roc_host);
    abi.increfBox(@ptrFromInt(model), 1);
    last_out = abi.roc_view(@ptrFromInt(model), s);
    return @intFromPtr(last_out.asSlice().ptr);
}

/// Structured records projected from the model: a JSON object the host
/// stores and replicates. Shares the `plaza_out_len` output slot with view.
export fn plaza_records(model: usize) usize {
    ensure();
    last_out.decref(&roc_host);
    abi.increfBox(@ptrFromInt(model), 1);
    last_out = abi.roc_records(@ptrFromInt(model));
    return @intFromPtr(last_out.asSlice().ptr);
}

/// What the model wants from the server (subscriptions and requests).
/// Shares the `plaza_out_len` output slot with view and records.
export fn plaza_wants(model: usize) usize {
    ensure();
    last_out.decref(&roc_host);
    abi.increfBox(@ptrFromInt(model), 1);
    last_out = abi.roc_wants(@ptrFromInt(model));
    return @intFromPtr(last_out.asSlice().ptr);
}

/// A serialization of the model the host can journal. Shares the
/// `plaza_out_len` output slot with view and records.
export fn plaza_snapshot(model: usize) usize {
    ensure();
    last_out.decref(&roc_host);
    abi.increfBox(@ptrFromInt(model), 1);
    last_out = abi.roc_snapshot(@ptrFromInt(model));
    return @intFromPtr(last_out.asSlice().ptr);
}

/// Rebuild a model from a snapshot string produced by `plaza_snapshot`.
export fn plaza_restore(ptr: usize, len: usize) usize {
    ensure();
    const p: [*]const u8 = @ptrFromInt(ptr);
    const s = abi.RocStr.fromSlice(p[0..len], &roc_host);
    return @intFromPtr(abi.roc_restore(s));
}

export fn plaza_out_len() usize {
    return last_out.asSlice().len;
}
