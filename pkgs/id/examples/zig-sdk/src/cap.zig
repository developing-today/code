//! Helpers for the world capability protocol, for Zig guest
//! programs.
//!
//! A Zig port of the Roc `RocCap` module
//! (`pkgs/id/examples/roc-world/RocCap.roc`). A world program answers
//! the server with `wants(model)`: a JSON object with `subscribe` and
//! `requests`. Each request carries a program-chosen `id`; the server
//! answers with `{"cap":"result","id":...,"ok":true,"value":...}` or
//! `{"cap":"result","id":...,"ok":false,"error":...}`. Apps otherwise
//! re-derive the same bookkeeping — ids, pending lists, result
//! matching, argument documents — in every program. This module keeps
//! that in one place, on top of the SDK's `json` values.
//!
//! Everything here is pure bookkeeping; the app owns its model and
//! decides when to send.
//!
//! Memory is explicit, as everywhere in this SDK: a `Session` owns
//! its pending entries — the id and cap strings are copies, and a
//! document passed to `attach` or `fromPending` must be
//! allocator-owned (built by the request builders below or by
//! `json.decode`), because the session takes ownership of it and
//! frees it on `done`/`deinit`. The request builders return fully
//! owned documents, safe to free with `Value.deinit`. An arena
//! allocator makes all of this moot inside a guest, and is the
//! natural fit.

const std = @import("std");
const json = @import("json.zig");

pub const Value = json.Value;

/// One outstanding request: its program-chosen id, the capability it
/// asks for, and the document to (re-)announce in `wants`.
pub const Entry = struct {
    id: []const u8,
    cap: []const u8,
    document: Value,
};

pub const Session = struct {
    next: u64 = 0,
    pending: std.ArrayListUnmanaged(Entry) = .empty,

    /// A session with no outstanding requests.
    pub fn empty() Session {
        return .{};
    }

    pub fn deinit(self: *Session, alloc: std.mem.Allocator) void {
        for (self.pending.items) |entry| {
            alloc.free(entry.id);
            alloc.free(entry.cap);
            entry.document.deinit(alloc);
        }
        self.pending.deinit(alloc);
        self.pending = .empty;
    }

    /// Reserve the next request id under `cap_name`. Build the
    /// document with `chatRequest`/`nowRequest`/... and pass it to
    /// `attach` so `requests` can re-announce it after replay. The
    /// returned id is borrowed from the session and stays valid
    /// until the session changes.
    pub fn issue(
        self: *Session,
        alloc: std.mem.Allocator,
        cap_name: []const u8,
    ) ![]const u8 {
        const id = try std.fmt.allocPrint(alloc, "r{d}", .{self.next});
        errdefer alloc.free(id);
        const cap_copy = try alloc.dupe(u8, cap_name);
        errdefer alloc.free(cap_copy);
        try self.pending.append(alloc, .{
            .id = id,
            .cap = cap_copy,
            .document = .null,
        });
        self.next += 1;
        return id;
    }

    /// Store the request document under its id, taking ownership of
    /// it (any document it replaces is freed).
    pub fn attach(self: *Session, alloc: std.mem.Allocator, id: []const u8, document: Value) void {
        for (self.pending.items) |*entry| {
            if (std.mem.eql(u8, entry.id, id)) {
                entry.document.deinit(alloc);
                entry.document = document;
                return;
            }
        }
    }

    /// The outstanding request documents, in issue order, for
    /// `wants`. The caller frees the returned slice (not the
    /// documents, which the session still owns).
    pub fn requests(self: Session, alloc: std.mem.Allocator) ![]Value {
        const out = try alloc.alloc(Value, self.pending.items.len);
        for (self.pending.items, 0..) |entry, i| {
            out[i] = entry.document;
        }
        return out;
    }

    /// The request counter, for snapshotting.
    pub fn counter(self: Session) u64 {
        return self.next;
    }

    /// Rebuild a session from a counter and its pending documents,
    /// taking ownership of the documents (their `id`/`cap` fields
    /// become the entries; a missing or non-string field becomes
    /// `""`).
    pub fn fromPending(
        alloc: std.mem.Allocator,
        next: u64,
        documents: []const Value,
    ) !Session {
        var session = Session{ .next = next };
        errdefer session.deinit(alloc);
        for (documents) |document| {
            const id = try alloc.dupe(u8, stringField(document, "id"));
            errdefer alloc.free(id);
            const cap_copy = try alloc.dupe(u8, stringField(document, "cap"));
            errdefer alloc.free(cap_copy);
            try session.pending.append(alloc, .{
                .id = id,
                .cap = cap_copy,
                .document = document,
            });
        }
        return session;
    }

    /// The capability name a result `id` answers, or `""` when the
    /// id is not pending (an earlier model's retry, or somebody
    /// else's request). Use it to route the result exactly as the
    /// version that sent it would have.
    pub fn answered(self: Session, id: []const u8) []const u8 {
        for (self.pending.items) |entry| {
            if (std.mem.eql(u8, entry.id, id)) return entry.cap;
        }
        return "";
    }

    /// Retire the request a result answers, freeing its entry.
    pub fn done(self: *Session, alloc: std.mem.Allocator, id: []const u8) void {
        for (self.pending.items, 0..) |entry, i| {
            if (std.mem.eql(u8, entry.id, id)) {
                const removed = self.pending.orderedRemove(i);
                alloc.free(removed.id);
                alloc.free(removed.cap);
                removed.document.deinit(alloc);
                return;
            }
        }
    }

    /// Whether any request is still outstanding.
    pub fn isEmpty(self: Session) bool {
        return self.pending.items.len == 0;
    }
};

/// A string field of a JSON object, or `""` when missing or not a
/// string. The result borrows from the value.
pub fn stringField(value: Value, key: []const u8) []const u8 {
    const found = json.get(value, key) catch return "";
    return json.asString(found) catch "";
}

/// An array field of a JSON object, or `[]` when missing or not an
/// array. The result borrows from the value.
pub fn listField(value: Value, key: []const u8) []const Value {
    const found = json.get(value, key) catch return &.{};
    return json.asArray(found) catch &.{};
}

/// A `chat.say` request document.
pub fn chatRequest(alloc: std.mem.Allocator, id: []const u8, text: []const u8) !Value {
    const args = try object(alloc, &.{try stringFieldOf(alloc, "text", text)});
    return object(alloc, &.{
        try stringFieldOf(alloc, "id", id),
        try stringFieldOf(alloc, "cap", "chat.say"),
        .{ .key = try alloc.dupe(u8, "args"), .value = args },
    });
}

/// A `time.now` request document.
pub fn nowRequest(alloc: std.mem.Allocator, id: []const u8) !Value {
    return object(alloc, &.{
        try stringFieldOf(alloc, "id", id),
        try stringFieldOf(alloc, "cap", "time.now"),
    });
}

/// A `players.list` request document.
pub fn playersRequest(alloc: std.mem.Allocator, id: []const u8) !Value {
    return object(alloc, &.{
        try stringFieldOf(alloc, "id", id),
        try stringFieldOf(alloc, "cap", "players.list"),
    });
}

/// A `world.info` request document.
pub fn worldRequest(alloc: std.mem.Allocator, id: []const u8) !Value {
    return object(alloc, &.{
        try stringFieldOf(alloc, "id", id),
        try stringFieldOf(alloc, "cap", "world.info"),
    });
}

/// A `chat.history` request document. `count` is clamped by the
/// server to `MAX_CHAT_HISTORY_LINES` (64).
pub fn historyRequest(alloc: std.mem.Allocator, id: []const u8, count: u64) !Value {
    const args = try object(alloc, &.{try numberFieldOf(alloc, "count", count)});
    return object(alloc, &.{
        try stringFieldOf(alloc, "id", id),
        try stringFieldOf(alloc, "cap", "chat.history"),
        .{ .key = try alloc.dupe(u8, "args"), .value = args },
    });
}

/// The lines of a `chat.history` result value (`{"lines":[...]}`),
/// or `[]`.
pub fn historyLines(value: Value) []const Value {
    return listField(value, "lines");
}

/// A `records.read` request document. `after` is exclusive; `""`
/// starts at the first key. Pages are byte-bounded; follow
/// `nextAfter` while it is non-empty, passing it as the next `after`.
pub fn recordsRequest(
    alloc: std.mem.Allocator,
    id: []const u8,
    prefix: []const u8,
    after: []const u8,
) !Value {
    const args = try object(alloc, &.{
        try stringFieldOf(alloc, "prefix", prefix),
        try stringFieldOf(alloc, "after", after),
    });
    return object(alloc, &.{
        try stringFieldOf(alloc, "id", id),
        try stringFieldOf(alloc, "cap", "records.read"),
        .{ .key = try alloc.dupe(u8, "args"), .value = args },
    });
}

/// The `{"key":...,"value":...}` entries of a `records.read` result.
pub fn recordsPage(value: Value) []const Value {
    return listField(value, "records");
}

/// The cursor for the next `records.read`, or `""` when the page is
/// the last.
pub fn nextAfter(value: Value) []const u8 {
    return stringField(value, "next_after");
}

/// A `schedule.at` request document: ask for one `schedule_id` fire
/// at `at_ms` (unix milliseconds). A time in the past fires on the
/// next wakeup. The server refuses ids that are empty, longer than
/// 64 bytes, or not printable ASCII; a duplicate of a pending id;
/// and times more than 30 days ahead. The result is
/// `{"id":..,"fires_at":..}` or `bad_args`.
pub fn scheduleAtRequest(
    alloc: std.mem.Allocator,
    id: []const u8,
    schedule_id: []const u8,
    at_ms: u64,
) !Value {
    const args = try object(alloc, &.{
        try stringFieldOf(alloc, "id", schedule_id),
        try numberFieldOf(alloc, "at", at_ms),
    });
    return object(alloc, &.{
        try stringFieldOf(alloc, "id", id),
        try stringFieldOf(alloc, "cap", "schedule.at"),
        .{ .key = try alloc.dupe(u8, "args"), .value = args },
    });
}

/// A `schedule.cancel` request document. The result is
/// `{"cancelled":bool}`: `true` when a pending schedule with
/// `schedule_id` was dropped before it fired.
pub fn scheduleCancelRequest(
    alloc: std.mem.Allocator,
    id: []const u8,
    schedule_id: []const u8,
) !Value {
    const args = try object(alloc, &.{try stringFieldOf(alloc, "id", schedule_id)});
    return object(alloc, &.{
        try stringFieldOf(alloc, "id", id),
        try stringFieldOf(alloc, "cap", "schedule.cancel"),
        .{ .key = try alloc.dupe(u8, "args"), .value = args },
    });
}

fn object(alloc: std.mem.Allocator, fields: []const Value.Field) !Value {
    return .{ .object = try alloc.dupe(Value.Field, fields) };
}

fn stringFieldOf(alloc: std.mem.Allocator, key: []const u8, text: []const u8) !Value.Field {
    return .{
        .key = try alloc.dupe(u8, key),
        .value = .{ .string = try alloc.dupe(u8, text) },
    };
}

fn numberFieldOf(alloc: std.mem.Allocator, key: []const u8, n: u64) !Value.Field {
    return .{
        .key = try alloc.dupe(u8, key),
        .value = .{ .number = try std.fmt.allocPrint(alloc, "{d}", .{n}) },
    };
}

// ---------------------------------------------------------------------------
// Tests — behavioural mirrors of the RocCap.roc contract (the Roc
// module carries its expectations in its doc comments and callers).
// ---------------------------------------------------------------------------

const testing = std.testing;

fn expectEncodedAs(value: Value, expected: []const u8) !void {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const text = try json.encode(arena.allocator(), value);
    try testing.expectEqualStrings(expected, text);
}

test "cap: request documents encode like Roc" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    try expectEncodedAs(
        try chatRequest(alloc, "r0", "hi"),
        "{\"id\":\"r0\",\"cap\":\"chat.say\",\"args\":{\"text\":\"hi\"}}",
    );
    try expectEncodedAs(
        try nowRequest(alloc, "r1"),
        "{\"id\":\"r1\",\"cap\":\"time.now\"}",
    );
    try expectEncodedAs(
        try playersRequest(alloc, "r2"),
        "{\"id\":\"r2\",\"cap\":\"players.list\"}",
    );
    try expectEncodedAs(
        try worldRequest(alloc, "r3"),
        "{\"id\":\"r3\",\"cap\":\"world.info\"}",
    );
    try expectEncodedAs(
        try historyRequest(alloc, "r4", 64),
        "{\"id\":\"r4\",\"cap\":\"chat.history\",\"args\":{\"count\":64}}",
    );
    try expectEncodedAs(
        try recordsRequest(alloc, "r5", "chat/", ""),
        "{\"id\":\"r5\",\"cap\":\"records.read\",\"args\":{\"prefix\":\"chat/\",\"after\":\"\"}}",
    );
    try expectEncodedAs(
        try scheduleAtRequest(alloc, "r6", "wake", 1700000000000),
        "{\"id\":\"r6\",\"cap\":\"schedule.at\",\"args\":{\"id\":\"wake\",\"at\":1700000000000}}",
    );
    try expectEncodedAs(
        try scheduleCancelRequest(alloc, "r7", "wake"),
        "{\"id\":\"r7\",\"cap\":\"schedule.cancel\",\"args\":{\"id\":\"wake\"}}",
    );
}

test "cap: issue reserves sequential ids and tracks pending" {
    var session = Session.empty();
    try testing.expect(session.isEmpty());
    try testing.expectEqual(@as(u64, 0), session.counter());
    const alloc = testing.allocator;
    const first = try session.issue(alloc, "chat.say");
    try testing.expectEqualStrings("r0", first);
    const second = try session.issue(alloc, "time.now");
    try testing.expectEqualStrings("r1", second);
    try testing.expectEqual(@as(u64, 2), session.counter());
    try testing.expect(!session.isEmpty());
    try testing.expectEqualStrings("chat.say", session.answered("r0"));
    try testing.expectEqualStrings("time.now", session.answered("r1"));
    try testing.expectEqualStrings("", session.answered("r9"));
    session.deinit(alloc);
    try testing.expect(session.isEmpty());
}

test "cap: attach fills the document requests re-announces" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var session = Session.empty();
    const id = try session.issue(alloc, "chat.say");
    // Before attach, the pending document is null.
    const before = try session.requests(alloc);
    try testing.expectEqual(@as(usize, 1), before.len);
    try testing.expectEqualStrings("[null]", try json.encode(alloc, .{ .array = before }));
    session.attach(alloc, id, try chatRequest(alloc, id, "hello"));
    const after = try session.requests(alloc);
    try testing.expectEqualStrings(
        "[{\"id\":\"r0\",\"cap\":\"chat.say\",\"args\":{\"text\":\"hello\"}}]",
        try json.encode(alloc, .{ .array = after }),
    );
    // Attaching under an unknown id changes nothing.
    session.attach(alloc, "r9", try nowRequest(alloc, "r9"));
    try testing.expectEqual(@as(usize, 1), (try session.requests(alloc)).len);
}

test "cap: done retires the answered request" {
    var session = Session.empty();
    const alloc = testing.allocator;
    _ = try session.issue(alloc, "chat.say");
    _ = try session.issue(alloc, "time.now");
    session.done(alloc, "r0");
    try testing.expectEqualStrings("", session.answered("r0"));
    try testing.expectEqualStrings("time.now", session.answered("r1"));
    try testing.expect(!session.isEmpty());
    session.done(alloc, "r9"); // unknown id: no-op
    try testing.expectEqualStrings("time.now", session.answered("r1"));
    session.done(alloc, "r1");
    try testing.expect(session.isEmpty());
    session.deinit(alloc);
}

test "cap: from_pending rebuilds a session from documents" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const decoded = try json.decode(alloc, "[{\"id\":\"r3\",\"cap\":\"chat.say\",\"args\":{\"text\":\"hi\"}},{\"id\":\"r4\",\"cap\":\"time.now\"}]");
    const documents = try json.asArray(decoded);
    var session = try Session.fromPending(alloc, 7, documents);
    try testing.expectEqual(@as(u64, 7), session.counter());
    try testing.expectEqualStrings("chat.say", session.answered("r3"));
    try testing.expectEqualStrings("time.now", session.answered("r4"));
    // The documents round-trip through requests unchanged.
    const announced = try session.requests(alloc);
    try testing.expectEqualStrings(
        "[{\"id\":\"r3\",\"cap\":\"chat.say\",\"args\":{\"text\":\"hi\"}},{\"id\":\"r4\",\"cap\":\"time.now\"}]",
        try json.encode(alloc, .{ .array = announced }),
    );
    // And the counter continues where the snapshot left off.
    const next_id = try session.issue(alloc, "players.list");
    try testing.expectEqualStrings("r7", next_id);
}

test "cap: from_pending tolerates documents without id or cap" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const decoded = try json.decode(alloc, "[{\"cap\":7},{}]");
    var session = try Session.fromPending(alloc, 2, try json.asArray(decoded));
    try testing.expectEqual(@as(u64, 2), session.counter());
    try testing.expectEqualStrings("", session.answered(""));
    try testing.expectEqual(@as(usize, 2), (try session.requests(alloc)).len);
}

test "cap: result readers pick fields out of result values" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const history = try json.decode(alloc, "{\"lines\":[\"a\",\"b\"]}");
    try testing.expectEqual(@as(usize, 2), historyLines(history).len);
    const page = try json.decode(alloc, "{\"records\":[{\"key\":\"k\",\"value\":1}],\"next_after\":\"k2\"}");
    try testing.expectEqual(@as(usize, 1), recordsPage(page).len);
    try testing.expectEqualStrings("k2", nextAfter(page));
    // Missing or wrong-typed fields read as empty.
    const other = try json.decode(alloc, "{\"lines\":5}");
    try testing.expectEqual(@as(usize, 0), historyLines(other).len);
    try testing.expectEqualStrings("", nextAfter(other));
    try testing.expectEqualStrings("", stringField(other, "absent"));
    try testing.expectEqual(@as(usize, 0), listField(other, "absent").len);
}
