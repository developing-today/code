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
# Two players over the id network (slice 3 of #14862): the world
# holds two seats, red and black. A participant's first key takes the
# first open seat, red before black; with both seats held, a third
# participant only watches. Turn order is the world's rule, not the
# screen's: keys from the side not to move, from a watcher, or from
# the host change nothing. An open seat is played by the weak AI, so
# one player alone gets the single-player game unchanged, and a player
# who leaves hands their side to the AI at once instead of stalling the
# game. Join and leave come from the `players` subscription; a seat
# shows the name its player joined with.
#
# Later slices of #14862: the three-machine demonstration.

Model : {
    phase : Str,
    board : Str,
    cursor : U64,
    selected : U64,
    sel : U64,
    chain : U64,
    rng : U64,
    plies : U64,
    turn : Str,
    red : Seat,
    black : Seat,
    roster : List(Seat),
    note : Str,
}

## A seat, or a roster entry: a participant and the name they joined
## with. Participant 0 is the host, never a player, so a seat with pid
## 0 is open and the weak AI plays it.
Seat : { pid : U64, name : Str }

open_seat : Seat
open_seat = { pid: 0, name: "" }

## At most this many online participants are remembered for names.
max_roster : U64
max_roster = 32

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
        turn: "red",
        red: open_seat,
        black: open_seat,
        roster: [],
        note: "",
    }
}

## A new game on the same seats. If only one side has a player and the
## other side moves first, the AI opens at once.
restart : Model -> Model
restart = |model| ai_fill({ ..init(model.rng), red: model.red, black: model.black, roster: model.roster })

update : Model, U64, Str -> Model
update = |model, pid, ev|
    match RocJson.decode(ev) {
        Ok(event) =>
            match RocJson.get(event, "cap") {
                Ok(cap_value) =>
                    match RocJson.as_string(cap_value) {
                        Ok(name) => if pid == 0 and name == "players" { presence(model, event) } else { model }
                        Err(_) => input(model, pid, ev)
                    }

                Err(_) => input(model, pid, ev)
            }

        Err(_) => input(model, pid, ev)
    }

## Participant input: raw terminal bytes, decoded into keys. The host
## (participant 0) never plays. A participant without a seat takes the
## first open one; with both seats held their keys change nothing, and
## so do keys from the side not to move. A key that ends the player's
## turn ends their input: the rest of the event cannot move for the
## other side.
input : Model, U64, Str -> Model
input = |model, pid, ev|
    if pid == 0 {
        model
    } else {
        seated = claim(model, pid)
        side = side_of(seated, pid)
        if side == "" {
            model
        } else if seated.phase == "playing" and side != seated.turn {
            seated
        } else {
            List.fold(Key.parse(ev), seated, |current, key|
                if current.phase == "playing" and current.turn != side {
                    current
                } else {
                    apply_key(current, key)
                })
        }
    }

## Seat a participant in the first open seat, red before black. A
## participant already seated, or arriving with both seats held, leaves
## the seats as they are.
claim : Model, U64 -> Model
claim = |model, pid|
    if model.red.pid == pid or model.black.pid == pid {
        model
    } else if model.red.pid == 0 {
        { ..model, red: { pid: pid, name: roster_name(model, pid) } }
    } else if model.black.pid == 0 {
        { ..model, black: { pid: pid, name: roster_name(model, pid) } }
    } else {
        model
    }

side_of : Model, U64 -> Str
side_of = |model, pid|
    if pid != 0 and model.red.pid == pid {
        "red"
    } else if pid != 0 and model.black.pid == pid {
        "black"
    } else {
        ""
    }

seat_of : Model, Str -> Seat
seat_of = |model, side| if side == "red" { model.red } else { model.black }

other_side : Str -> Str
other_side = |side| if side == "red" { "black" } else { "red" }

roster_name : Model, U64 -> Str
roster_name = |model, pid|
    match List.first(List.keep_if(model.roster, |entry| entry.pid == pid)) {
        Ok(entry) => entry.name
        Err(_) => "p${pid.to_str()}"
    }

# --- presence ------------------------------------------------------------------

## Host presence events from the `players` subscription:
## `{"cap":"players","event":"joined"|"left","participant":{"id":..,"name":..}}`.
presence = |model, event| {
    part = field(event, "participant")
    pid = num_field(part, "id", 0)
    kind = string_field(event, "event")
    if pid == 0 {
        model
    } else if kind == "joined" {
        joined(model, pid, sanitize(string_field(part, "name")))
    } else if kind == "left" {
        left(model, pid)
    } else {
        model
    }
}

## Remember the participant's name; a seat they already hold shows it.
joined : Model, U64, Str -> Model
joined = |model, pid, name| {
    shown = if name == "" { "p${pid.to_str()}" } else { name }
    others = List.keep_if(model.roster, |entry| entry.pid != pid)
    kept = if List.len(others) >= max_roster { List.drop_first(others, 1) } else { others }
    {
        ..model,
        roster: List.append(kept, { pid: pid, name: shown }),
        red: rename(model.red, pid, shown),
        black: rename(model.black, pid, shown),
    }
}

rename : Seat, U64, Str -> Seat
rename = |seat, pid, name| if seat.pid == pid { { pid: pid, name: name } } else { seat }

## A player who leaves opens their seat and says so on the board; if it
## was their move and the other side still has a player, the AI moves
## for them at once, so the game neither stalls nor loses its place.
left : Model, U64 -> Model
left = |model, pid| {
    base = { ..model, roster: List.keep_if(model.roster, |entry| entry.pid != pid) }
    if model.red.pid == pid {
        ai_fill({ ..base, red: open_seat, note: "red left: AI plays red" })
    } else if model.black.pid == pid {
        ai_fill({ ..base, black: open_seat, note: "black left: AI plays black" })
    } else {
        base
    }
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
        if is_side(cell_at(model.board, model.cursor), model.turn) and List.any(legal_moves(model.board, model.turn), |move| move.from == model.cursor) {
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
        List.keep_if(legal_moves(model.board, model.turn), |move| move.from == model.sel)
    }

is_side : U8, Str -> Bool
is_side = |cell, side| if side == "red" { is_red(cell) } else { is_black(cell) }

first_landing : Move -> U64
first_landing = |move|
    match List.first(move.landings) {
        Ok(land) => land
        Err(_) => move.from
    }

## A player's move is on the board and the turn passes. If the side
## now to move is pieceless or stuck, the mover wins. If that side has
## no player, the AI replies at once. `phase` is red's result: "won"
## when red wins, "lost" when black does.
complete_turn : Model -> Model
complete_turn = |model| {
    mover = model.turn
    passed = { ..model, turn: other_side(mover), note: "" }
    if side_beaten(passed.board, passed.turn) {
        { ..passed, phase: win_phase(mover) }
    } else if seat_of(passed, passed.turn).pid == 0 {
        ai_turn(passed)
    } else {
        passed
    }
}

side_beaten : Str, Str -> Bool
side_beaten = |board, side| count_side(board, side) == 0 or List.is_empty(legal_moves(board, side))

win_phase : Str -> Str
win_phase = |side| if side == "red" { "won" } else { "lost" }

## The weak AI moves for the side to move; mid-chain it must finish the
## chain its piece started.
ai_turn : Model -> Model
ai_turn = |model| {
    side = model.turn
    moves =
        if model.chain == 1 {
            piece = cell_at(model.board, model.sel)
            List.map(jump_chains(model.board, model.sel, piece), |chain| { from: model.sel, landings: chain.landings, captured: chain.captured })
        } else {
            legal_moves(model.board, side)
        }
    if List.is_empty(moves) {
        model
    } else {
        choice = ai_pick(model.board, model.rng, moves)
        board_after = apply_move(model.board, choice.move)
        settled = { ..model, board: board_after, rng: choice.rng, plies: model.plies + 1, turn: other_side(side), selected: 0, chain: 0 }
        if side_beaten(board_after, settled.turn) {
            { ..settled, phase: win_phase(side) }
        } else {
            settled
        }
    }
}

## When the side to move has no player but the other side has one, the
## AI moves now (after a leave, or opening a restarted game).
ai_fill : Model -> Model
ai_fill = |model|
    if model.phase == "playing" and seat_of(model, model.turn).pid == 0 and seat_of(model, other_side(model.turn)).pid != 0 {
        ai_turn(model)
    } else {
        model
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
    advance = if piece == 98 { (final / 8) * 2 } else if piece == 114 { (7 - final / 8) * 2 } else { 0 }
    captured_value + crown_bonus + advance
}

## Pick a move for `side`. One draw in four plays a uniformly random
## legal move (the blunder draw). Otherwise the highest one-ply score
## wins, with uniform jitter of 0..40 on every score — enough that the
## AI regularly prefers a worse capture or a pointless advance, and
## never enough to make it look ahead.
ai_choose : Str, U64, Str -> { move : Move, rng : U64 }
ai_choose = |board, rng, side| ai_pick(board, rng, legal_moves(board, side))

ai_pick : Str, U64, List(Move) -> { move : Move, rng : U64 }
ai_pick = |board, rng, moves| {
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
frame_height = 13

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
    hint = if model.note == "" { "arrows move, ENTER select" } else { model.note }
    noted = Screen.put_text(status, 0, 11, hint, plain)
    Screen.put_text(noted, 0, 12, "R:${short(seat_label(model.red))}  B:${short(seat_label(model.black))}", plain)
}

## Who plays a side: the seated player's name, or the AI.
seat_label : Seat -> Str
seat_label = |seat| if seat.pid == 0 { "AI" } else { seat.name }

short : Str -> Str
short = |text|
    match Str.from_utf8(List.take_first(Str.to_utf8(text), 9)) {
        Ok(cut) => cut
        Err(_) => ""
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

status_text : Model -> Str
status_text = |model| {
    versus = model.red.pid != 0 and model.black.pid != 0
    black_solo = model.black.pid != 0 and model.red.pid == 0
    if model.phase == "won" {
        if versus { "RED WINS! ENTER = new game" } else if black_solo { "AI WINS - ENTER to retry" } else { "YOU WIN! ENTER = new game" }
    } else if model.phase == "lost" {
        if versus { "BLACK WINS! ENTER = new game" } else if black_solo { "YOU WIN! ENTER = new game" } else { "AI WINS - ENTER to retry" }
    } else if model.chain == 1 {
        "jump again: pick a square"
    } else if model.selected == 1 {
        "selected: pick a landing"
    } else if versus or black_solo {
        "${model.turn} to play"
    } else {
        "your move: red to play"
    }
}

# --- records, wants, snapshot --------------------------------------------------

records : Model -> Str
records = |model| encode_or_crash(
    RocJson.Object([
        { key: "red", value: RocJson.Number(count_side(model.board, "red").to_str()) },
        { key: "black", value: RocJson.Number(count_side(model.board, "black").to_str()) },
        { key: "phase", value: RocJson.String(model.phase) },
        { key: "plies", value: RocJson.Number(model.plies.to_str()) },
        { key: "turn", value: RocJson.String(model.turn) },
        { key: "board", value: RocJson.String(model.board) },
        { key: "red_player", value: RocJson.String(seat_label(model.red)) },
        { key: "black_player", value: RocJson.String(seat_label(model.black)) },
    ]),
)

## Turn-based: no tick and no requests. The game subscribes to
## `players` so seats show names and a player who leaves opens their
## seat to the AI. Moves arrive as participant input events.
wants : Model -> Str
wants = |_model| "{\"v\":1,\"subscribe\":[\"players\"],\"requests\":[]}"

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
        { key: "turn", value: RocJson.String(model.turn) },
        { key: "red_pid", value: RocJson.Number(model.red.pid.to_str()) },
        { key: "red_name", value: RocJson.String(model.red.name) },
        { key: "black_pid", value: RocJson.Number(model.black.pid.to_str()) },
        { key: "black_name", value: RocJson.String(model.black.name) },
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
                turn: if string_field(event, "turn") == "black" { "black" } else { "red" },
                red: restored_seat(event, "red_pid", "red_name"),
                black: restored_seat(event, "black_pid", "black_name"),
                roster: [],
                note: "",
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

## A restored seat keeps its participant and a sanitized name; an open
## seat stays open whatever name the text carries.
restored_seat = |event, pid_key, name_key| {
    pid = num_field(event, pid_key, 0)
    if pid == 0 { open_seat } else { { pid: pid, name: sanitize(string_field(event, name_key)) } }
}

## Names are drawn on the board and written into records, so only plain
## name bytes survive, at most 16 of them.
sanitize : Str -> Str
sanitize = |name|
    match Str.from_utf8(List.take_first(List.keep_if(Str.to_utf8(name), is_name_byte), 16)) {
        Ok(text) => text
        Err(_) => ""
    }

is_name_byte : U8 -> Bool
is_name_byte = |b|
    (b >= 48 and b <= 57) or (b >= 65 and b <= 90) or (b >= 97 and b <= 122) or b == 45 or b == 95

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
chain_start = { ..init(1), board: double_board, cursor: 27, selected: 1, sel: 45 }
chain_mid = confirm(chain_start)
chain_done = confirm({ ..chain_mid, cursor: 13 })

expect chain_mid.chain == 1 and chain_mid.sel == 27
expect cell_at(chain_mid.board, 36) == empty_cell and cell_at(chain_mid.board, 20) == 98
expect chain_done.phase == "won" and count_side(chain_done.board, "black") == 0

# Capturing the last black piece through the cursor wins the game.
win_model = { ..init(1), board: forced_board, cursor: 13, selected: 1, sel: 27 }
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

ai_pick_black = ai_choose(ai_board, 1, "black")

# Forced to capture, the AI captures.
expect ai_pick_black.move.captured == [29]
expect count_side(apply_move(ai_board, ai_pick_black.move), "red") == 0

ai_open = ai_choose(start_board, 7, "black")

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

mid_game = { ..init(12345), board: double_board, cursor: 13, selected: 1, sel: 27, chain: 1, plies: 9 }

expect restore(snapshot(mid_game)) == mid_game
expect restore("not json at all") == init(0)

# --- seats and turns (slice 3 of #14862) ----------------------------------------------
#
# Participant 7 joins as ann and 9 as bo. "a" is a cursor move that
# clamps at the left edge, so it seats a player without changing the
# board; " " confirms; w/a/s/d move the cursor.

joined_event : U64, Str -> Str
joined_event = |pid, name| "{\"cap\":\"players\",\"event\":\"joined\",\"participant\":{\"id\":${pid.to_str()},\"name\":\"${name}\"}}"

left_event : U64 -> Str
left_event = |pid| "{\"cap\":\"players\",\"event\":\"left\",\"participant\":{\"id\":${pid.to_str()},\"name\":\"x\"}}"

lobby = update(update(init(0), 0, joined_event(7, "ann")), 0, joined_event(9, "bo"))
red_seated = update(lobby, 7, "a")
both_seated = update(red_seated, 9, "a")

expect List.len(lobby.roster) == 2 and lobby.red.pid == 0
expect red_seated.red == { pid: 7, name: "ann" } and red_seated.black.pid == 0
expect both_seated.black == { pid: 9, name: "bo" } and both_seated.board == start_board and both_seated.turn == "red"

# Out of turn, a watcher, the host and a forged presence event from a
# participant all change nothing.
expect update(both_seated, 9, " ") == both_seated
expect update(both_seated, 9, "w") == both_seated
expect update(both_seated, 11, " wd ") == both_seated
expect update(both_seated, 0, " wd ") == both_seated
expect update(both_seated, 9, left_event(7)) == both_seated

# Red moves 40 to 33; with black seated the AI does not reply, and the
# rest of red's event cannot move for black.
red_moved = update(both_seated, 7, " wd ")
red_moved_extra = update(both_seated, 7, " wd ww sd ")

expect red_moved.turn == "black" and red_moved.plies == 1
expect cell_at(red_moved.board, 33) == 114 and cell_at(red_moved.board, 40) == empty_cell
expect red_moved_extra == red_moved
expect update(red_moved, 7, " ") == red_moved

# Black answers 17 to 26 and the turn comes back to red.
black_moved = update(red_moved, 9, "ww sd ")

expect black_moved.turn == "red" and black_moved.plies == 2
expect cell_at(black_moved.board, 26) == 98 and cell_at(black_moved.board, 17) == empty_cell

# A third participant still only watches once both seats are held.
expect update(black_moved, 11, "a") == black_moved

# Black leaves on red's move: the seat opens and the board says so;
# nothing moves until red does, then the AI answers for black.
black_gone = update(black_moved, 0, left_event(9))

expect black_gone.black == open_seat and black_gone.board == black_moved.board
expect black_gone.note == "black left: AI plays black" and List.len(black_gone.roster) == 1

# Red leaves on red's move with black still seated: the AI moves for
# red at once and the turn passes to black.
red_gone = update(black_moved, 0, left_event(7))

expect red_gone.red == open_seat and red_gone.turn == "black" and red_gone.plies == 3

# A returning player takes the open seat with their first key.
red_back = update(update(red_gone, 0, joined_event(12, "cy")), 12, "a")

expect red_back.red == { pid: 12, name: "cy" } and red_back.turn == "black"

# One player alone gets the single-player game: the AI answers at once.
solo = update(update(init(0), 7, "a"), 7, " wd ")

expect solo.turn == "red" and solo.plies == 2 and solo.black.pid == 0

# Records name the side to move and who plays each side.
expect Str.contains(records(both_seated), "\"red_player\":\"ann\"") and Str.contains(records(both_seated), "\"black_player\":\"bo\"")
expect Str.contains(records(solo), "\"black_player\":\"AI\"")

# The board is a record too, so every participant can check it.
expect Str.contains(records(both_seated), "\"board\":\"${start_board}\"")

# Seats survive a snapshot; the roster is rebuilt from presence.
expect restore(snapshot(black_moved)) == { ..black_moved, roster: [] }
expect restore(snapshot({ ..both_seated, red: { pid: 7, name: "a\"b" } })).red == { pid: 7, name: "ab" }
