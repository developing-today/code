app [program] { pf: platform "../../../examples/roc-world/platform.roc" }

import pf.RocJson
import pf.Screen
import pf.Key

# Checkers, as a Roc world program (slice 1 of #14862).
#
# The human plays red (bottom, moving up) against a deliberately weak
# AI playing black. Full English-draughts rules: captures are forced,
# multi-jumps are single moves, a man that reaches the far row is
# crowned and the turn ends, and kings move and capture both ways. A
# side with no pieces or no legal move loses.
#
# The AI is weak on purpose (the issue's acceptance bar): it looks
# exactly one ply ahead, scores a move only by the material it captures
# this turn plus a small advance/crowning bonus, adds uniform jitter to
# every score, and one draw in four ignores the scores entirely and
# plays a uniformly random legal move. It never checks whether the
# piece it just moved can be recaptured. A beginner is hampered; a
# competent child beats it easily. No search, no opening book, no
# positional table beyond the advance bonus.
#
# Everything is a pure function of (model, event): keystrokes are
# journaled like any other world input, the AI's randomness comes from
# a xorshift PRNG carried in the model, and the journal replays a game
# exactly. The AI replies inside the same `update` that completes the
# human's turn, so at rest it is always red to move.
#
# The frame is data: `view`/`layout` render a `Screen` and the shell
# paints it in whichever presentation mode the session negotiated
# (alternate, primary, or low-write text, #14936/#14937). The game
# never changes when the mode does.
#
# Controls: arrow keys or WASD move the cursor, Enter (or Space)
# selects a piece and confirms a landing square; selecting a piece
# with a jump mid-chain must keep jumping. Enter (or R) starts a new
# game after a win or a loss.
#
# Seats (#15111, the multiplayer slice of #14862): each colour has a
# seat holding the participant id that plays it, 0 when open. The
# first participant to send input takes red; another participant takes
# black by pressing J, which retires the AI. While black's seat is
# open the weak AI plays black exactly as in single-player. Turn
# enforcement lives here, in `update`, not in any UI: a key from a
# participant who does not hold the seat of the side to move — the
# other player out of turn, or a spectator — is dropped and the model
# is returned unchanged. The host (participant 0) never types.
#
# Later slices of #15111 script the two-process game over the id
# network (host/join and central server); #14862's three-machine
# demonstration follows.

Model : {
    phase : Str,
    board : Str,
    cursor : U64,
    selected : U64,
    sel : U64,
    chain : U64,
    rng : U64,
    plies : U64,
    turn : U64,
    red_seat : U64,
    black_seat : U64,
}

## One full move: the path of landing squares (one for a step or a
## single jump, more for a multi-jump chain) and the jumped pieces.
Move : { from : U64, landings : List(U64), captured : List(U64) }

program = { init, update, view, layout, records, wants, snapshot, restore }

# --- board basics ------------------------------------------------------------
#
# The board is a 64-byte string, row-major, row 0 at the top (black's
# home rows) and row 7 at the bottom (red's). Bytes: '.' empty,
# 'r'/'R' red man/king, 'b'/'B' black man/king. Pieces sit on dark
# squares only: (row + col) odd.

empty_cell : U8
empty_cell = 46

board_from_rows : List(Str) -> Str
board_from_rows = |rows| List.fold(rows, "", |acc, row| Str.concat(acc, row))

start_board : Str
start_board = board_from_rows([
    ".b.b.b.b",
    "b.b.b.b.",
    ".b.b.b.b",
    "........",
    "........",
    "r.r.r.r.",
    ".r.r.r.r",
    "r.r.r.r.",
])

cell_at : Str, U64 -> U8
cell_at = |board, index|
    match List.get(Str.to_utf8(board), index) {
        Ok(cell) => cell
        Err(_) => empty_cell
    }

set_cell : Str, U64, U8 -> Str
set_cell = |board, index, value| {
    cells = Str.to_utf8(board)
    match List.set(cells, index, value) {
        Ok(updated) =>
            match Str.from_utf8(updated) {
                Ok(text) => text
                Err(_) => board
            }

        Err(_) => board
    }
}

is_red : U8 -> Bool
is_red = |cell| cell == 114 or cell == 82

is_black : U8 -> Bool
is_black = |cell| cell == 98 or cell == 66

is_king : U8 -> Bool
is_king = |cell| cell == 82 or cell == 66

is_man : U8 -> Bool
is_man = |cell| cell == 114 or cell == 98

is_enemy : U8, U8 -> Bool
is_enemy = |piece, cell|
    if is_red(piece) {
        is_black(cell)
    } else {
        is_red(cell)
    }

count_side : Str, Str -> U64
count_side = |board, side|
    List.fold(Str.to_utf8(board), 0, |acc, cell|
        if (side == "red" and is_red(cell)) or (side == "black" and is_black(cell)) {
            acc + 1
        } else {
            acc
        })

## Diagonal directions as tags: 0 = up-left, 1 = up-right,
## 2 = down-left, 3 = down-right (up = toward row 0, red's direction).
step_idx : U64, U64 -> Try(U64, {})
step_idx = |index, dir| {
    row = index / 8
    col = index % 8
    if dir == 0 and row > 0 and col > 0 {
        Ok(index - 9)
    } else if dir == 1 and row > 0 and col < 7 {
        Ok(index - 7)
    } else if dir == 2 and row < 7 and col > 0 {
        Ok(index + 7)
    } else if dir == 3 and row < 7 and col < 7 {
        Ok(index + 9)
    } else {
        Err({})
    }
}

dirs_for : U8 -> List(U64)
dirs_for = |piece|
    if is_king(piece) {
        [0, 1, 2, 3]
    } else if is_red(piece) {
        [0, 1]
    } else {
        [2, 3]
    }

king_row_for : U8, U64 -> Bool
king_row_for = |piece, index|
    if is_red(piece) {
        index / 8 == 0
    } else {
        index / 8 == 7
    }

king_of : U8 -> U8
king_of = |piece|
    if is_red(piece) {
        82
    } else {
        66
    }

# --- move generation ---------------------------------------------------------

## One diagonal step onto an empty square.
steps_from : Str, U64, U8 -> List(U64)
steps_from = |board, index, piece|
    List.fold(dirs_for(piece), [], |acc, dir|
        match step_idx(index, dir) {
            Ok(land) =>
                if cell_at(board, land) == empty_cell {
                    List.append(acc, land)
                } else {
                    acc
                }

            Err(_) => acc
        })

## The midpoint and landing of a jump in one direction, if the
## midpoint holds an enemy and the landing is empty.
jump_step : Str, U64, U8, U64 -> Try({ mid : U64, land : U64 }, {})
jump_step = |board, index, piece, dir|
    match step_idx(index, dir) {
        Ok(mid) =>
            if is_enemy(piece, cell_at(board, mid)) {
                match step_idx(mid, dir) {
                    Ok(land) =>
                        if cell_at(board, land) == empty_cell {
                            Ok({ mid: mid, land: land })
                        } else {
                            Err({})
                        }

                    Err(_) => Err({})
                }
            } else {
                Err({})
            }

        Err(_) => Err({})
    }

## Apply one step or one jump segment: move the piece (crowning a man
## that lands on its king row) and clear a jumped midpoint, which the
## geometry gives exactly — a jump spans 14 or 18 squares, a step 7
## or 9.
apply_segment : Str, U64, U64, U8 -> Str
apply_segment = |board, from, land, piece| {
    landed = if is_man(piece) and king_row_for(piece, land) { king_of(piece) } else { piece }
    moved = set_cell(set_cell(board, from, empty_cell), land, landed)
    delta = if land > from { land - from } else { from - land }
    if delta > 9 {
        set_cell(moved, (from + land) / 2, empty_cell)
    } else {
        moved
    }
}

## Every maximal jump chain for the piece at `index`: a chain ends
## when no further jump exists, or immediately when a man is crowned
## (crowning ends the turn, so a new king never continues a chain).
jump_chains : Str, U64, U8 -> List({ landings : List(U64), captured : List(U64) })
jump_chains = |board, index, piece|
    List.fold(dirs_for(piece), [], |acc, dir|
        match jump_step(board, index, piece, dir) {
            Ok(jump) => {
                crowned = is_man(piece) and king_row_for(piece, jump.land)
                landed_piece = if crowned { king_of(piece) } else { piece }
                board_after = apply_segment(board, index, jump.land, piece)
                if crowned {
                    List.append(acc, { landings: [jump.land], captured: [jump.mid] })
                } else {
                    rest = jump_chains(board_after, jump.land, landed_piece)
                    if List.is_empty(rest) {
                        List.append(acc, { landings: [jump.land], captured: [jump.mid] })
                    } else {
                        List.fold(rest, acc, |inner, tail|
                            List.append(inner, {
                                landings: List.prepend(tail.landings, jump.land),
                                captured: List.prepend(tail.captured, jump.mid),
                            }))
                    }
                }
            }

            Err(_) => acc
        })

## The indices of every piece belonging to a side.
side_indices : Str, Str -> List(U64)
side_indices = |board, side|
    List.fold(
        List.map_with_index(Str.to_utf8(board), |cell, index| { cell: cell, index: index }),
        [],
        |acc, entry|
            if (side == "red" and is_red(entry.cell)) or (side == "black" and is_black(entry.cell)) {
                List.append(acc, entry.index)
            } else {
                acc
            },
    )

## All legal moves for a side. Captures are forced: if any piece of
## the side can jump, only jumps are legal.
legal_moves : Str, Str -> List(Move)
legal_moves = |board, side| {
    indices = side_indices(board, side)
    jumps = List.fold(indices, [], |acc, index|
        List.fold(jump_chains(board, index, cell_at(board, index)), acc, |inner, chain|
            List.append(inner, { from: index, landings: chain.landings, captured: chain.captured })))
    if List.is_empty(jumps) {
        List.fold(indices, [], |acc, index|
            List.fold(steps_from(board, index, cell_at(board, index)), acc, |inner, land|
                List.append(inner, { from: index, landings: [land], captured: [] })))
    } else {
        jumps
    }
}

## Apply a whole move: clear the origin and every jumped piece, place
## the piece on its final landing, crowned if it ends as a man on its
## king row.
apply_move : Str, Move -> Str
apply_move = |board, move| {
    piece = cell_at(board, move.from)
    final = match List.last(move.landings) {
        Ok(land) => land
        Err(_) => move.from
    }
    landed = if is_man(piece) and king_row_for(piece, final) { king_of(piece) } else { piece }
    cleared = List.fold(move.captured, set_cell(board, move.from, empty_cell), |acc, index| set_cell(acc, index, empty_cell))
    set_cell(cleared, final, landed)
}

# --- lifecycle ---------------------------------------------------------------

init : U64 -> Model
init = |seed| {
    rng = if seed == 0 { 2685821657736338717 } else { seed }
    {
        phase: "playing",
        board: start_board,
        cursor: 40,
        selected: 0,
        sel: 0,
        chain: 0,
        rng: rng,
        plies: 0,
        turn: 0,
        red_seat: 0,
        black_seat: 0,
    }
}

## A new game after a win or a loss keeps everyone in their seats.
restart : Model -> Model
restart = |model| { ..init(model.rng), red_seat: model.red_seat, black_seat: model.black_seat }

## The side to move: turn 0 is red, 1 is black.
side_to_move : Model -> Str
side_to_move = |model| if model.turn == 1 { "black" } else { "red" }

other_side : Str -> Str
other_side = |side| if side == "red" { "black" } else { "red" }

owns : Str, U8 -> Bool
owns = |side, cell| if side == "red" { is_red(cell) } else { is_black(cell) }

update : Model, U64, Str -> Model
update = |model, pid, ev|
    match RocJson.decode(ev) {
        Ok(event) =>
            match RocJson.get(event, "cap") {
                Ok(cap_value) =>
                    match RocJson.as_string(cap_value) {
                        Ok(_name) => model
                        Err(_) => input(model, pid, ev)
                    }

                Err(_) => input(model, pid, ev)
            }

        Err(_) => input(model, pid, ev)
    }

## Participant input: raw terminal bytes, decoded into keys. The
## sender may first take a seat; then each key applies only while the
## sender holds the seat of the side to move (or, after a win or a
## loss, any seat, to start a new game). Every other key is dropped,
## so out-of-turn and spectator input leave the model unchanged. Keys
## are checked one at a time: a key after a turn-ending confirm in the
## same event belongs to the other side and is dropped too.
input : Model, U64, Str -> Model
input = |model, pid, ev| {
    keys = Key.parse(ev)
    if pid == 0 {
        model
    } else {
        List.fold(keys, take_seat(model, pid, keys), |acc, key|
            if may_act(acc, pid) {
                apply_key(acc, key)
            } else {
                acc
            })
    }
}

## A participant without a seat takes red if it is open, or black if
## it is open and the input holds J. Anyone else stays a spectator.
take_seat : Model, U64, List(Key) -> Model
take_seat = |model, pid, keys|
    if pid == model.red_seat or pid == model.black_seat {
        model
    } else if model.red_seat == 0 {
        { ..model, red_seat: pid }
    } else if model.black_seat == 0 and List.any(keys, is_join_key) {
        { ..model, black_seat: pid }
    } else {
        model
    }

is_join_key : Key -> Bool
is_join_key = |key|
    match key {
        Char("j") | Char("J") => Bool.True
        _ => Bool.False
    }

## Whether `pid` may act now: it holds the seat of the side to move, or
## the game is over and it holds either seat.
may_act : Model, U64 -> Bool
may_act = |model, pid|
    if model.phase != "playing" {
        pid == model.red_seat or pid == model.black_seat
    } else if model.turn == 1 {
        pid == model.black_seat
    } else {
        pid == model.red_seat
    }

apply_key : Model, Key -> Model
apply_key = |model, key|
    if model.phase == "playing" {
        match key {
            Up | Char("w") | Char("W") => move_cursor(model, 0)
            Right | Char("d") | Char("D") => move_cursor(model, 1)
            Down | Char("s") | Char("S") => move_cursor(model, 2)
            Left | Char("a") | Char("A") => move_cursor(model, 3)
            Enter | Char(" ") => confirm(model)
            _ => model
        }
    } else {
        match key {
            Enter | Char("r") | Char("R") => restart(model)
            _ => model
        }
    }

move_cursor : Model, U64 -> Model
move_cursor = |model, dir| {
    row = model.cursor / 8
    col = model.cursor % 8
    if dir == 0 and row > 0 {
        { ..model, cursor: model.cursor - 8 }
    } else if dir == 2 and row < 7 {
        { ..model, cursor: model.cursor + 8 }
    } else if dir == 3 and col > 0 {
        { ..model, cursor: model.cursor - 1 }
    } else if dir == 1 and col < 7 {
        { ..model, cursor: model.cursor + 1 }
    } else {
        model
    }
}

## Enter/Space: select a piece of the side to move under the cursor,
## deselect it by confirming on it again, or confirm the landing square
## under the cursor. Mid-chain the selection is locked to the jumping
## piece.
confirm : Model -> Model
confirm = |model|
    if model.phase != "playing" {
        restart(model)
    } else if model.selected == 0 {
        side = side_to_move(model)
        if owns(side, cell_at(model.board, model.cursor)) and List.any(legal_moves(model.board, side), |move| move.from == model.cursor) {
            { ..model, selected: 1, sel: model.cursor }
        } else {
            model
        }
    } else if model.cursor == model.sel and model.chain == 0 {
        { ..model, selected: 0 }
    } else {
        candidates = List.keep_if(current_options(model), |move| first_landing(move) == model.cursor)
        match List.first(candidates) {
            Ok(chosen) => {
                piece = cell_at(model.board, model.sel)
                board_after = apply_segment(model.board, model.sel, model.cursor, piece)
                if List.len(chosen.landings) > 1 {
                    { ..model, board: board_after, sel: model.cursor, chain: 1 }
                } else {
                    complete_turn({ ..model, board: board_after, selected: 0, chain: 0, plies: model.plies + 1 })
                }
            }

            Err(_) => model
        }
    }

## The moves the selected piece may still make: mid-chain only the
## continuation jumps, otherwise its full legal moves.
current_options : Model -> List(Move)
current_options = |model|
    if model.chain == 1 {
        piece = cell_at(model.board, model.sel)
        List.map(jump_chains(model.board, model.sel, piece), |chain| {
            from: model.sel,
            landings: chain.landings,
            captured: chain.captured,
        })
    } else {
        List.keep_if(legal_moves(model.board, side_to_move(model)), |move| move.from == model.sel)
    }

first_landing : Move -> U64
first_landing = |move|
    match List.first(move.landings) {
        Ok(land) => land
        Err(_) => move.from
    }

## A human move is on the board. If the other side is pieceless or
## stuck the mover wins ("won" is red's win, "lost" black's). Otherwise
## the turn passes: to the black player if black's seat is taken, or to
## the AI, which replies at once — and if that leaves red pieceless or
## stuck the game is lost.
complete_turn : Model -> Model
complete_turn = |model| {
    mover = side_to_move(model)
    other = other_side(mover)
    winner_phase = if mover == "red" { "won" } else { "lost" }
    next_turn = if other == "black" { 1 } else { 0 }
    if count_side(model.board, other) == 0 or List.is_empty(legal_moves(model.board, other)) {
        { ..model, phase: winner_phase }
    } else if other == "red" or model.black_seat != 0 {
        { ..model, turn: next_turn }
    } else {
        choice = ai_choose(model.board, model.rng)
        board_after = apply_move(model.board, choice.move)
        settled = { ..model, board: board_after, rng: choice.rng, plies: model.plies + 1 }
        if count_side(board_after, "red") == 0 or List.is_empty(legal_moves(board_after, "red")) {
            { ..settled, phase: "lost" }
        } else {
            settled
        }
    }
}

# --- the weak AI ---------------------------------------------------------------

piece_value : U8 -> U64
piece_value = |cell|
    if is_king(cell) {
        300
    } else {
        100
    }

## One ply of greed, nothing more: the material captured by this move,
## a small bonus for crowning, and a whisper for advancing a man.
## Whether the moved piece hangs, whether the move opens a line, what
## the opponent threatens — none of it is considered.
score_move : Str, Move -> U64
score_move = |board, move| {
    captured_value = List.fold(move.captured, 0, |acc, index| acc + piece_value(cell_at(board, index)))
    piece = cell_at(board, move.from)
    final = match List.last(move.landings) {
        Ok(land) => land
        Err(_) => move.from
    }
    crown_bonus = if is_man(piece) and king_row_for(piece, final) { 40 } else { 0 }
    advance = if piece == 98 { (final / 8) * 2 } else { 0 }
    captured_value + crown_bonus + advance
}

## Pick black's move. One draw in four plays a uniformly random legal
## move (the blunder draw). Otherwise the highest one-ply score wins,
## with uniform jitter of 0..40 on every score — enough that the AI
## regularly prefers a worse capture or a pointless advance, and never
## enough to make it look ahead.
ai_choose : Str, U64 -> { move : Move, rng : U64 }
ai_choose = |board, rng| {
    moves = legal_moves(board, "black")
    fallback = { from: 0, landings: [], captured: [] }
    first_move = match List.first(moves) {
        Ok(move) => move
        Err(_) => fallback
    }
    rng_drawn = xorshift(rng)
    if List.is_empty(moves) {
        { move: fallback, rng: rng_drawn }
    } else if rng_drawn % 4 == 0 {
        picked = match List.get(moves, (rng_drawn / 4) % List.len(moves)) {
            Ok(move) => move
            Err(_) => first_move
        }
        { move: picked, rng: rng_drawn }
    } else {
        scored = List.fold(
            moves,
            { rng: rng_drawn, best: first_move, best_score: 0, seen: 0 },
            |acc, move| {
                drawn = xorshift(acc.rng)
                score = score_move(board, move) + (drawn % 41)
                if acc.seen == 0 or score > acc.best_score {
                    { rng: drawn, best: move, best_score: score, seen: acc.seen + 1 }
                } else {
                    { rng: drawn, best: acc.best, best_score: acc.best_score, seen: acc.seen + 1 }
                }
            },
        )
        { move: scored.best, rng: scored.rng }
    }
}

## xorshift64: shifts and xors only, so no arithmetic can overflow.
## (The pinned compiler puts these on the concrete type: U64.bitwise_xor,
## U64.shl_wrap, U64.shr_zf_wrap — there is no Num.bitwise_xor.)
xorshift : U64 -> U64
xorshift = |state| {
    a = U64.bitwise_xor(state, U64.shl_wrap(state, 13))
    b = U64.bitwise_xor(a, U64.shr_zf_wrap(a, 7))
    U64.bitwise_xor(b, U64.shl_wrap(b, 17))
}

# --- rendering ---------------------------------------------------------------

frame_width : U64
frame_width = 28

frame_height : U64
frame_height = 12

plain : { fg : U8, bg : U8, bold : Bool }
plain = { fg: 0, bg: 0, bold: Bool.False }

view : Model -> Str
view = |model| Screen.render_json(frame(model))

## Sessions with a viewport render through `layout`; the game renders
## at its declared dimensions and the host crops, so every mode and
## viewport sees the same frame.
layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

frame : Model -> Screen
frame = |model| {
    hud = "CHECKERS  RED ${count_side(model.board, "red").to_str()}  BLACK ${count_side(model.board, "black").to_str()}"
    base = Screen.put_text(Screen.new(frame_width, frame_height), 0, 0, hud, plain)
    header = Screen.put_text(base, 0, 1, "   a  b  c  d  e  f  g  h", plain)
    ranked = List.fold([0, 1, 2, 3, 4, 5, 6, 7], header, |screen, row|
        Screen.put_text(screen, 0, 2 + row, (8 - row).to_str(), plain))
    cells = List.fold(
        List.map_with_index(Str.to_utf8(model.board), |cell, index| { cell: cell, index: index }),
        ranked,
        |screen, entry|
            Screen.put_text(screen, 2 + (entry.index % 8) * 3, 2 + entry.index / 8, glyph_for(entry.cell, entry.index), style_for(entry.cell)),
    )
    marked = if model.selected == 1 { mark(cells, model.sel, 46) } else { cells }
    cursored = mark(marked, model.cursor, 226)
    status = Screen.put_text(cursored, 0, 10, status_text(model), plain)
    Screen.put_text(status, 0, 11, help_text(model), plain)
}

## o/O are red (the human), x/X are black (the AI); capitals are
## kings. A dot marks an empty dark square.
glyph_for : U8, U64 -> Str
glyph_for = |cell, index|
    if cell == 114 {
        " o "
    } else if cell == 82 {
        " O "
    } else if cell == 98 {
        " x "
    } else if cell == 66 {
        " X "
    } else if (index / 8 + index % 8) % 2 == 1 {
        " . "
    } else {
        "   "
    }

style_for : U8 -> { fg : U8, bg : U8, bold : Bool }
style_for = |cell|
    if is_red(cell) {
        { fg: 203, bg: 0, bold: is_king(cell) }
    } else if is_black(cell) {
        { fg: 245, bg: 0, bold: is_king(cell) }
    } else {
        plain
    }

## Bracket a square: green for the selected piece, yellow for the
## cursor (drawn last, so the cursor wins when they coincide).
mark : Screen, U64, U8 -> Screen
mark = |screen, index, color| {
    style = { fg: color, bg: 0, bold: Bool.True }
    row = index / 8
    col = index % 8
    Screen.put_text(Screen.put_text(screen, 2 + col * 3, 2 + row, "[", style), 4 + col * 3, 2 + row, "]", style)
}

## The bottom line: once red is seated and black's seat is open, it
## tells a second participant how to take black.
help_text : Model -> Str
help_text = |model|
    if model.red_seat != 0 and model.black_seat == 0 {
        "arrows ENTER, J=join black"
    } else {
        "arrows move, ENTER select"
    }

status_text : Model -> Str
status_text = |model|
    if model.black_seat != 0 {
        two_player_status(model)
    } else if model.phase == "won" {
        "YOU WIN! ENTER = new game"
    } else if model.phase == "lost" {
        "AI WINS - ENTER to retry"
    } else if model.chain == 1 {
        "jump again: pick a square"
    } else if model.selected == 1 {
        "selected: pick a landing"
    } else {
        "your move: red to play"
    }

## Two humans: name the side, not "you".
two_player_status : Model -> Str
two_player_status = |model|
    if model.phase == "won" {
        "RED WINS - ENTER = new game"
    } else if model.phase == "lost" {
        "BLACK WINS - ENTER new game"
    } else if model.chain == 1 {
        "${side_to_move(model)}: jump again"
    } else if model.selected == 1 {
        "${side_to_move(model)}: pick a landing"
    } else {
        "${side_to_move(model)} to play"
    }

# --- records, wants, snapshot --------------------------------------------------

records : Model -> Str
records = |model| encode_or_crash(
    RocJson.Object([
        { key: "red", value: RocJson.Number(count_side(model.board, "red").to_str()) },
        { key: "black", value: RocJson.Number(count_side(model.board, "black").to_str()) },
        { key: "phase", value: RocJson.String(model.phase) },
        { key: "plies", value: RocJson.Number(model.plies.to_str()) },
        { key: "turn", value: RocJson.String(side_to_move(model)) },
        { key: "red_seat", value: RocJson.Number(model.red_seat.to_str()) },
        { key: "black_seat", value: RocJson.Number(model.black_seat.to_str()) },
    ]),
)

## Turn-based: the game wants nothing from the server — no tick, no
## requests. Moves arrive as participant input events.
wants : Model -> Str
wants = |_model| "{}"

## The whole model as one JSON object; `restore` reads it back with
## per-field defaults, so a checkpoint the game cannot parse restarts
## the opening position rather than trapping.
snapshot : Model -> Str
snapshot = |model| encode_or_crash(
    RocJson.Object([
        { key: "phase", value: RocJson.String(model.phase) },
        { key: "board", value: RocJson.String(model.board) },
        { key: "cursor", value: RocJson.Number(model.cursor.to_str()) },
        { key: "selected", value: RocJson.Number(model.selected.to_str()) },
        { key: "sel", value: RocJson.Number(model.sel.to_str()) },
        { key: "chain", value: RocJson.Number(model.chain.to_str()) },
        { key: "rng", value: RocJson.Number(model.rng.to_str()) },
        { key: "plies", value: RocJson.Number(model.plies.to_str()) },
        { key: "turn", value: RocJson.Number(model.turn.to_str()) },
        { key: "red_seat", value: RocJson.Number(model.red_seat.to_str()) },
        { key: "black_seat", value: RocJson.Number(model.black_seat.to_str()) },
    ]),
)

restore : Str -> Model
restore = |text|
    match RocJson.decode(text) {
        Err(_) => init(0)
        Ok(event) => {
            fresh = init(0)
            phase = string_field(event, "phase")
            {
                phase: if phase == "won" or phase == "lost" { phase } else { "playing" },
                board: board_field(event),
                cursor: clamp(num_field(event, "cursor", fresh.cursor), 0, 63),
                selected: if num_field(event, "selected", fresh.selected) == 1 { 1 } else { 0 },
                sel: clamp(num_field(event, "sel", fresh.sel), 0, 63),
                chain: if num_field(event, "chain", fresh.chain) == 1 { 1 } else { 0 },
                rng: num_field(event, "rng", fresh.rng),
                plies: num_field(event, "plies", fresh.plies),
                turn: if num_field(event, "turn", fresh.turn) == 1 { 1 } else { 0 },
                red_seat: num_field(event, "red_seat", fresh.red_seat),
                black_seat: num_field(event, "black_seat", fresh.black_seat),
            }
        }
    }

## A restored board must be exactly 64 cells of board bytes; anything
## else falls back to the opening position.
board_field = |event| {
    text = string_field(event, "board")
    bytes = Str.to_utf8(text)
    valid = List.fold(bytes, Bool.True, |acc, byte|
        acc and (byte == empty_cell or byte == 114 or byte == 82 or byte == 98 or byte == 66))
    if List.len(bytes) == 64 and valid {
        text
    } else {
        start_board
    }
}

## Clamp a restored scalar into the range the game can render and
## step without underflowing; snapshots are self-produced, but
## `restore` stays total on hostile text too.
clamp : U64, U64, U64 -> U64
clamp = |value, low, high|
    if value < low {
        low
    } else if value > high {
        high
    } else {
        value
    }

# --- JSON helpers --------------------------------------------------------------

encode_or_crash = |value| match RocJson.encode(value) {
    Ok(text) => text
    Err(_) => crash "checkers attempted to encode an invalid or oversized JSON value"
}

field = |value, key| match RocJson.get(value, key) {
    Ok(found) => found
    Err(_) => RocJson.Null
}

string_field = |value, key| match RocJson.as_string(field(value, key)) {
    Ok(text) => text
    Err(_) => ""
}

num_value = |value| match RocJson.as_number(value) {
    Ok(text) =>
        match U64.from_str(text) {
            Ok(number) => number
            Err(_) => 0
        }

    Err(_) => 0
}

num_field = |value, key, fallback|
    match RocJson.get(value, key) {
        Ok(found) =>
            match RocJson.as_number(found) {
                Ok(_) => num_value(found)
                Err(_) => fallback
            }

        Err(_) => fallback
    }

# --- tests ---------------------------------------------------------------------
#
# (The pinned compiler takes single-expression expects in app files —
# no block form — so scenarios are named defs and the expects assert
# over them.)

first_or_empty : List(Move) -> Move
first_or_empty = |moves|
    match List.first(moves) {
        Ok(move) => move
        Err(_) => { from: 0, landings: [], captured: [] }
    }

expect Str.count_utf8_bytes(start_board) == 64
expect count_side(start_board, "red") == 12 and count_side(start_board, "black") == 12
expect init(0).phase == "playing" and init(0).cursor == 40

# The opening position gives each side the standard seven moves.
expect List.len(legal_moves(start_board, "red")) == 7
expect List.len(legal_moves(start_board, "black")) == 7

# Cursor movement clamps at the board edges.
expect apply_key(init(0), Left).cursor == 40
expect apply_key(init(0), Down).cursor == 48

# A host tick is not a subscription this game holds; it changes nothing.
expect update(init(0), 0, "{\"cap\":\"time.tick\",\"interval\":1000,\"now\":1}") == init(0)

# --- forced capture ------------------------------------------------------------

forced_board = board_from_rows([
    "........",
    "........",
    "....b...",
    "...r....",
    "........",
    "........",
    "........",
    "........",
])

forced_moves = legal_moves(forced_board, "red")
forced_move = first_or_empty(forced_moves)
forced_after = apply_move(forced_board, forced_move)

# The jump exists, so it is the only legal move (captures are forced).
expect List.len(forced_moves) == 1
expect forced_move.landings == [13] and forced_move.captured == [20]
expect cell_at(forced_after, 13) == 114 and cell_at(forced_after, 27) == empty_cell
expect count_side(forced_after, "black") == 0

# --- multi-jump ------------------------------------------------------------------

double_board = board_from_rows([
    "........",
    "........",
    "....b...",
    "........",
    "....b...",
    ".....r..",
    "........",
    "........",
])

double_moves = legal_moves(double_board, "red")
double_move = first_or_empty(double_moves)
double_after = apply_move(double_board, double_move)

expect List.len(double_moves) == 1
expect double_move.landings == [27, 13] and double_move.captured == [36, 20]
expect cell_at(double_after, 13) == 114 and count_side(double_after, "black") == 0

# The same chain driven through the cursor: the first confirm jumps
# once and locks the chain; the second completes it and wins.
chain_start = { phase: "playing", board: double_board, cursor: 27, selected: 1, sel: 45, chain: 0, rng: 1, plies: 0, turn: 0, red_seat: 0, black_seat: 0 }
chain_mid = confirm(chain_start)
chain_done = confirm({ ..chain_mid, cursor: 13 })

expect chain_mid.chain == 1 and chain_mid.sel == 27
expect cell_at(chain_mid.board, 36) == empty_cell and cell_at(chain_mid.board, 20) == 98
expect chain_done.phase == "won" and count_side(chain_done.board, "black") == 0

# Capturing the last black piece through the cursor wins the game.
win_model = { phase: "playing", board: forced_board, cursor: 13, selected: 1, sel: 27, chain: 0, rng: 1, plies: 0, turn: 0, red_seat: 0, black_seat: 0 }
won_game = confirm(win_model)

expect won_game.phase == "won" and cell_at(won_game.board, 13) == 114

# --- crowning and kings ------------------------------------------------------------

king_board = board_from_rows([
    "........",
    "..r.....",
    "........",
    "........",
    "........",
    "........",
    "........",
    "......b.",
])

kinged = apply_move(king_board, { from: 10, landings: [1], captured: [] })

expect cell_at(kinged, 1) == 82

king_back_board = board_from_rows([
    "........",
    "........",
    "........",
    "...R....",
    "........",
    "........",
    "........",
    "........",
])

# A king steps backward as well as forward.
expect List.any(legal_moves(king_back_board, "red"), |move| move.landings == [34])

# --- the weak AI -------------------------------------------------------------------

ai_board = board_from_rows([
    "........",
    "........",
    "....b...",
    ".....r..",
    "........",
    "........",
    "........",
    "........",
])

ai_pick = ai_choose(ai_board, 1)

# Forced to capture, the AI captures.
expect ai_pick.move.captured == [29]
expect count_side(apply_move(ai_board, ai_pick.move), "red") == 0

ai_open = ai_choose(start_board, 7)

# Whatever the jitter or the blunder draw picks, it is a legal move.
expect List.any(legal_moves(start_board, "black"), |move| move == ai_open.move)

# --- a full opening exchange through the keys ---------------------------------------

picked = apply_key(init(0), Enter)
opened = List.fold([Enter, Up, Right, Enter], init(0), apply_key)

expect picked.selected == 1
expect confirm(picked).selected == 0
expect opened.plies == 2 and opened.phase == "playing"
expect cell_at(opened.board, 33) == 114 and cell_at(opened.board, 40) == empty_cell
expect count_side(opened.board, "red") == 12 and count_side(opened.board, "black") == 12

# --- snapshot round-trip --------------------------------------------------------------

mid_game = { phase: "playing", board: double_board, cursor: 13, selected: 1, sel: 27, chain: 1, rng: 12345, plies: 9, turn: 0, red_seat: 4, black_seat: 6 }

expect restore(snapshot(mid_game)) == mid_game
expect restore("not json at all") == init(0)

# --- seats and turn enforcement (#15111) ----------------------------------------------
#
# Participants 7 (red) and 9 (black) play; 11 is a spectator.

seat_red = update(init(0), 7, "\r")

# The first participant to type takes red, and the key still applies.
expect seat_red.red_seat == 7 and seat_red.black_seat == 0 and seat_red.selected == 1

# The host never types: participant-0 input changes nothing.
expect update(init(0), 0, "\r") == init(0)

# Without J, a second participant is a spectator: its input is dropped
# and the model is unchanged.
expect update(seat_red, 9, "\r") == seat_red
expect update(seat_red, 9, "wd\r") == seat_red

# Single-player through `update` plays exactly as through the keys: the
# AI still answers red's move inside the same update.
solo = List.fold(["w", "d", "\r"], seat_red, |acc, ev| update(acc, 7, ev))

expect solo.plies == 2 and solo.turn == 0 and solo.board == opened.board

# J takes the open black seat and retires the AI; nothing else changes.
seat_both = update(seat_red, 9, "j")

expect seat_both.black_seat == 9 and { ..seat_both, black_seat: 0 } == seat_red

# Red moves 40 -> 33. The turn passes to black and the AI does not
# answer (its PRNG is untouched).
red_moved = List.fold(["w", "d", "\r"], seat_both, |acc, ev| update(acc, 7, ev))

expect red_moved.turn == 1 and red_moved.plies == 1 and red_moved.rng == seat_both.rng
expect cell_at(red_moved.board, 33) == 114 and count_side(red_moved.board, "black") == 12

# Out of turn: red's keys, and a spectator's, are dropped with the
# model unchanged.
expect update(red_moved, 7, "\r") == red_moved
expect update(red_moved, 7, "sa\r") == red_moved
expect update(red_moved, 11, "j\r") == red_moved

# Black moves 17 -> 24 in one event; the turn passes back to red.
black_moved = update(red_moved, 9, "ww\rsa\r")

expect black_moved.turn == 0 and black_moved.plies == 2
expect cell_at(black_moved.board, 24) == 98 and cell_at(black_moved.board, 17) == empty_cell

# A key after a turn-ending confirm in the same event belongs to the
# other side and is dropped. Red walks the cursor 24 -> 42, moves
# 42 -> 35, and its trailing "s" does not move the cursor off 35.
red_then_extra = update(black_moved, 7, "ssdd\rwd\rs")

expect red_then_extra.turn == 1 and red_then_extra.plies == 3
expect cell_at(red_then_extra.board, 35) == 114 and red_then_extra.cursor == 35

# Black wins by taking red's last piece; only a seated player restarts,
# and the seats survive the new game.
black_win_board = board_from_rows([
    "........",
    "........",
    "....b...",
    "...r....",
    "........",
    "........",
    "........",
    "........",
])

black_to_win = { phase: "playing", board: black_win_board, cursor: 34, selected: 1, sel: 20, chain: 0, rng: 1, plies: 4, turn: 1, red_seat: 7, black_seat: 9 }
black_won = update(black_to_win, 9, "\r")

expect update(black_to_win, 7, "\r") == black_to_win
expect black_won.phase == "lost" and count_side(black_won.board, "red") == 0
expect status_text(black_won) == "BLACK WINS - ENTER new game"
expect update(black_won, 11, "\r") == black_won

rematch = update(black_won, 7, "\r")

expect rematch.phase == "playing" and rematch.turn == 0 and rematch.red_seat == 7 and rematch.black_seat == 9

# A snapshot from before seats existed restores with open seats and red
# to move.
expect restore("{\"phase\":\"playing\",\"board\":\"${start_board}\",\"plies\":3}").turn == 0
expect restore("{\"phase\":\"playing\",\"plies\":3}").red_seat == 0
