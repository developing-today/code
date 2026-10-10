//! Native worker for a Roc world program.
//!
//! The server runs this statically linked executable with the app linked in
//! (`roc build --target=x64musl`); it speaks the world program ABI over
//! stdin/stdout frames so the host can drive a native program exactly like a
//! Wasm one: `init`, `update`, `view`, `records`, `wants`, `snapshot`,
//! `restore`.
//!
//! Frame: big-endian `u32` length (excluding itself), then one opcode byte
//! and the payload. Reply: big-endian `u32` length, then `0` (failure) or `1`
//! followed by the payload. A crash exits with code 2; the parent treats any
//! exit, short read or oversize frame as a poisoned program.

const std = @import("std");
const abi = @import("roc_platform_abi.zig");

extern "c" fn read(fd: c_int, buf: [*]u8, n: usize) isize;
extern "c" fn write(fd: c_int, buf: [*]const u8, n: usize) isize;

/// Largest accepted frame payload (the host applies the same message cap).
const MAX_PAYLOAD: usize = 1024 * 1024;

const OP_INIT: u8 = 1;
const OP_UPDATE: u8 = 2;
const OP_VIEW: u8 = 3;
const OP_RECORDS: u8 = 4;
const OP_WANTS: u8 = 5;
const OP_SNAPSHOT: u8 = 6;
const OP_RESTORE: u8 = 7;

var env: abi.RocEnv = undefined;
var roc_host: abi.RocHost = undefined;
var last_out: abi.RocStr = abi.RocStr.empty();
var model: usize = 0;
/// The worker's own copy of the model snapshot. The app frees its model
/// arguments with its own refcount conventions (this host cannot safely
/// incref them), so the worker never keeps the model pointer between calls:
/// every call restores from this snapshot and every update saves a fresh
/// one.
var stored: []u8 = &.{};

fn hostAlloc(length: usize, alignment: usize) callconv(.c) *anyopaque {
    return abi.DefaultAllocators.rocAlloc(&roc_host, length, alignment);
}
fn hostDealloc(ptr: *anyopaque, alignment: usize) callconv(.c) void {
    abi.DefaultAllocators.rocDealloc(&roc_host, ptr, alignment);
}
fn hostRealloc(ptr: *anyopaque, new_length: usize, alignment: usize) callconv(.c) *anyopaque {
    return abi.DefaultAllocators.rocRealloc(&roc_host, ptr, new_length, alignment);
}
fn hostDbg(_: [*]const u8, _: usize) callconv(.c) void {}
fn hostExpectFailed(_: [*]const u8, _: usize) callconv(.c) void {}

fn save_model() void {
    const snapshot = abi.roc_snapshot(@ptrFromInt(model));
    const copy = std.heap.c_allocator.alloc(u8, snapshot.asSlice().len) catch {
        _ = write(2, "worker: out of memory for snapshot\n", 35);
        std.process.exit(2);
    };
    @memcpy(copy, snapshot.asSlice());
    if (stored.len != 0) std.heap.c_allocator.free(stored);
    stored = copy;
}

fn load_model() void {
    const s = abi.RocStr.fromSlice(stored, &roc_host);
    model = @intFromPtr(abi.roc_restore(s));
}

fn hostCrashed(_: [*]const u8, _: usize) callconv(.c) void {
    // Tell the parent the program died, then die. The parent treats any
    // exit as a poisoned program, like a Wasm trap.
    _ = write(2, "guest crashed\n", 14);
    std.process.exit(2);
}

comptime {
    @export(&hostAlloc, .{ .name = "roc_alloc", .visibility = .hidden });
    @export(&hostDealloc, .{ .name = "roc_dealloc", .visibility = .hidden });
    @export(&hostRealloc, .{ .name = "roc_realloc", .visibility = .hidden });
    @export(&hostDbg, .{ .name = "roc_dbg", .visibility = .hidden });
    @export(&hostExpectFailed, .{ .name = "roc_expect_failed", .visibility = .hidden });
    @export(&hostCrashed, .{ .name = "roc_crashed", .visibility = .hidden });
}

fn read_exact(fd: c_int, buffer: []u8) bool {
    var done: usize = 0;
    while (done < buffer.len) {
        const got = read(fd, buffer[done..].ptr, buffer.len - done);
        if (got <= 0) return false;
        done += @intCast(got);
    }
    return true;
}

fn write_all(fd: c_int, buffer: []const u8) bool {
    var done: usize = 0;
    while (done < buffer.len) {
        const put = write(fd, buffer[done..].ptr, buffer.len - done);
        if (put <= 0) return false;
        done += @intCast(put);
    }
    return true;
}

/// Reply `[u32 len][u8 ok][payload]`; `len` covers the status byte and the
/// payload.
fn reply(ok: bool, payload: []const u8) bool {
    var header: [5]u8 = undefined;
    const total: u32 = @intCast(1 + payload.len);
    std.mem.writeInt(u32, header[0..4], total, .big);
    header[4] = if (ok) 1 else 0;
    if (!write_all(1, &header)) return false;
    if (payload.len == 0) return true;
    return write_all(1, payload);
}

/// A projection through the shared output slot (view, records, wants,
/// snapshot).
fn project(value: abi.RocStr) bool {
    // The returned string is owned; leak it rather than release it with a
    // refcount convention the app does not share (bounded by call sizes).
    last_out = value;
    return reply(true, last_out.asSlice());
}

pub fn main() u8 {
    env = .{ .allocator = std.heap.c_allocator, .roc_io = abi.RocIo.native() };
    roc_host = abi.makeRocHost(&env);

    var length_buffer: [4]u8 = undefined;
    const payload = std.heap.c_allocator.alloc(u8, MAX_PAYLOAD) catch {
        return 1;
    };
    defer std.heap.c_allocator.free(payload);

    while (true) {
        if (!read_exact(0, &length_buffer)) return 0;
        const length = std.mem.readInt(u32, &length_buffer, .big);
        // `length` counts the opcode byte and the payload.
        if (length == 0 or length > MAX_PAYLOAD) {
            _ = reply(false, "");
            return 1;
        }
        if (!read_exact(0, payload[0..length])) return 0;
        const op = payload[0];
        const body = payload[1..length];
        switch (op) {
            OP_INIT => {
                if (body.len != 8) {
                    if (!reply(false, "")) return 1;
                    continue;
                }
                const seed = std.mem.readInt(u64, body[0..8], .big);
                model = @intFromPtr(abi.roc_init(seed));
                save_model();
                if (!reply(true, "")) return 1;
            },
            OP_UPDATE => {
                if (body.len < 4) {
                    if (!reply(false, "")) return 1;
                    continue;
                }
                const participant = std.mem.readInt(u32, body[0..4], .big);
                load_model();
                const s = abi.RocStr.fromSlice(body[4..], &roc_host);
                model = @intFromPtr(abi.roc_update(@ptrFromInt(model), participant, s));
                save_model();
                if (!reply(true, "")) return 1;
            },
            OP_VIEW => {
                load_model();
                const s = abi.RocStr.fromSlice(body, &roc_host);
                if (!project(abi.roc_view(@ptrFromInt(model), s))) return 1;
            },
            OP_RECORDS => {
                load_model();
                if (!project(abi.roc_records(@ptrFromInt(model)))) return 1;
            },
            OP_WANTS => {
                load_model();
                if (!project(abi.roc_wants(@ptrFromInt(model)))) return 1;
            },
            OP_SNAPSHOT => {
                // Answer from our own copy; no model round trip.
                if (!reply(true, stored)) return 1;
            },
            OP_RESTORE => {
                const copy = std.heap.c_allocator.alloc(u8, body.len) catch {
                    _ = reply(false, "");
                    return 1;
                };
                @memcpy(copy, body);
                if (stored.len != 0) std.heap.c_allocator.free(stored);
                stored = copy;
                load_model();
                if (!reply(true, "")) return 1;
            },
            else => {
                _ = reply(false, "");
                return 1;
            },
        }
    }
}
