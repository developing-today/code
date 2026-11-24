//! A cell-grid screen for Zig guest programs: text, boxes, and grids
//! drawn into a bounded buffer, rendered as plain text, as the
//! versioned JSON screen document, or as ANSI terminal output.
//!
//! A Zig port of the Roc `Screen` module
//! (`pkgs/id/examples/roc-world/Screen.roc`), with the same wire
//! behaviour: the JSON document is byte-for-byte the Roc one (run-length
//! style spans, default-styled runs omitted, rows padded to width), and
//! a column is one Unicode scalar value everywhere.
//!
//! Two deliberate differences from Roc, both following the `grid`
//! port's precedent:
//!
//! - **Memory is explicit.** Renders allocate from the
//!   caller-provided allocator (an arena is the natural fit inside a
//!   wasm32-freestanding guest) and the caller frees the result.
//! - **Drawing mutates in place** (`*Screen`) instead of returning a
//!   new screen per call, which avoids a copy of the cell buffer per
//!   draw inside a guest. Renders take the screen by value and never
//!   mutate it.
//!
//! A cell's `ch` is a borrowed slice — usually a string literal or a
//! slice of the text passed to `putText` — so a screen does not own
//! the text it shows; keep the source text alive while the screen is.

const std = @import("std");
const grid = @import("grid.zig");

/// One cell's style: 256-colour foreground/background indexes
/// (0 = default) and a bold flag.
pub const Style = struct {
    fg: u8 = 0,
    bg: u8 = 0,
    bold: bool = false,

    pub fn eql(self: Style, other: Style) bool {
        return self.fg == other.fg and self.bg == other.bg and self.bold == other.bold;
    }

    /// The default style (all zero) writes no span in the JSON
    /// document and no SGR codes in ANSI output.
    pub fn isDefault(self: Style) bool {
        return self.fg == 0 and self.bg == 0 and !self.bold;
    }
};

/// One screen cell: a single Unicode scalar value with its style.
pub const Cell = struct {
    ch: []const u8,
    fg: u8 = 0,
    bg: u8 = 0,
    bold: bool = false,

    pub fn style(self: Cell) Style {
        return .{ .fg = self.fg, .bg = self.bg, .bold = self.bold };
    }

    pub fn styled(ch: []const u8, s: Style) Cell {
        return .{ .ch = ch, .fg = s.fg, .bg = s.bg, .bold = s.bold };
    }
};

pub const InitError = error{
    OutOfMemory,
    InvalidDimensions,
};

pub const Screen = struct {
    width: u64,
    height: u64,
    cells: []Cell,

    /// A blank screen of `width` by `height` cells, every cell a
    /// space in the default style.
    pub fn init(alloc: std.mem.Allocator, width: u64, height: u64) InitError!Screen {
        const cell_count = std.math.mul(u64, width, height) catch
            return error.InvalidDimensions;
        const cells = try alloc.alloc(Cell, @intCast(cell_count));
        @memset(cells, .{ .ch = " " });
        return .{ .width = width, .height = height, .cells = cells };
    }

    pub fn deinit(self: Screen, alloc: std.mem.Allocator) void {
        alloc.free(self.cells);
    }

    /// Write `text` starting at column `x` of row `y`, clipped to the
    /// screen. One column per Unicode scalar value.
    pub fn putText(self: *Screen, x: u64, y: u64, text: []const u8, s: Style) void {
        var column = x;
        var i: usize = 0;
        while (i < text.len) {
            const ch = charAt(text, i);
            i += charLenAt(text, i);
            if (column < self.width and y < self.height) {
                self.cells[@intCast(y * self.width + column)] = Cell.styled(ch, s);
            }
            column += 1;
        }
    }

    /// Replace one row's cells from `x` with `cells`, each carrying
    /// its own style, clipped to the screen.
    pub fn putRow(self: *Screen, y: u64, x: u64, row_cells: []const Cell) void {
        if (y >= self.height) return;
        for (row_cells, 0..) |cell, offset| {
            const column = x + @as(u64, @intCast(offset));
            if (column < self.width) {
                self.cells[@intCast(y * self.width + column)] = cell;
            }
        }
    }

    /// Draw every cell of `g` at (`x`, `y`) through `glyph`, with one
    /// style for the whole grid. Cells outside the screen are clipped.
    pub fn putGrid(
        self: *Screen,
        alloc: std.mem.Allocator,
        x: u64,
        y: u64,
        g: grid.Grid,
        glyph: *const fn (u8) []const u8,
        s: Style,
    ) !void {
        const dims = g.dimensions();
        const row_cells = try alloc.alloc(Cell, @intCast(dims.width));
        defer alloc.free(row_cells);
        for (0..@intCast(dims.height)) |gy| {
            for (0..@intCast(dims.width)) |gx| {
                const value = g.get(@intCast(gx), @intCast(gy), 32);
                row_cells[gx] = Cell.styled(glyph(value), s);
            }
            self.putRow(y + @as(u64, @intCast(gy)), x, row_cells);
        }
    }

    /// Fill the rectangle at (`x`, `y`) of size `w` by `h` with `ch`
    /// and one style.
    pub fn fill(self: *Screen, x: u64, y: u64, w: u64, h: u64, ch: []const u8, s: Style) void {
        for (0..@intCast(h)) |row| {
            for (0..@intCast(w)) |column| {
                self.putText(x + @as(u64, @intCast(column)), y + @as(u64, @intCast(row)), ch, s);
            }
        }
    }

    /// A horizontal line of `ch` from (`x`, `y`), `w` cells wide.
    pub fn rule(self: *Screen, x: u64, y: u64, w: u64, ch: []const u8, s: Style) void {
        self.fill(x, y, w, 1, ch, s);
    }

    /// Draw an ASCII border around the rectangle at (`x`, `y`) of
    /// size `w` by `h`, with `title` written into the top edge. The
    /// border cells borrow only static literals, never a temporary.
    pub fn boxed(
        self: *Screen,
        x: u64,
        y: u64,
        w: u64,
        h: u64,
        title: []const u8,
    ) void {
        if (w < 2 or h < 2) return;
        const plain = Style{};
        for (0..@intCast(w)) |i| {
            const column = @as(u64, @intCast(i));
            const ch: []const u8 = if (i == 0 or i == w - 1) "+" else "-";
            self.putText(x + column, y, ch, plain);
            self.putText(x + column, y + h - 1, ch, plain);
        }
        self.putText(x + 2, y, title, plain);
        var row: u64 = y + 1;
        while (row + 1 < y + h) : (row += 1) {
            self.putText(x, row, "|", plain);
            self.putText(x + w - 1, row, "|", plain);
        }
    }

    /// The screen as plain text: rows joined by newlines, styles
    /// dropped.
    pub fn renderPlain(self: Screen, alloc: std.mem.Allocator) ![]u8 {
        var out: std.ArrayListUnmanaged(u8) = .empty;
        errdefer out.deinit(alloc);
        for (0..@intCast(self.height)) |row| {
            if (row > 0) try out.append(alloc, '\n');
            for (self.rowCells(@intCast(row))) |cell| {
                try out.appendSlice(alloc, cell.ch);
            }
        }
        return out.toOwnedSlice(alloc);
    }

    /// A portable, versioned screen document for SSH and browser
    /// presenters. The host validates the document and derives a
    /// bounded cell grid. Plain text views remain supported for
    /// programs that do not use this function.
    pub fn renderJson(self: Screen, alloc: std.mem.Allocator) ![]u8 {
        var out: std.ArrayListUnmanaged(u8) = .empty;
        errdefer out.deinit(alloc);
        try appendDocumentHead(&out, alloc, self.width, self.height);
        for (0..@intCast(self.height)) |row| {
            if (row > 0) try out.append(alloc, ',');
            const text = try self.rowText(alloc, @intCast(row));
            defer alloc.free(text);
            try appendJsonQuote(&out, alloc, text);
        }
        try out.appendSlice(alloc, "],\"styles\":[");
        for (0..@intCast(self.height)) |row| {
            if (row > 0) try out.append(alloc, ',');
            try self.appendRowStylesJson(&out, alloc, @intCast(row));
        }
        try out.appendSlice(alloc, "]}");
        return out.toOwnedSlice(alloc);
    }

    /// The screen as ANSI terminal output: home the cursor, then each
    /// row with SGR codes wherever the style changes. Rows end with
    /// CRLF for raw terminals.
    pub fn renderAnsi(self: Screen, alloc: std.mem.Allocator) ![]u8 {
        var out: std.ArrayListUnmanaged(u8) = .empty;
        errdefer out.deinit(alloc);
        try out.appendSlice(alloc, "\x1b[H");
        for (0..@intCast(self.height)) |row| {
            if (row > 0) try out.appendSlice(alloc, "\r\n");
            var current = Style{};
            for (self.rowCells(@intCast(row))) |cell| {
                const s = cell.style();
                if (!s.eql(current)) {
                    try out.appendSlice(alloc, "\x1b[0m");
                    try appendSgr(&out, alloc, s);
                    current = s;
                }
                try out.appendSlice(alloc, cell.ch);
            }
            try out.appendSlice(alloc, "\x1b[0m");
        }
        return out.toOwnedSlice(alloc);
    }

    fn rowCells(self: Screen, row: u64) []const Cell {
        const start: usize = @intCast(row * self.width);
        return self.cells[start .. start + @as(usize, @intCast(self.width))];
    }

    fn rowText(self: Screen, alloc: std.mem.Allocator, row: u64) ![]u8 {
        var out: std.ArrayListUnmanaged(u8) = .empty;
        errdefer out.deinit(alloc);
        for (self.rowCells(row)) |cell| {
            try out.appendSlice(alloc, cell.ch);
        }
        return out.toOwnedSlice(alloc);
    }

    fn appendRowStylesJson(
        self: Screen,
        out: *std.ArrayListUnmanaged(u8),
        alloc: std.mem.Allocator,
        row: u64,
    ) !void {
        try out.append(alloc, '[');
        var first = true;
        var run_start: u64 = 0;
        var run_len: u64 = 0;
        var run_style = Style{};
        for (self.rowCells(row), 0..) |cell, column| {
            const s = cell.style();
            if (run_len == 0) {
                run_start = @intCast(column);
                run_len = 1;
                run_style = s;
            } else if (s.eql(run_style)) {
                run_len += 1;
            } else {
                try appendSpan(out, alloc, &first, run_start, run_len, run_style);
                run_start = @intCast(column);
                run_len = 1;
                run_style = s;
            }
        }
        try appendSpan(out, alloc, &first, run_start, run_len, run_style);
        try out.append(alloc, ']');
    }
};

/// One piece of a `renderRowsJson` row: text laid out with one style.
pub const Piece = struct {
    text: []const u8,
    style: Style = .{},
};

/// The same document as `Screen.renderJson`, built from whole rows
/// instead of a cell list. Each entry of `rows` is one screen row:
/// pieces of text with one style each, laid end to end. A row is
/// padded with spaces to `width` columns and cut off at it; missing
/// rows are blank and rows past `height` are dropped. A column is one
/// Unicode scalar value, as in `chars`. Adjacent pieces with equal
/// styles merge, and the default style (all zero) writes no span, so
/// the output matches `renderJson` for the same picture. Cost grows
/// with the number of pieces, not the number of cells, so prefer a
/// few long pieces per row to one piece per cell.
pub fn renderRowsJson(
    alloc: std.mem.Allocator,
    width: u64,
    height: u64,
    rows: []const []const Piece,
) ![]u8 {
    var out: std.ArrayListUnmanaged(u8) = .empty;
    errdefer out.deinit(alloc);
    try appendDocumentHead(&out, alloc, width, height);
    for (0..@intCast(height)) |row| {
        if (row > 0) try out.append(alloc, ',');
        const built = try buildRow(alloc, width, piecesAt(rows, row));
        defer alloc.free(built.text);
        try appendJsonQuote(&out, alloc, built.text);
    }
    try out.appendSlice(alloc, "],\"styles\":[");
    for (0..@intCast(height)) |row| {
        if (row > 0) try out.append(alloc, ',');
        const built = try buildRow(alloc, width, piecesAt(rows, row));
        defer alloc.free(built.spans);
        try out.append(alloc, '[');
        try out.appendSlice(alloc, built.spans);
        try out.append(alloc, ']');
    }
    try out.appendSlice(alloc, "]}");
    return out.toOwnedSlice(alloc);
}

/// The top or bottom edge of a box `w` columns wide: `+---+`, with
/// `title` written over it from the third column and cut off before
/// the corner. Use it with `renderRowsJson` where `boxed` would need
/// a cell list.
pub fn boxEdge(alloc: std.mem.Allocator, w: u64, title: []const u8) ![]u8 {
    if (w < 2) return alloc.dupe(u8, "");
    var out: std.ArrayListUnmanaged(u8) = .empty;
    errdefer out.deinit(alloc);
    try out.append(alloc, '+');
    for (0..@intCast(w - 2)) |_| try out.append(alloc, '-');
    try out.append(alloc, '+');
    if (w < 4 or title.len == 0) return out.toOwnedSlice(alloc);
    const label = fit(title, w - 3).text;
    const tail = out.items[2 + label.len ..];
    const kept = try alloc.dupe(u8, tail);
    defer alloc.free(kept);
    out.shrinkRetainingCapacity(2);
    try out.appendSlice(alloc, label);
    try out.appendSlice(alloc, kept);
    return out.toOwnedSlice(alloc);
}

/// Split text into one slice per Unicode scalar value. A group of
/// bytes that is not valid UTF-8 becomes `"?"`, mirroring Roc
/// `chars`. The slices borrow `text` (except the `"?"` replacements).
pub fn chars(alloc: std.mem.Allocator, text: []const u8) ![][]const u8 {
    const count = charCount(text);
    const out = try alloc.alloc([]const u8, count);
    var i: usize = 0;
    var n: usize = 0;
    while (i < text.len) {
        out[n] = charAt(text, i);
        n += 1;
        i += charLenAt(text, i);
    }
    return out;
}

/// How many Unicode scalar values `text` holds, counting an invalid
/// byte group as one.
pub fn charCount(text: []const u8) usize {
    var count: usize = 0;
    var i: usize = 0;
    while (i < text.len) {
        count += 1;
        i += charLenAt(text, i);
    }
    return count;
}

/// `text` as a JSON string literal, quotes included, with the Roc
/// `Screen.json_quote` escapes.
pub fn jsonQuote(alloc: std.mem.Allocator, text: []const u8) ![]u8 {
    var out: std.ArrayListUnmanaged(u8) = .empty;
    errdefer out.deinit(alloc);
    try appendJsonQuote(&out, alloc, text);
    return out.toOwnedSlice(alloc);
}

/// The byte length of the char starting at `i`, grouping UTF-8
/// continuation bytes with their lead byte (mirrors Roc `chars`).
fn charLenAt(bytes: []const u8, i: usize) usize {
    var n: usize = 1;
    while (i + n < bytes.len and bytes[i + n] >= 128 and bytes[i + n] < 192) {
        n += 1;
    }
    return n;
}

/// The char starting at `i`, or `"?"` when its bytes are not valid
/// UTF-8.
fn charAt(bytes: []const u8, i: usize) []const u8 {
    const raw = bytes[i .. i + charLenAt(bytes, i)];
    if (std.unicode.utf8ValidateSlice(raw)) return raw;
    return "?";
}

const Fit = struct {
    text: []const u8,
    columns: u64,
};

/// `text` cut to at most `room` columns, with the columns it kept.
fn fit(text: []const u8, room: u64) Fit {
    const columns: u64 = @intCast(charCount(text));
    if (columns <= room) return .{ .text = text, .columns = columns };
    var i: usize = 0;
    var n: u64 = 0;
    while (n < room) : (n += 1) {
        i += charLenAt(text, i);
    }
    return .{ .text = text[0..i], .columns = room };
}

const BuiltRow = struct {
    text: []u8,
    spans: []u8,
};

fn piecesAt(rows: []const []const Piece, row: usize) []const Piece {
    if (row < rows.len) return rows[row];
    return &.{};
}

/// One row of `renderRowsJson`: its padded text and its style spans
/// (the span list without the surrounding brackets).
fn buildRow(
    alloc: std.mem.Allocator,
    width: u64,
    pieces: []const Piece,
) !BuiltRow {
    var text: std.ArrayListUnmanaged(u8) = .empty;
    errdefer text.deinit(alloc);
    var spans: std.ArrayListUnmanaged(u8) = .empty;
    errdefer spans.deinit(alloc);
    var first = true;
    var x: u64 = 0;
    var run_x: u64 = 0;
    var run_len: u64 = 0;
    var run_style = Style{};
    for (pieces) |piece| {
        const fitted = fit(piece.text, width - x);
        if (fitted.columns == 0) continue;
        if (run_len > 0 and piece.style.eql(run_style)) {
            run_len += fitted.columns;
        } else {
            try appendSpan(&spans, alloc, &first, run_x, run_len, run_style);
            run_x = x;
            run_len = fitted.columns;
            run_style = piece.style;
        }
        try text.appendSlice(alloc, fitted.text);
        x += fitted.columns;
    }
    try appendSpan(&spans, alloc, &first, run_x, run_len, run_style);
    for (x..@intCast(width)) |_| try text.append(alloc, ' ');
    return .{
        .text = try text.toOwnedSlice(alloc),
        .spans = try spans.toOwnedSlice(alloc),
    };
}

/// Append one style span to a span list, unless the run is empty or
/// default-styled. `first` tracks the comma placement.
fn appendSpan(
    out: *std.ArrayListUnmanaged(u8),
    alloc: std.mem.Allocator,
    first: *bool,
    x: u64,
    length: u64,
    s: Style,
) !void {
    if (length == 0 or s.isDefault()) return;
    if (!first.*) try out.append(alloc, ',');
    first.* = false;
    try out.appendSlice(alloc, "{\"x\":");
    try appendUint(out, alloc, x);
    try out.appendSlice(alloc, ",\"length\":");
    try appendUint(out, alloc, length);
    try out.appendSlice(alloc, ",\"fg\":");
    try appendUint(out, alloc, s.fg);
    try out.appendSlice(alloc, ",\"bg\":");
    try appendUint(out, alloc, s.bg);
    try out.appendSlice(alloc, ",\"bold\":");
    try out.appendSlice(alloc, if (s.bold) "true" else "false");
    try out.append(alloc, '}');
}

fn appendDocumentHead(
    out: *std.ArrayListUnmanaged(u8),
    alloc: std.mem.Allocator,
    width: u64,
    height: u64,
) !void {
    try out.appendSlice(alloc, "{\"kind\":\"screen\",\"version\":1,\"width\":");
    try appendUint(out, alloc, width);
    try out.appendSlice(alloc, ",\"height\":");
    try appendUint(out, alloc, height);
    try out.appendSlice(alloc, ",\"rows\":[");
}

fn appendJsonQuote(
    out: *std.ArrayListUnmanaged(u8),
    alloc: std.mem.Allocator,
    text: []const u8,
) !void {
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

fn appendSgr(
    out: *std.ArrayListUnmanaged(u8),
    alloc: std.mem.Allocator,
    s: Style,
) !void {
    if (s.bold) try out.appendSlice(alloc, "\x1b[1m");
    if (s.fg != 0) {
        try out.appendSlice(alloc, "\x1b[38;5;");
        try appendUint(out, alloc, s.fg);
        try out.append(alloc, 'm');
    }
    if (s.bg != 0) {
        try out.appendSlice(alloc, "\x1b[48;5;");
        try appendUint(out, alloc, s.bg);
        try out.append(alloc, 'm');
    }
}

fn appendUint(out: *std.ArrayListUnmanaged(u8), alloc: std.mem.Allocator, value: u64) !void {
    if (value == 0) {
        try out.append(alloc, '0');
        return;
    }
    var buf: [20]u8 = undefined;
    var i: usize = buf.len;
    var n = value;
    while (n > 0) {
        i -= 1;
        buf[i] = @intCast('0' + n % 10);
        n /= 10;
    }
    try out.appendSlice(alloc, buf[i..]);
}

// ---------------------------------------------------------------------------
// Tests — mirrors of the Roc `expect` cases in Screen.roc.
// ---------------------------------------------------------------------------

const testing = std.testing;

fn expectRenderedPlain(screen: Screen, expected: []const u8) !void {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const text = try screen.renderPlain(arena.allocator());
    try testing.expectEqualStrings(expected, text);
}

fn expectRenderedJson(screen: Screen, expected: []const u8) !void {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const text = try screen.renderJson(arena.allocator());
    try testing.expectEqualStrings(expected, text);
}

test "screen: chars splits on Unicode scalar values" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const ascii = try chars(alloc, "ab");
    try testing.expectEqual(@as(usize, 2), ascii.len);
    try testing.expectEqualStrings("a", ascii[0]);
    try testing.expectEqualStrings("b", ascii[1]);
    const accented = try chars(alloc, "h\xc3\xa9");
    try testing.expectEqual(@as(usize, 2), accented.len);
    try testing.expectEqualStrings("h", accented[0]);
    try testing.expectEqualStrings("\xc3\xa9", accented[1]);
    try testing.expectEqual(@as(usize, 3), charCount("a\xc3\xa9z"));
}

test "screen: put_text writes and render_plain reads back" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var screen = try Screen.init(alloc, 4, 2);
    screen.putText(1, 0, "hi", .{});
    try expectRenderedPlain(screen, " hi \n    ");
}

test "screen: put_text clips at the right edge" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var screen = try Screen.init(alloc, 3, 1);
    screen.putText(2, 0, "abc", .{});
    try expectRenderedPlain(screen, "  a");
}

test "screen: boxed draws a titled border" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var screen = try Screen.init(alloc, 6, 3);
    screen.boxed(0, 0, 6, 3, "t");
    try expectRenderedPlain(screen, "+-t--+\n|    |\n+----+");
}

test "screen: render_json of a default-styled row writes no spans" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var screen = try Screen.init(alloc, 2, 1);
    screen.putText(0, 0, "ok", .{});
    try expectRenderedJson(screen, "{\"kind\":\"screen\",\"version\":1,\"width\":2,\"height\":1,\"rows\":[\"ok\"],\"styles\":[[]]}");
}

test "screen: render_json merges a styled run into one span" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var screen = try Screen.init(alloc, 3, 1);
    screen.putText(0, 0, "abc", .{ .fg = 196, .bold = true });
    try expectRenderedJson(screen, "{\"kind\":\"screen\",\"version\":1,\"width\":3,\"height\":1,\"rows\":[\"abc\"],\"styles\":[[{\"x\":0,\"length\":3,\"fg\":196,\"bg\":0,\"bold\":true}]]}");
}

test "screen: render_json drops default runs between styled runs" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var screen = try Screen.init(alloc, 3, 1);
    screen.putText(0, 0, "a", .{ .fg = 2 });
    screen.putText(1, 0, "b", .{});
    screen.putText(2, 0, "c", .{ .fg = 2 });
    try expectRenderedJson(screen, "{\"kind\":\"screen\",\"version\":1,\"width\":3,\"height\":1,\"rows\":[\"abc\"],\"styles\":[[{\"x\":0,\"length\":1,\"fg\":2,\"bg\":0,\"bold\":false},{\"x\":2,\"length\":1,\"fg\":2,\"bg\":0,\"bold\":false}]]}");
}

test "screen: json_quote escapes quotes, backslashes, and controls" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const quoted = try jsonQuote(alloc, "a\"\\\n");
    try testing.expectEqualStrings("\"a\\\"\\\\\\n\"", quoted);
    const plain = try jsonQuote(alloc, "plain");
    try testing.expectEqualStrings("\"plain\"", plain);
}

test "screen: render_rows_json pads a row to width" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const rows = [_][]const Piece{&[_]Piece{.{ .text = "ab" }}};
    const text = try renderRowsJson(alloc, 4, 1, &rows);
    try testing.expectEqualStrings("{\"kind\":\"screen\",\"version\":1,\"width\":4,\"height\":1,\"rows\":[\"ab  \"],\"styles\":[[]]}", text);
}

test "screen: render_rows_json merges equal styles and blanks missing rows" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const hot = Style{ .fg = 196, .bold = true };
    const rows = [_][]const Piece{&[_]Piece{
        .{ .text = "a", .style = hot },
        .{ .text = "bc", .style = hot },
    }};
    const text = try renderRowsJson(alloc, 3, 2, &rows);
    try testing.expectEqualStrings("{\"kind\":\"screen\",\"version\":1,\"width\":3,\"height\":2,\"rows\":[\"abc\",\"   \"],\"styles\":[[{\"x\":0,\"length\":3,\"fg\":196,\"bg\":0,\"bold\":true}],[]]}", text);
}

test "screen: render_rows_json cuts pieces at width and skips default spans" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const rows = [_][]const Piece{&[_]Piece{
        .{ .text = "a\"" },
        .{ .text = "xyz", .style = .{ .fg = 2 } },
    }};
    const text = try renderRowsJson(alloc, 3, 1, &rows);
    try testing.expectEqualStrings("{\"kind\":\"screen\",\"version\":1,\"width\":3,\"height\":1,\"rows\":[\"a\\\"x\"],\"styles\":[[{\"x\":2,\"length\":1,\"fg\":2,\"bg\":0,\"bold\":false}]]}", text);
}

test "screen: box_edge writes the title over the top edge" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    try testing.expectEqualStrings("+-ti---+", try boxEdge(alloc, 8, "ti"));
    try testing.expectEqualStrings("+-too+", try boxEdge(alloc, 6, "toolong"));
    try testing.expectEqualStrings("+--+", try boxEdge(alloc, 4, ""));
    try testing.expectEqualStrings("", try boxEdge(alloc, 1, "x"));
}

test "screen: put_row and rule draw styled cells" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var screen = try Screen.init(alloc, 5, 2);
    screen.putRow(0, 1, &[_]Cell{ .{ .ch = "#" }, .{ .ch = "@" } });
    screen.rule(0, 1, 5, "-", .{});
    try expectRenderedPlain(screen, " #@  \n-----");
    // A row past the bottom is a no-op, as is an over-long row past the edge.
    screen.putRow(2, 0, &[_]Cell{.{ .ch = "x" }});
    screen.putRow(0, 4, &[_]Cell{ .{ .ch = "x" }, .{ .ch = "y" } });
    try expectRenderedPlain(screen, " #@ x\n-----");
}

test "screen: put_grid draws grid cells through a glyph" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const glyph = struct {
        fn glyph(value: u8) []const u8 {
            return switch (value) {
                1 => "#",
                2 => "o",
                3 => "x",
                4 => "@",
                else => " ",
            };
        }
    }.glyph;
    const g = try grid.Grid.fromCells(alloc, 2, 2, &[_]u8{ 1, 2, 3, 4 }, 0);
    var screen = try Screen.init(alloc, 4, 2);
    try screen.putGrid(alloc, 1, 0, g, glyph, .{});
    try expectRenderedPlain(screen, " #o \n x@ ");
}

test "screen: render_ansi homes the cursor and resets per style change" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var screen = try Screen.init(alloc, 2, 1);
    screen.putText(0, 0, "ab", .{});
    const plain = try screen.renderAnsi(alloc);
    try testing.expectEqualStrings("\x1b[Hab\x1b[0m", plain);
    var hot = try Screen.init(alloc, 2, 1);
    hot.putText(0, 0, "a", .{ .fg = 196, .bold = true });
    hot.putText(1, 0, "b", .{});
    const styled = try hot.renderAnsi(alloc);
    try testing.expectEqualStrings("\x1b[H\x1b[0m\x1b[1m\x1b[38;5;196ma\x1b[0mb\x1b[0m", styled);
}
