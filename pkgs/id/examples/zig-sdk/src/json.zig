//! A small, dependency-free JSON value type for Zig guest programs.
//!
//! A Zig port of the Roc `RocJson` module
//! (`pkgs/id/examples/roc-world/RocJson.roc`), with the same wire
//! behaviour: numbers keep their exact lexeme (no implicit float loss),
//! decode/encode share the 64 KiB input/output limits and the 64-level
//! nesting limit, and errors carry the same codes as the Roc
//! `DecodeError` (see `errorCode`).
//!
//! Memory is explicit: `decode` allocates the value tree from the
//! caller-provided allocator (an arena is the natural fit inside a
//! wasm32-freestanding guest), and `Value.deinit` frees it. `encode`
//! allocates the output buffer from the caller-provided allocator.

const std = @import("std");

pub const max_input_bytes: usize = 65536;
pub const max_output_bytes: usize = 65536;
pub const max_nesting_depth: u32 = 64;

pub const DecodeError = error{
    InvalidJson,
    InputTooLarge,
    NestingTooDeep,
    OutputTooLarge,
    InvalidNumber,
    MissingField,
    WrongType,
};

/// The full error set of the fallible helpers below: a decode/encode
/// failure, or an allocation failure from the caller-provided
/// allocator. `errorCode` only covers the `DecodeError` members; an
/// allocation failure surfaces as `error.OutOfMemory` with no Roc code.
pub const Error = DecodeError || error{OutOfMemory};

/// The numeric code of an error, matching Roc `decode_error_code`:
/// InvalidJson 0, InputTooLarge 1, NestingTooDeep 2, OutputTooLarge 3,
/// InvalidNumber 4, MissingField 5, WrongType 6.
pub fn errorCode(err: DecodeError) u8 {
    return switch (err) {
        error.InvalidJson => 0,
        error.InputTooLarge => 1,
        error.NestingTooDeep => 2,
        error.OutputTooLarge => 3,
        error.InvalidNumber => 4,
        error.MissingField => 5,
        error.WrongType => 6,
    };
}

pub const Value = union(enum) {
    null,
    boolean: bool,
    /// The exact JSON number lexeme, e.g. `"-12.5e+2"`.
    number: []const u8,
    string: []const u8,
    array: []Value,
    object: []Field,

    pub const Field = struct {
        key: []const u8,
        value: Value,
    };

    /// Free a value tree allocated by `decode`. Values built by hand
    /// from borrowed slices must not be freed this way unless every
    /// slice came from `alloc`.
    pub fn deinit(self: Value, alloc: std.mem.Allocator) void {
        switch (self) {
            .array => |items| {
                for (items) |item| item.deinit(alloc);
                alloc.free(items);
            },
            .object => |fields| {
                for (fields) |field| {
                    alloc.free(field.key);
                    field.value.deinit(alloc);
                }
                alloc.free(fields);
            },
            .string => |text| alloc.free(text),
            .number => |lexeme| alloc.free(lexeme),
            .null, .boolean => {},
        }
    }

    /// Structural equality through encoding, mirroring Roc `same_json`.
    pub fn eql(self: Value, other: Value, alloc: std.mem.Allocator) bool {
        const left = encode(alloc, self) catch return false;
        defer alloc.free(left);
        const right = encode(alloc, other) catch return false;
        defer alloc.free(right);
        return std.mem.eql(u8, left, right);
    }
};

/// Parse `text` into a value. Fails with `InputTooLarge` above
/// `max_input_bytes`, `NestingTooDeep` past `max_nesting_depth`, and
/// `InvalidJson` for malformed input (including a trailing value after
/// the first, and an invalid number lexeme).
pub fn decode(alloc: std.mem.Allocator, text: []const u8) Error!Value {
    if (text.len > max_input_bytes) return error.InputTooLarge;
    var parser = Parser{ .bytes = text, .alloc = alloc };
    const value = try parser.parseValue(0);
    parser.skipSpace();
    if (parser.pos != text.len) {
        value.deinit(alloc);
        return error.InvalidJson;
    }
    return value;
}

/// Render a value as compact JSON. Fails with `NestingTooDeep` past
/// `max_nesting_depth`, `InvalidNumber` for a number lexeme that is not
/// valid JSON, and `OutputTooLarge` above `max_output_bytes`.
pub fn encode(alloc: std.mem.Allocator, value: Value) Error![]u8 {
    var out: std.ArrayListUnmanaged(u8) = .empty;
    errdefer out.deinit(alloc);
    try encodeValue(&out, alloc, value, 0);
    if (out.items.len > max_output_bytes) return error.OutputTooLarge;
    return out.toOwnedSlice(alloc);
}

/// The field named `key` of an object, or `MissingField` / `WrongType`.
pub fn get(value: Value, key: []const u8) Error!Value {
    switch (value) {
        .object => |fields| {
            for (fields) |field| {
                if (std.mem.eql(u8, field.key, key)) return field.value;
            }
            return error.MissingField;
        },
        else => return error.WrongType,
    }
}

pub fn asString(value: Value) Error![]const u8 {
    return switch (value) {
        .string => |text| text,
        else => error.WrongType,
    };
}

pub fn asNumber(value: Value) Error![]const u8 {
    return switch (value) {
        .number => |lexeme| lexeme,
        else => error.WrongType,
    };
}

pub fn asBool(value: Value) Error!bool {
    return switch (value) {
        .boolean => |b| b,
        else => error.WrongType,
    };
}

pub fn asArray(value: Value) Error![]Value {
    return switch (value) {
        .array => |items| items,
        else => error.WrongType,
    };
}

pub fn asObject(value: Value) Error![]Value.Field {
    return switch (value) {
        .object => |fields| fields,
        else => error.WrongType,
    };
}

const Parser = struct {
    bytes: []const u8,
    pos: usize = 0,
    alloc: std.mem.Allocator,

    fn byteAt(self: *Parser, at: usize) u8 {
        if (at >= self.bytes.len) return 0;
        return self.bytes[at];
    }

    fn skipSpace(self: *Parser) void {
        while (self.pos < self.bytes.len) {
            const c = self.bytes[self.pos];
            if (c == ' ' or c == '\t' or c == '\n' or c == '\r') {
                self.pos += 1;
            } else break;
        }
    }

    fn parseValue(self: *Parser, depth: u32) Error!Value {
        self.skipSpace();
        const c = self.byteAt(self.pos);
        switch (c) {
            'n' => {
                try self.expectLiteral("null");
                return .null;
            },
            't' => {
                try self.expectLiteral("true");
                return .{ .boolean = true };
            },
            'f' => {
                try self.expectLiteral("false");
                return .{ .boolean = false };
            },
            '"' => {
                const text = try self.parseString();
                return .{ .string = text };
            },
            '[' => {
                if (depth >= max_nesting_depth) return error.NestingTooDeep;
                return self.parseArray(depth + 1);
            },
            '{' => {
                if (depth >= max_nesting_depth) return error.NestingTooDeep;
                return self.parseObject(depth + 1);
            },
            '-', '0'...'9' => return self.parseNumber(),
            else => return error.InvalidJson,
        }
    }

    fn expectLiteral(self: *Parser, literal: []const u8) Error!void {
        if (self.pos + literal.len > self.bytes.len or
            !std.mem.eql(u8, self.bytes[self.pos .. self.pos + literal.len], literal))
        {
            return error.InvalidJson;
        }
        self.pos += literal.len;
    }

    fn parseArray(self: *Parser, depth: u32) Error!Value {
        // self.bytes[self.pos] == '['
        self.pos += 1;
        var items: std.ArrayListUnmanaged(Value) = .empty;
        errdefer {
            for (items.items) |item| item.deinit(self.alloc);
            items.deinit(self.alloc);
        }
        self.skipSpace();
        if (self.byteAt(self.pos) == ']') {
            self.pos += 1;
            return .{ .array = try items.toOwnedSlice(self.alloc) };
        }
        while (true) {
            const item = try self.parseValue(depth);
            errdefer item.deinit(self.alloc);
            try items.append(self.alloc, item);
            self.skipSpace();
            const next = self.byteAt(self.pos);
            if (next == ',') {
                self.pos += 1;
                continue;
            } else if (next == ']') {
                self.pos += 1;
                return .{ .array = try items.toOwnedSlice(self.alloc) };
            } else {
                return error.InvalidJson;
            }
        }
    }

    fn parseObject(self: *Parser, depth: u32) Error!Value {
        // self.bytes[self.pos] == '{'
        self.pos += 1;
        var fields: std.ArrayListUnmanaged(Value.Field) = .empty;
        errdefer {
            for (fields.items) |field| {
                self.alloc.free(field.key);
                field.value.deinit(self.alloc);
            }
            fields.deinit(self.alloc);
        }
        self.skipSpace();
        if (self.byteAt(self.pos) == '}') {
            self.pos += 1;
            return .{ .object = try fields.toOwnedSlice(self.alloc) };
        }
        if (self.byteAt(self.pos) != '"') return error.InvalidJson;
        while (true) {
            const key = try self.parseString();
            errdefer self.alloc.free(key);
            self.skipSpace();
            if (self.byteAt(self.pos) != ':') return error.InvalidJson;
            self.pos += 1;
            const value = try self.parseValue(depth);
            errdefer value.deinit(self.alloc);
            try fields.append(self.alloc, .{ .key = key, .value = value });
            self.skipSpace();
            const next = self.byteAt(self.pos);
            if (next == ',') {
                self.pos += 1;
                self.skipSpace();
                if (self.byteAt(self.pos) != '"') return error.InvalidJson;
                continue;
            } else if (next == '}') {
                self.pos += 1;
                return .{ .object = try fields.toOwnedSlice(self.alloc) };
            } else {
                return error.InvalidJson;
            }
        }
    }

    fn parseString(self: *Parser) Error![]u8 {
        // self.bytes[self.pos] == '"'; consume it and read to the closing quote.
        self.pos += 1;
        var out: std.ArrayListUnmanaged(u8) = .empty;
        errdefer out.deinit(self.alloc);
        while (true) {
            if (self.pos >= self.bytes.len) return error.InvalidJson;
            const c = self.bytes[self.pos];
            if (c == '"') {
                self.pos += 1;
                const text = try out.toOwnedSlice(self.alloc);
                if (!std.unicode.utf8ValidateSlice(text)) {
                    self.alloc.free(text);
                    return error.InvalidJson;
                }
                return text;
            } else if (c == '\\') {
                self.pos += 1;
                if (self.pos >= self.bytes.len) return error.InvalidJson;
                const esc = self.bytes[self.pos];
                switch (esc) {
                    '"', '\\', '/' => {
                        try out.append(self.alloc, esc);
                        self.pos += 1;
                    },
                    'b' => {
                        try out.append(self.alloc, 8);
                        self.pos += 1;
                    },
                    'f' => {
                        try out.append(self.alloc, 12);
                        self.pos += 1;
                    },
                    'n' => {
                        try out.append(self.alloc, 10);
                        self.pos += 1;
                    },
                    'r' => {
                        try out.append(self.alloc, 13);
                        self.pos += 1;
                    },
                    't' => {
                        try out.append(self.alloc, 9);
                        self.pos += 1;
                    },
                    'u' => {
                        self.pos += 1;
                        const cp = try self.parseUnicode();
                        var buf: [4]u8 = undefined;
                        const len = std.unicode.utf8Encode(cp, &buf) catch
                            return error.InvalidJson;
                        try out.appendSlice(self.alloc, buf[0..len]);
                    },
                    else => return error.InvalidJson,
                }
            } else if (c < 0x20) {
                return error.InvalidJson;
            } else {
                try out.append(self.alloc, c);
                self.pos += 1;
            }
        }
    }

    fn parseUnicode(self: *Parser) Error!u21 {
        const first = try self.readHex4();
        if (first >= 0xD800 and first <= 0xDBFF) {
            // High surrogate: a `\uDC00`–`\uDFFF` low surrogate must follow.
            if (self.byteAt(self.pos) == '\\' and self.byteAt(self.pos + 1) == 'u') {
                self.pos += 2;
                const second = try self.readHex4();
                if (second >= 0xDC00 and second <= 0xDFFF) {
                    return @intCast(0x10000 + (first - 0xD800) * 0x400 + (second - 0xDC00));
                }
            }
            return error.InvalidJson;
        } else if (first >= 0xDC00 and first <= 0xDFFF) {
            return error.InvalidJson;
        }
        return @intCast(first);
    }

    fn readHex4(self: *Parser) Error!u32 {
        var value: u32 = 0;
        for (0..4) |_| {
            const digit = hexValue(self.byteAt(self.pos)) orelse
                return error.InvalidJson;
            value = value * 16 + digit;
            self.pos += 1;
        }
        return value;
    }

    fn parseNumber(self: *Parser) Error!Value {
        const start = self.pos;
        while (self.pos < self.bytes.len) {
            const c = self.bytes[self.pos];
            if ((c >= '0' and c <= '9') or c == '-' or c == '+' or c == '.' or
                c == 'e' or c == 'E')
            {
                self.pos += 1;
            } else break;
        }
        const lexeme = self.bytes[start..self.pos];
        if (lexeme.len == 0 or !validNumber(lexeme)) return error.InvalidJson;
        const owned = try self.alloc.dupe(u8, lexeme);
        errdefer self.alloc.free(owned);
        return .{ .number = owned };
    }
};

fn hexValue(c: u8) ?u32 {
    if (c >= '0' and c <= '9') return c - '0';
    if (c >= 'A' and c <= 'F') return c - 'A' + 10;
    if (c >= 'a' and c <= 'f') return c - 'a' + 10;
    return null;
}

/// Strict JSON number grammar: `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`.
/// Mirrors Roc `valid_number`.
pub fn validNumber(lexeme: []const u8) bool {
    var i: usize = 0;
    if (i < lexeme.len and lexeme[i] == '-') i += 1;
    if (i >= lexeme.len) return false;
    if (lexeme[i] == '0') {
        i += 1;
    } else if (lexeme[i] >= '1' and lexeme[i] <= '9') {
        while (i < lexeme.len and lexeme[i] >= '0' and lexeme[i] <= '9') i += 1;
    } else {
        return false;
    }
    if (i < lexeme.len and lexeme[i] == '.') {
        i += 1;
        if (i >= lexeme.len or lexeme[i] < '0' or lexeme[i] > '9') return false;
        while (i < lexeme.len and lexeme[i] >= '0' and lexeme[i] <= '9') i += 1;
    }
    if (i < lexeme.len and (lexeme[i] == 'e' or lexeme[i] == 'E')) {
        i += 1;
        if (i < lexeme.len and (lexeme[i] == '+' or lexeme[i] == '-')) i += 1;
        if (i >= lexeme.len or lexeme[i] < '0' or lexeme[i] > '9') return false;
        while (i < lexeme.len and lexeme[i] >= '0' and lexeme[i] <= '9') i += 1;
    }
    return i == lexeme.len;
}

fn encodeValue(
    out: *std.ArrayListUnmanaged(u8),
    alloc: std.mem.Allocator,
    value: Value,
    depth: u32,
) Error!void {
    switch (value) {
        .null => try out.appendSlice(alloc, "null"),
        .boolean => |b| try out.appendSlice(alloc, if (b) "true" else "false"),
        .number => |lexeme| {
            if (!validNumber(lexeme)) return error.InvalidNumber;
            try out.appendSlice(alloc, lexeme);
        },
        .string => |text| try encodeQuoted(out, alloc, text),
        .array => |items| {
            if (depth >= max_nesting_depth) return error.NestingTooDeep;
            try out.append(alloc, '[');
            for (items, 0..) |item, i| {
                if (i > 0) try out.append(alloc, ',');
                try encodeValue(out, alloc, item, depth + 1);
            }
            try out.append(alloc, ']');
        },
        .object => |fields| {
            if (depth >= max_nesting_depth) return error.NestingTooDeep;
            try out.append(alloc, '{');
            for (fields, 0..) |field, i| {
                if (i > 0) try out.append(alloc, ',');
                try encodeQuoted(out, alloc, field.key);
                try out.append(alloc, ':');
                try encodeValue(out, alloc, field.value, depth + 1);
            }
            try out.append(alloc, '}');
        },
    }
}

fn encodeQuoted(
    out: *std.ArrayListUnmanaged(u8),
    alloc: std.mem.Allocator,
    text: []const u8,
) Error!void {
    try out.append(alloc, '"');
    for (text) |byte| {
        switch (byte) {
            '"' => try out.appendSlice(alloc, "\\\""),
            '\\' => try out.appendSlice(alloc, "\\\\"),
            8 => try out.appendSlice(alloc, "\\b"),
            9 => try out.appendSlice(alloc, "\\t"),
            10 => try out.appendSlice(alloc, "\\n"),
            12 => try out.appendSlice(alloc, "\\f"),
            13 => try out.appendSlice(alloc, "\\r"),
            0x00...0x07, 0x0b, 0x0e...0x1f => {
                const hex = "0123456789abcdef";
                var esc = [_]u8{ '\\', 'u', '0', '0', 0, 0 };
                esc[4] = hex[byte >> 4];
                esc[5] = hex[byte & 0x0f];
                try out.appendSlice(alloc, &esc);
            },
            else => try out.append(alloc, byte),
        }
    }
    try out.append(alloc, '"');
}

// ---------------------------------------------------------------------------
// Tests — mirrors of the Roc `expect` cases in RocJson.roc.
// ---------------------------------------------------------------------------

const testing = std.testing;

fn expectEncodedAs(value: Value, expected: []const u8) !void {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const text = try encode(arena.allocator(), value);
    try testing.expectEqualStrings(expected, text);
}

fn expectDecodedAs(text: []const u8, expected: Value) !void {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const value = try decode(arena.allocator(), text);
    try testing.expect(value.eql(expected, arena.allocator()));
}

fn expectDecodeError(text: []const u8, wanted: DecodeError) !void {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const result = decode(arena.allocator(), text);
    try testing.expectError(wanted, result);
}

fn expectEncodeError(value: Value, wanted: DecodeError) !void {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const result = encode(arena.allocator(), value);
    try testing.expectError(wanted, result);
}

fn nestedValue(alloc: std.mem.Allocator, depth: u32, inner: Value) !Value {
    if (depth == 0) return inner;
    const one = try alloc.alloc(Value, 1);
    one[0] = try nestedValue(alloc, depth - 1, inner);
    return .{ .array = one };
}

fn nestedObjectValue(alloc: std.mem.Allocator, depth: u32, inner: Value) !Value {
    if (depth == 0) return inner;
    const one = try alloc.alloc(Value.Field, 1);
    const key = try alloc.dupe(u8, "a");
    one[0] = .{ .key = key, .value = try nestedObjectValue(alloc, depth - 1, inner) };
    return .{ .object = one };
}

fn depthText(alloc: std.mem.Allocator, depth: u32) ![]u8 {
    var out: std.ArrayListUnmanaged(u8) = .empty;
    for (0..depth) |_| try out.append(alloc, '[');
    try out.append(alloc, '0');
    for (0..depth) |_| try out.append(alloc, ']');
    return out.toOwnedSlice(alloc);
}

fn objectDepthText(alloc: std.mem.Allocator, depth: u32) ![]u8 {
    var out: std.ArrayListUnmanaged(u8) = .empty;
    for (0..depth) |_| try out.appendSlice(alloc, "{\"a\":");
    try out.append(alloc, '0');
    for (0..depth) |_| try out.append(alloc, '}');
    return out.toOwnedSlice(alloc);
}

test "json: encode escapes strings like Roc" {
    try expectEncodedAs(.{ .string = "a\"\\\n\x01" }, "\"a\\\"\\\\\\n\\u0001\"");
    try expectEncodedAs(.{ .number = "-0.25E+3" }, "-0.25E+3");
}

test "json: decode nested document" {
    try expectDecodedAs(
        "{\"outer\":[null,true,{\"n\":-12.5e+2,\"s\":\"quote: \\\" slash: \\\\ tab:\\t line:\\n\"}]}",
        .{ .object = @constCast(&[_]Value.Field{
            .{ .key = "outer", .value = .{ .array = @constCast(&[_]Value{
                .null,
                .{ .boolean = true },
                .{ .object = @constCast(&[_]Value.Field{
                    .{ .key = "n", .value = .{ .number = "-12.5e+2" } },
                    .{ .key = "s", .value = .{ .string = "quote: \" slash: \\ tab:\t line:\n" } },
                }) },
            }) } },
        }) },
    );
}

test "json: decode surrogate pair" {
    try expectDecodedAs("\"\\uD83D\\uDE03\"", .{ .string = "😃" });
}

test "json: malformed input is InvalidJson" {
    try expectDecodeError("[1,", error.InvalidJson);
    try expectDecodeError("{\"x\":\"unfinished}", error.InvalidJson);
    try expectDecodeError("true false", error.InvalidJson);
    try expectDecodeError("01", error.InvalidJson);
    try expectDecodeError("1e+", error.InvalidJson);
}

test "json: field access and type checks" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const obj = try decode(alloc, "{\"items\":[\"ok\"]}");
    const items = try get(obj, "items");
    try testing.expectEqual(@as(usize, 1), (try asArray(items)).len);
    try testing.expectError(error.MissingField, get(try decode(alloc, "{}"), "absent"));
    try testing.expectEqualStrings("ok", try asString(.{ .string = "ok" }));
    try testing.expectError(error.WrongType, asString(.{ .number = "1" }));
    try testing.expectError(error.WrongType, asBool(.{ .string = "no" }));
}

test "json: round trip" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const sample = Value{ .array = @constCast(&[_]Value{
        .null,
        .{ .boolean = false },
        .{ .number = "0.125" },
        .{ .string = "x\x08\t\\\"" },
        .{ .object = @constCast(&[_]Value.Field{
            .{ .key = "k", .value = .{ .array = &.{} } },
        }) },
    }) };
    const text = try encode(alloc, sample);
    const back = try decode(alloc, text);
    try testing.expect(back.eql(sample, alloc));
}

test "json: invalid number lexeme is InvalidNumber on encode" {
    try expectEncodeError(.{ .number = "01" }, error.InvalidNumber);
    try expectEncodeError(
        .{ .object = @constCast(&[_]Value.Field{
            .{ .key = "n", .value = .{ .number = "1e+" } },
        }) },
        error.InvalidNumber,
    );
}

test "json: nesting depth 64 decodes, 65 is NestingTooDeep" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const depth64 = try depthText(alloc, 64);
    const value64 = try decode(alloc, depth64);
    try testing.expect(value64.eql(try nestedValue(alloc, 64, .{ .number = "0" }), alloc));
    try expectDecodeError(try depthText(alloc, 65), error.NestingTooDeep);
    const obj64 = try objectDepthText(alloc, 64);
    const obj_value64 = try decode(alloc, obj64);
    try testing.expect(obj_value64.eql(try nestedObjectValue(alloc, 64, .{ .number = "0" }), alloc));
    try expectDecodeError(try objectDepthText(alloc, 65), error.NestingTooDeep);
    try expectEncodeError(try nestedValue(alloc, 65, .null), error.NestingTooDeep);
    try expectEncodeError(try nestedObjectValue(alloc, 65, .null), error.NestingTooDeep);
}

test "json: input and output size limits" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    // "null" + 65532 spaces is exactly max_input_bytes and decodes.
    var padded_ok: std.ArrayListUnmanaged(u8) = .empty;
    try padded_ok.appendSlice(alloc, "null");
    for (0..65532) |_| try padded_ok.append(alloc, ' ');
    const ok_value = try decode(alloc, padded_ok.items);
    try testing.expect(ok_value == .null);
    // One more byte is InputTooLarge.
    var padded_big: std.ArrayListUnmanaged(u8) = .empty;
    try padded_big.appendSlice(alloc, "null");
    for (0..65533) |_| try padded_big.append(alloc, ' ');
    try expectDecodeError(padded_big.items, error.InputTooLarge);
    // A 65535-byte string encodes to 65537 bytes: OutputTooLarge.
    const big = try alloc.alloc(u8, 65535);
    @memset(big, 'A');
    try expectEncodeError(.{ .string = big }, error.OutputTooLarge);
}

test "json: error codes match Roc decode_error_code" {
    try testing.expectEqual(@as(u8, 0), errorCode(error.InvalidJson));
    try testing.expectEqual(@as(u8, 1), errorCode(error.InputTooLarge));
    try testing.expectEqual(@as(u8, 2), errorCode(error.NestingTooDeep));
    try testing.expectEqual(@as(u8, 3), errorCode(error.OutputTooLarge));
    try testing.expectEqual(@as(u8, 4), errorCode(error.InvalidNumber));
    try testing.expectEqual(@as(u8, 5), errorCode(error.MissingField));
    try testing.expectEqual(@as(u8, 6), errorCode(error.WrongType));
}
