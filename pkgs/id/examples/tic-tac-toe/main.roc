app [program] { pf: platform "../roc-world/platform.roc" }

# A two-player tic-tac-toe world.
#
# State is a 9-character board string, the ply count, and the winner. The
# turn is the ply parity, so `update` is a pure function of (model, event)
# and the journal replays it exactly. Players send the cell they want
# (a digit 0-8 as a string); occupied cells, moves after a win, and moves
# outside the board are ignored.

Model : {
    board : Str,
    plays : U64,
    winner : Str,
}

program = { init, update, view, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { board: "---------", plays: 0, winner: "" }

update : Model, U64, Str -> Model
update = |model, _pid, ev| {
    cells = Str.to_utf8(model.board)
    cell = parse_cell(ev)

    if model.winner != "" or cell > 8 or cell_at(cells, cell) != empty_cell {
        model
    } else {
        mark = if model.plays % 2 == 0 x_mark else o_mark
        board = match List.set(cells, cell, mark) {
            Ok(updated) => match Str.from_utf8(updated) {
                Ok(text) => text
                Err(_) => model.board
            }
            Err(_) => model.board
        }
        { board: board, plays: model.plays + 1, winner: winner_of(Str.to_utf8(board)) }
    }
}

view : Model, Str -> Str
view = |model, _viewer| {
    state = if model.winner == "" {
        turn = if model.plays % 2 == 0 "X" else "O"
        "next=${turn}"
    } else {
        "winner=${model.winner}"
    }
    "${model.board}  plays=${model.plays.to_str()} ${state}"
}

## This program wants nothing from the server.
wants : Model -> Str
wants = |_model| "{}"

records : Model -> Str
records = |model| "{\"board\":\"${model.board}\",\"plays\":${model.plays.to_str()},\"winner\":\"${model.winner}\"}"

## The board is the whole state: the ply count is the number of marks on it
## and the winner follows from its lines, so nothing else is stored.
snapshot : Model -> Str
snapshot = |model| model.board

restore : Str -> Model
restore = |board| {
    cells = Str.to_utf8(board)
    if List.len(cells) != 9 {
        { board: "---------", plays: 0, winner: "" }
    } else {
        marks = List.keep_if(cells, |cell| cell != empty_cell)
        { board: board, plays: List.len(marks), winner: winner_of(cells) }
    }
}

# --- helpers ---------------------------------------------------------------

empty_cell : U8
empty_cell = 45 # '-'

x_mark : U8
x_mark = 88 # 'X'

o_mark : U8
o_mark = 79 # 'O'

## A cell digit "0".."8" as a cell index; 255 means "not a move".
parse_cell = |ev| {
    bytes = Str.to_utf8(ev)
    if List.len(bytes) == 1 {
        match List.first(bytes) {
            Ok(digit) => if digit >= 48 and digit <= 56 digit.to_u64() - 48 else 255
            Err(_) => 255
        }
    } else {
        255
    }
}

cell_at = |cells, index|
    match List.get(cells, index) {
        Ok(byte) => byte
        Err(_) => empty_cell
    }

mark_name = |cell| if cell == x_mark "X" else "O"

has_line = |cells, a, b, c| {
    cell = cell_at(cells, a)
    cell != empty_cell and cell == cell_at(cells, b) and cell == cell_at(cells, c)
}

winner_of = |cells|
    if has_line(cells, 0, 1, 2) mark_name(cell_at(cells, 0))
    else if has_line(cells, 3, 4, 5) mark_name(cell_at(cells, 3))
    else if has_line(cells, 6, 7, 8) mark_name(cell_at(cells, 6))
    else if has_line(cells, 0, 3, 6) mark_name(cell_at(cells, 0))
    else if has_line(cells, 1, 4, 7) mark_name(cell_at(cells, 1))
    else if has_line(cells, 2, 5, 8) mark_name(cell_at(cells, 2))
    else if has_line(cells, 0, 4, 8) mark_name(cell_at(cells, 0))
    else if has_line(cells, 2, 4, 6) mark_name(cell_at(cells, 2))
    else ""
