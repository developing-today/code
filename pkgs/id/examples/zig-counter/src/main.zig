//! Zig counter world — the playable example for the Zig guest SDK
//! (#12596), twin of `examples/roc-counter/main.roc` and
//! `examples/rust-counter` on the Plaza ABI v1 contract documented in
//! `../roc-world/ABI.md`:
//!
//! - `init` ignores the seed; the model is `{ count: i64 }` starting
//!   at 0.
//! - `update` increments on a trimmed `inc` event, like the Roc
//!   program (events are matched after a text trim, so `" inc "`
//!   counts too).
//! - `view` renders the boxed 20x3 screen through the SDK's screen
//!   helper; `layout` widens the box to 30 columns at width >= 30.
//! - `records` publishes `{"count":N}`, encoded with the SDK's json
//!   helper; `wants` is the empty document.
//! - `snapshot` is the decimal count; `restore` parses it back and
//!   is total (invalid text restores count 0), so
//!   `restore(snapshot(m))` behaves exactly like `m`.
//!
//! The pure program (`init`/`update`/`view`/`layout`/`records`/
//! `wants`/`snapshot`/`restore`) is host-testable; the `plaza_*`
//! exports below are the byte-level glue, mirroring the Rust SDK's
//! `plaza_guest!` plumbing: a leaked model pointer the host hands
//! back, one shared output slot released by the next output call,
//! and `plaza_alloc`/`plaza_free` over the guest allocator.

const std = @import("std");
const builtin = @import("builtin");
const sdk = @import("zig-sdk");
const screen = sdk.screen;
const json = sdk.json;

/// The guest allocator: the wasm heap inside a guest module, the
/// page allocator in host test runs (the program logic is exercised
/// natively there; the exports' pointer widths are a wasm32 affair).
const guest_allocator: std.mem.Allocator = if (builtin.target.cpu.arch == .wasm32)
    std.heap.wasm_allocator
else
    std.heap.page_allocator;

// ---- the world program --------------------------------------------------

pub const Model = struct {
    count: i64,
};

pub fn init(seed: u64) Model {
    _ = seed;
    return .{ .count = 0 };
}

fn isTrimByte(byte: u8) bool {
    return byte == ' ' or byte == '\t' or byte == '\n' or byte == '\r';
}

/// Byte trim matching `RocText.trim` / the Rust SDK's `text::trim`
/// (space, tab, LF, CR at both ends).
pub fn trim(bytes: []const u8) []const u8 {
    var start: usize = 0;
    var end: usize = bytes.len;
    while (start < end and isTrimByte(bytes[start])) start += 1;
    while (end > start and isTrimByte(bytes[end - 1])) end -= 1;
    return bytes[start..end];
}

pub fn update(model: Model, participant: u32, event: []const u8) Model {
    _ = participant;
    if (std.mem.eql(u8, trim(event), "inc")) {
        return .{ .count = model.count + 1 };
    }
    return model;
}

fn render(alloc: std.mem.Allocator, model: Model, width: u64, height: u64) ![]u8 {
    var scr = try screen.Screen.init(alloc, width, height);
    defer scr.deinit(alloc);
    scr.boxed(0, 0, width, height, "counter");
    var label_buf: [32]u8 = undefined;
    const label = try std.fmt.bufPrint(&label_buf, "count={d}", .{model.count});
    scr.putText(2, 1, label, .{});
    return scr.renderJson(alloc);
}

pub fn view(alloc: std.mem.Allocator, model: Model) ![]u8 {
    return render(alloc, model, 20, 3);
}

/// Reflow: a wide viewport gets a wider box, a short one keeps
/// three rows (the Roc program's layout, verbatim).
pub fn layout(alloc: std.mem.Allocator, model: Model, width: u64, height: u64) ![]u8 {
    _ = height;
    const w: u64 = if (width >= 30) 30 else 20;
    return render(alloc, model, w, 3);
}

/// Structured data the host stores and replicates: a JSON object of
/// records, built and encoded with the SDK's json helper.
pub fn records(alloc: std.mem.Allocator, model: Model) ![]u8 {
    var lexeme_buf: [24]u8 = undefined;
    const lexeme = try std.fmt.bufPrint(&lexeme_buf, "{d}", .{model.count});
    var fields = [_]json.Value.Field{
        .{ .key = "count", .value = .{ .number = lexeme } },
    };
    return json.encode(alloc, .{ .object = fields[0..] });
}

/// This program wants nothing from the server.
pub fn wants(alloc: std.mem.Allocator) ![]u8 {
    return alloc.dupe(u8, "{}");
}

pub fn snapshot(alloc: std.mem.Allocator, model: Model) ![]u8 {
    return std.fmt.allocPrint(alloc, "{d}", .{model.count});
}

pub fn restore(bytes: []const u8) Model {
    const count = std.fmt.parseInt(i64, trim(bytes), 10) catch 0;
    return .{ .count = count };
}

// ---- Plaza ABI v1 glue ----------------------------------------------------

/// The one shared output slot (ABI.md): the host reads and copies
/// the range immediately; the next output-producing call releases
/// the previous buffer.
var output: []u8 = &.{};

fn setOutput(bytes: []u8) i32 {
    if (output.len > 0) guest_allocator.free(output);
    output = bytes;
    return @intCast(@intFromPtr(output.ptr));
}

fn leakModel(model: Model) i32 {
    const ptr = guest_allocator.create(Model) catch @trap();
    ptr.* = model;
    return @intCast(@intFromPtr(ptr));
}

fn modelRef(model_ptr: i32) *Model {
    return @ptrFromInt(@as(usize, @intCast(model_ptr)));
}

fn inputBytes(ptr: i32, len: i32) []const u8 {
    const n: usize = @intCast(@max(len, 0));
    if (n == 0) return "";
    const p: [*]const u8 = @ptrFromInt(@as(usize, @intCast(ptr)));
    return p[0..n];
}

export fn plaza_abi_version() i32 {
    return 1;
}

export fn plaza_alloc(len: i32) i32 {
    const n: usize = @intCast(@max(len, 1));
    const slice = guest_allocator.alloc(u8, n) catch @trap();
    return @intCast(@intFromPtr(slice.ptr));
}

export fn plaza_free(ptr: i32, len: i32) void {
    if (ptr == 0) return;
    const n: usize = @intCast(@max(len, 1));
    const p: [*]u8 = @ptrFromInt(@as(usize, @intCast(ptr)));
    guest_allocator.free(p[0..n]);
}

export fn plaza_init(seed: i64) i32 {
    return leakModel(init(@bitCast(seed)));
}

export fn plaza_update(model_ptr: i32, participant: i32, event_ptr: i32, event_len: i32) i32 {
    const model = modelRef(model_ptr);
    const next = update(model.*, @bitCast(participant), inputBytes(event_ptr, event_len));
    guest_allocator.destroy(model);
    return leakModel(next);
}

export fn plaza_view(model_ptr: i32) i32 {
    return setOutput(view(guest_allocator, modelRef(model_ptr).*) catch @trap());
}

export fn plaza_layout(model_ptr: i32, width: i64, height: i64) i32 {
    return setOutput(layout(guest_allocator, modelRef(model_ptr).*, @bitCast(width), @bitCast(height)) catch @trap());
}

export fn plaza_records(model_ptr: i32) i32 {
    return setOutput(records(guest_allocator, modelRef(model_ptr).*) catch @trap());
}

export fn plaza_wants(model_ptr: i32) i32 {
    _ = model_ptr;
    return setOutput(wants(guest_allocator) catch @trap());
}

export fn plaza_snapshot(model_ptr: i32) i32 {
    return setOutput(snapshot(guest_allocator, modelRef(model_ptr).*) catch @trap());
}

export fn plaza_restore(snapshot_ptr: i32, snapshot_len: i32) i32 {
    return leakModel(restore(inputBytes(snapshot_ptr, snapshot_len)));
}

export fn plaza_out_len() i32 {
    return @intCast(output.len);
}

export fn plaza_error_ptr() i32 {
    // No crash has been captured: the diagnostic pair reads as an
    // empty message (length 0), which ABI.md permits at rest. Like
    // the Rust SDK, the pointer is the current output slot.
    return @intCast(@intFromPtr(output.ptr));
}

export fn plaza_error_len() i32 {
    return 0;
}

// ---- tests (mirrors of the rust-counter test suite) -----------------------

test "counter matches the roc program" {
    const alloc = std.testing.allocator;
    var model = init(7);
    try std.testing.expectEqual(@as(i64, 0), model.count);
    model = update(model, 1, "inc");
    try std.testing.expectEqual(@as(i64, 1), model.count);
    model = update(model, 1, "  inc \n");
    try std.testing.expectEqual(@as(i64, 2), model.count);
    model = update(model, 1, "dec");
    try std.testing.expectEqual(@as(i64, 2), model.count);

    const view_doc = try view(alloc, model);
    defer alloc.free(view_doc);
    try std.testing.expect(std.mem.indexOf(u8, view_doc, "count=2") != null);

    const records_doc = try records(alloc, model);
    defer alloc.free(records_doc);
    try std.testing.expectEqualStrings("{\"count\":2}", records_doc);

    const wants_doc = try wants(alloc);
    defer alloc.free(wants_doc);
    try std.testing.expectEqualStrings("{}", wants_doc);

    // Snapshot/restore round-trips byte-identically, and restore is
    // total on invalid text.
    const snap = try snapshot(alloc, model);
    defer alloc.free(snap);
    try std.testing.expectEqualStrings("2", snap);
    const restored = restore(snap);
    const snap_again = try snapshot(alloc, restored);
    defer alloc.free(snap_again);
    try std.testing.expectEqualStrings(snap, snap_again);
    try std.testing.expectEqual(@as(i64, 0), restore("junk").count);
    try std.testing.expectEqual(@as(i64, 0), restore("").count);
}

test "view is the boxed screen document" {
    const alloc = std.testing.allocator;
    const doc = try view(alloc, .{ .count = 1 });
    defer alloc.free(doc);
    try std.testing.expectEqualStrings(
        "{\"kind\":\"screen\",\"version\":1,\"width\":20,\"height\":3,\"rows\":[\"+-counter----------+\",\"| count=1          |\",\"+------------------+\"],\"styles\":[[],[],[]]}",
        doc,
    );
}

test "layout widens the box at width 30" {
    const alloc = std.testing.allocator;
    const wide = try layout(alloc, .{ .count = 1 }, 30, 3);
    defer alloc.free(wide);
    try std.testing.expect(std.mem.indexOf(u8, wide, "\"width\":30") != null);
    const narrow = try layout(alloc, .{ .count = 1 }, 29, 3);
    defer alloc.free(narrow);
    try std.testing.expect(std.mem.indexOf(u8, narrow, "\"width\":20") != null);
}
