## Bounded row-major grids for portable world programs.
##
## A grid is a width, a height, and a row-major list of cells. `get` and `set`
## are bounds-checked and never trap: a read outside the grid returns
## `fallback`, and a write outside it returns the grid unchanged. Game and data
## programs build their boards, canvases, and inventories on this instead of
## re-deriving index arithmetic in every app.
RocGrid := {
	width : U64,
	height : U64,
	cells : List(U8),
}.{
	## A `width` by `height` grid with every cell set to `fill`.
	new : U64, U64, U8 -> RocGrid
	new = |width, height, fill| {
		width: width,
		height: height,
		cells: List.repeat(fill, width * height),
	}

	## Whether (`x`, `y`) is inside the grid.
	in_bounds : RocGrid, U64, U64 -> Bool
	in_bounds = |grid, x, y| x < grid.width and y < grid.height

	## The row-major index of (`x`, `y`). Only meaningful inside the grid.
	index : RocGrid, U64, U64 -> U64
	index = |grid, x, y| y * grid.width + x

	## The cell at (`x`, `y`), or `fallback` outside the grid.
	get : RocGrid, U64, U64, U8 -> U8
	get = |grid, x, y, fallback|
		if in_bounds(grid, x, y) {
			match List.get(grid.cells, index(grid, x, y)) {
				Ok(cell) => cell
				Err(_) => fallback
			}
		} else {
			fallback
		}

	## The grid with (`x`, `y`) set to `cell`; unchanged outside the grid.
	set : RocGrid, U64, U64, U8 -> RocGrid
	set = |grid, x, y, cell|
		if in_bounds(grid, x, y) {
			match List.set(grid.cells, index(grid, x, y), cell) {
				Ok(cells) => { width: grid.width, height: grid.height, cells: cells }
				Err(_) => grid
			}
		} else {
			grid
		}

	## How many cells hold `value`.
	count : RocGrid, U8 -> U64
	count = |grid, value| List.len(List.keep_if(grid.cells, |cell| cell == value))

	## The four orthogonal neighbours of (`x`, `y`) that are inside the grid:
	## up, down, left, right, in that order. A neighbour outside the grid is
	## left out, so edge and corner cells yield fewer than four.
	neighbors4 : RocGrid, U64, U64 -> List({ x : U64, y : U64 })
	neighbors4 = |grid, x, y| {
		up = if y > 0 { [{ x: x, y: y - 1 }] } else { [] }
		down = if y + 1 < grid.height { [{ x: x, y: y + 1 }] } else { [] }
		left = if x > 0 { [{ x: x - 1, y: y }] } else { [] }
		right = if x + 1 < grid.width { [{ x: x + 1, y: y }] } else { [] }
		List.concat(List.concat(List.concat(up, down), left), right)
	}

	## Cells of one row, left to right; an empty list for `y` outside the grid.
	row : RocGrid, U64 -> List(U8)
	row = |grid, y|
		if y < grid.height {
			List.sublist(grid.cells, { start: y * grid.width, len: grid.width })
		} else {
			[]
		}

	## Every cell, row-major, as a byte list (a reference, not a copy).
	bytes : RocGrid -> List(U8)
	bytes = |grid| grid.cells

	## The grid's size.
	dimensions : RocGrid -> { width : U64, height : U64 }
	dimensions = |grid| { width: grid.width, height: grid.height }

	## A grid of `width` by `height` from up to that many bytes of `cells`,
	## padding short input with `fill` and ignoring extra input.
	from_cells : U64, U64, List(U8), U8 -> RocGrid
	from_cells = |width, height, cells, fill| {
		taken = List.take_first(cells, width * height)
		padded = List.concat(taken, List.repeat(fill, width * height - List.len(taken)))
		{ width: width, height: height, cells: padded }
	}
}
