//! Bounded row-major grids for Zig guest programs.
//!
//! A Zig port of the Roc `RocGrid` module
//! (`pkgs/id/examples/roc-world/RocGrid.roc`): a width, a height, and a
//! row-major cell buffer. `get` and `set` are bounds-checked and never
//! trap: a read outside the grid returns `fallback`, and a write
//! outside it is a no-op. Game and data programs build their boards,
//! canvases, and inventories on this instead of re-deriving index
//! arithmetic in every app.
//!
//! One deliberate difference from Roc: Roc values are immutable, so
//! `RocGrid.set` returns a new grid; here `set` mutates the grid in
//! place (`*Grid`), which is the Zig idiom and avoids a copy per cell
//! write inside a wasm32 guest. `get`, `count`, and the read helpers
//! take the grid by value (a fat pointer) and do not mutate it.

const std = @import("std");

pub const Pos = struct {
    x: u64,
    y: u64,
};

/// The four orthogonal neighbours of a cell that are inside the grid,
/// in up, down, left, right order. Edge and corner cells yield fewer
/// than four; `len` says how many entries of `items` are valid.
pub const Neighbors = struct {
    items: [4]Pos,
    len: usize,
};

pub const InitError = error{
    OutOfMemory,
    InvalidDimensions,
};

pub const Grid = struct {
    width: u64,
    height: u64,
    cells: []u8,

    /// A `width` by `height` grid with every cell set to `fill`.
    pub fn init(alloc: std.mem.Allocator, width: u64, height: u64, fill: u8) InitError!Grid {
        const cell_count = std.math.mul(u64, width, height) catch
            return error.InvalidDimensions;
        const cells = try alloc.alloc(u8, cell_count);
        @memset(cells, fill);
        return .{ .width = width, .height = height, .cells = cells };
    }

    pub fn deinit(self: Grid, alloc: std.mem.Allocator) void {
        alloc.free(self.cells);
    }

    /// Whether (`x`, `y`) is inside the grid.
    pub fn inBounds(self: Grid, x: u64, y: u64) bool {
        return x < self.width and y < self.height;
    }

    /// The row-major index of (`x`, `y`). Only meaningful inside the grid.
    pub fn index(self: Grid, x: u64, y: u64) u64 {
        return y * self.width + x;
    }

    /// The cell at (`x`, `y`), or `fallback` outside the grid.
    pub fn get(self: Grid, x: u64, y: u64, fallback: u8) u8 {
        if (!self.inBounds(x, y)) return fallback;
        return self.cells[self.index(x, y)];
    }

    /// Set the cell at (`x`, `y`); a no-op outside the grid.
    pub fn set(self: *Grid, x: u64, y: u64, cell: u8) void {
        if (!self.inBounds(x, y)) return;
        self.cells[self.index(x, y)] = cell;
    }

    /// How many cells hold `value`.
    pub fn count(self: Grid, value: u8) u64 {
        var total: u64 = 0;
        for (self.cells) |cell| {
            if (cell == value) total += 1;
        }
        return total;
    }

    /// The four orthogonal neighbours of (`x`, `y`) that are inside the
    /// grid, in up, down, left, right order.
    pub fn neighbors4(self: Grid, x: u64, y: u64) Neighbors {
        var out = Neighbors{ .items = undefined, .len = 0 };
        if (y > 0) {
            out.items[out.len] = .{ .x = x, .y = y - 1 };
            out.len += 1;
        }
        if (y + 1 < self.height) {
            out.items[out.len] = .{ .x = x, .y = y + 1 };
            out.len += 1;
        }
        if (x > 0) {
            out.items[out.len] = .{ .x = x - 1, .y = y };
            out.len += 1;
        }
        if (x + 1 < self.width) {
            out.items[out.len] = .{ .x = x + 1, .y = y };
            out.len += 1;
        }
        return out;
    }

    /// Cells of one row, left to right; an empty slice for `y` outside
    /// the grid.
    pub fn row(self: Grid, y: u64) []const u8 {
        if (y >= self.height) return &.{};
        return self.cells[y * self.width .. (y + 1) * self.width];
    }

    /// Every cell, row-major.
    pub fn bytes(self: Grid) []const u8 {
        return self.cells;
    }

    /// The grid's size.
    pub fn dimensions(self: Grid) struct { width: u64, height: u64 } {
        return .{ .width = self.width, .height = self.height };
    }

    /// A grid of `width` by `height` from up to that many bytes of
    /// `cells`, padding short input with `fill` and ignoring extra input.
    pub fn fromCells(
        alloc: std.mem.Allocator,
        width: u64,
        height: u64,
        cells: []const u8,
        fill: u8,
    ) InitError!Grid {
        var grid = try Grid.init(alloc, width, height, fill);
        const take = @min(cells.len, grid.cells.len);
        @memcpy(grid.cells[0..take], cells[0..take]);
        return grid;
    }
};

// ---------------------------------------------------------------------------
// Tests — mirrors of the Roc `expect` behaviour in RocGrid.roc.
// ---------------------------------------------------------------------------

const testing = std.testing;

test "grid: new fills and get/set are bounds-checked" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var grid = try Grid.init(alloc, 3, 2, 0);
    try testing.expectEqual(@as(u64, 3), grid.width);
    try testing.expectEqual(@as(u64, 2), grid.height);
    try testing.expectEqual(@as(u8, 0), grid.get(0, 0, 99));
    try testing.expectEqual(@as(u8, 99), grid.get(3, 0, 99));
    try testing.expectEqual(@as(u8, 99), grid.get(0, 2, 99));
    grid.set(1, 0, 7);
    try testing.expectEqual(@as(u8, 7), grid.get(1, 0, 99));
    grid.set(3, 0, 7); // outside: no-op, no trap
    grid.set(0, 2, 7);
    try testing.expectEqual(@as(u64, 1), grid.count(7));
    try testing.expectEqual(@as(u64, 5), grid.count(0));
}

test "grid: index is row-major" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const grid = try Grid.init(arena.allocator(), 4, 3, 0);
    try testing.expectEqual(@as(u64, 0), grid.index(0, 0));
    try testing.expectEqual(@as(u64, 3), grid.index(3, 0));
    try testing.expectEqual(@as(u64, 4), grid.index(0, 1));
    try testing.expectEqual(@as(u64, 11), grid.index(3, 2));
    try testing.expect(grid.inBounds(3, 2));
    try testing.expect(!grid.inBounds(4, 2));
    try testing.expect(!grid.inBounds(0, 3));
}

test "grid: neighbors4 order is up, down, left, right" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const grid = try Grid.init(arena.allocator(), 3, 3, 0);
    const center = grid.neighbors4(1, 1);
    try testing.expectEqual(@as(usize, 4), center.len);
    try testing.expectEqual(Pos{ .x = 1, .y = 0 }, center.items[0]);
    try testing.expectEqual(Pos{ .x = 1, .y = 2 }, center.items[1]);
    try testing.expectEqual(Pos{ .x = 0, .y = 1 }, center.items[2]);
    try testing.expectEqual(Pos{ .x = 2, .y = 1 }, center.items[3]);
    const corner = grid.neighbors4(0, 0);
    try testing.expectEqual(@as(usize, 2), corner.len);
    try testing.expectEqual(Pos{ .x = 0, .y = 1 }, corner.items[0]);
    try testing.expectEqual(Pos{ .x = 1, .y = 0 }, corner.items[1]);
    const edge = grid.neighbors4(1, 0);
    try testing.expectEqual(@as(usize, 3), edge.len);
}

test "grid: row, bytes, dimensions" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    var grid = try Grid.init(alloc, 3, 2, 0);
    grid.set(0, 0, 1);
    grid.set(1, 0, 2);
    grid.set(2, 1, 3);
    try testing.expectEqualSlices(u8, &[_]u8{ 1, 2, 0 }, grid.row(0));
    try testing.expectEqualSlices(u8, &[_]u8{ 0, 0, 3 }, grid.row(1));
    try testing.expectEqual(@as(usize, 0), grid.row(2).len);
    try testing.expectEqualSlices(u8, &[_]u8{ 1, 2, 0, 0, 0, 3 }, grid.bytes());
    try testing.expectEqual(@as(u64, 3), grid.dimensions().width);
    try testing.expectEqual(@as(u64, 2), grid.dimensions().height);
}

test "grid: fromCells pads short input and ignores extra input" {
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const short = try Grid.fromCells(alloc, 3, 2, &[_]u8{ 1, 2 }, 9);
    try testing.expectEqualSlices(u8, &[_]u8{ 1, 2, 9, 9, 9, 9 }, short.bytes());
    const long = try Grid.fromCells(alloc, 2, 1, &[_]u8{ 5, 6, 7, 8 }, 0);
    try testing.expectEqualSlices(u8, &[_]u8{ 5, 6 }, long.bytes());
    const empty = try Grid.fromCells(alloc, 2, 2, &[_]u8{}, 4);
    try testing.expectEqual(@as(u64, 4), empty.count(4));
}
