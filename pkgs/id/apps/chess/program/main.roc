app [program] { pf: platform "../../../examples/roc-world/platform.roc" }

import pf.RocJson
import pf.Screen
import pf.Key

# Chess, as a Roc world program (slice 2 of #14862).
#
# The human plays white (bottom, moving toward row 0) against a
# deliberately weak AI playing black. Full chess rules as pure
# functions of (model, event): legal move generation including
# castling, en passant, promotion, check, checkmate, and stalemate.
# Board state snapshots and restores through the world ABI like the
# other apps.
#
# The AI is weak on purpose, same doctrine as checkers (slice 1): it
# looks exactly one ply ahead, scores a move only by the material it
# captures this turn plus a small promotion/advance bonus, adds
# uniform jitter to every score, and one draw in four ignores the
# scores entirely and plays a uniformly random legal move. It never
# checks whether the piece it just moved can be recaptured. A
# beginner is hampered; a competent child-level player beats it
# easily. No search, no opening book, no engine.
#
# The AI replies inside the same `update` that completes the human
# turn, so at rest it is always white to move. Its randomness comes
# from a xorshift PRNG carried in the model, and the journal replays
# a game exactly.
#
# The frame is data: `view`/`layout` render a `Screen` and the shell
# paints it in whichever presentation mode the session negotiated
# (alternate, primary, or low-write text, #14936/#14937). The game
# never changes when the mode does.
#
# Controls: arrow keys or WASD move the cursor, Enter (or Space)
# selects a piece and confirms a landing square. When a pawn reaches
# the last rank the game asks for the promotion piece: Left/Right
# (or A/D) cycle Q R B N, the letter keys pick directly, Enter
# confirms. Enter (or R) starts a new game after a win, loss, or
# draw.
#
# Later slices of #14862: id-network multiplayer (host/join and
# central server), and the three-machine demonstration.

Model : {
    phase : Str,
    board : Str,
    cursor : U64,
    selected : U64,
    sel : U64,
    castling : Str,
    ep : U64,
    promo_from : U64,
    promo_to : U64,
    promo_idx : U64,
    rng : U64,
    plies : U64,
}

## One chess move. `captured` is the index of the captured piece, or
## 64 when the move captures nothing (for en passant it is the index
## of the captured pawn, which is not the landing square). `promo` is
## the piece byte to promote to, or 0. `kind`: 0 = normal,
## 1 = castle short, 2 = castle long, 3 = en passant capture.
Move : { from : U64, to : U64, captured : U64, promo : U8, kind : U64 }

## The move-generation state: the board, the castling rights still
## available (a subsequence of "KQkq"), and the en-passant target
## square (the square behind a pawn that just double-pushed), or 64.
State : { board : Str, castling : Str, ep : U64 }

program = { init, update, view, layout, records, wants, snapshot, restore }

# --- board basics ------------------------------------------------------------
#
# The board is a 64-byte string, row-major, row 0 at the top (rank 8,
# black home rows) and row 7 at the bottom (rank 1, white home).
# Bytes: '.' empty; white pieces are uppercase 'P' 'N' 'B' 'R' 'Q'
# 'K', black pieces the lowercase twins.

empty_cell : U8
empty_cell = 46

no_square : U64
no_square = 64

board_from_rows : List(Str) -> Str
board_from_rows = |rows| List.fold(rows, "", |acc, row| Str.concat(acc, row))

start_board : Str
start_board = board_from_rows([
    "rnbqkbnr",
    "pppppppp",
    "........",
    "........",
    "........",
    "........",
    "PPPPPPPP",
    "RNBQKBNR",
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

is_white : U8 -> Bool
is_white = |cell| cell == 80 or cell == 78 or cell == 66 or cell == 82 or cell == 81 or cell == 75

is_black : U8 -> Bool
is_black = |cell| cell == 112 or cell == 110 or cell == 98 or cell == 114 or cell == 113 or cell == 107

is_enemy : U8, U8 -> Bool
is_enemy = |piece, cell|
    if is_white(piece) {
        is_black(cell)
    } else {
        is_white(cell)
    }

count_side : Str, Str -> U64
count_side = |board, side|
    List.fold(Str.to_utf8(board), 0, |acc, cell|
        if (side == "white" and is_white(cell)) or (side == "black" and is_black(cell)) {
            acc + 1
        } else {
            acc
        })

## The indices of every piece belonging to a side.
side_indices : Str, Str -> List(U64)
side_indices = |board, side|
    List.fold(
        List.map_with_index(Str.to_utf8(board), |cell, index| { cell: cell, index: index }),
        [],
        |acc, entry|
            if (side == "white" and is_white(entry.cell)) or (side == "black" and is_black(entry.cell)) {
                List.append(acc, entry.index)
            } else {
                acc
            },
    )

## Eight directions as tags: 0 = up-left, 1 = up, 2 = up-right,
## 3 = left, 4 = right, 5 = down-left, 6 = down, 7 = down-right
## (up = toward row 0, white direction of travel).
step_idx : U64, U64 -> Try(U64, {})
step_idx = |index, dir| {
    row = index / 8
    col = index % 8
    if dir == 0 and row > 0 and col > 0 {
        Ok(index - 9)
    } else if dir == 1 and row > 0 {
        Ok(index - 8)
    } else if dir == 2 and row > 0 and col < 7 {
        Ok(index - 7)
    } else if dir == 3 and col > 0 {
        Ok(index - 1)
    } else if dir == 4 and col < 7 {
        Ok(index + 1)
    } else if dir == 5 and row < 7 and col > 0 {
        Ok(index + 7)
    } else if dir == 6 and row < 7 {
        Ok(index + 8)
    } else if dir == 7 and row < 7 and col < 7 {
        Ok(index + 9)
    } else {
        Err({})
    }
}

## Knight jumps as tags 0..7: (-2,-1) (-2,+1) (-1,-2) (-1,+2)
## (+1,-2) (+1,+2) (+2,-1) (+2,+1) in (row, col) deltas.
knight_step : U64, U64 -> Try(U64, {})
knight_step = |index, tag| {
    row = index / 8
    col = index % 8
    if tag == 0 and row >= 2 and col >= 1 {
        Ok(index - 17)
    } else if tag == 1 and row >= 2 and col <= 6 {
        Ok(index - 15)
    } else if tag == 2 and row >= 1 and col >= 2 {
        Ok(index - 10)
    } else if tag == 3 and row >= 1 and col <= 5 {
        Ok(index - 6)
    } else if tag == 4 and row <= 6 and col >= 2 {
        Ok(index + 6)
    } else if tag == 5 and row <= 6 and col <= 5 {
        Ok(index + 10)
    } else if tag == 6 and row <= 5 and col >= 1 {
        Ok(index + 15)
    } else if tag == 7 and row <= 5 and col <= 6 {
        Ok(index + 17)
    } else {
        Err({})
    }
}

diag_dirs : List(U64)
diag_dirs = [0, 2, 5, 7]

straight_dirs : List(U64)
straight_dirs = [1, 3, 4, 6]

all_dirs : List(U64)
all_dirs = [0, 1, 2, 3, 4, 5, 6, 7]

# --- move generation ---------------------------------------------------------

plain_move : U64, U64 -> Move
plain_move = |from, to| { from: from, to: to, captured: no_square, promo: 0, kind: 0 }

capture_move : U64, U64 -> Move
capture_move = |from, to| { from: from, to: to, captured: to, promo: 0, kind: 0 }

## One step in a direction: a quiet move onto an empty square or a
## capture of an enemy, never onto a friendly piece.
step_moves : Str, U64, U8 -> List(Move)
step_moves = |board, index, piece|
    List.fold(all_dirs, [], |acc, dir|
        match step_idx(index, dir) {
            Ok(land) => {
                cell = cell_at(board, land)
                if cell == empty_cell {
                    List.append(acc, plain_move(index, land))
                } else if is_enemy(piece, cell) {
                    List.append(acc, capture_move(index, land))
                } else {
                    acc
                }
            }

            Err(_) => acc
        })

knight_moves : Str, U64, U8 -> List(Move)
knight_moves = |board, index, piece|
    List.fold([0, 1, 2, 3, 4, 5, 6, 7], [], |acc, tag|
        match knight_step(index, tag) {
            Ok(land) => {
                cell = cell_at(board, land)
                if cell == empty_cell {
                    List.append(acc, plain_move(index, land))
                } else if is_enemy(piece, cell) {
                    List.append(acc, capture_move(index, land))
                } else {
                    acc
                }
            }

            Err(_) => acc
        })

## Slide along each direction until the edge or the first piece:
## empty squares are quiet moves, the first enemy is a capture, a
## friendly piece just blocks.
slide_moves : Str, U64, U8, List(U64) -> List(Move)
slide_moves = |board, index, piece, dirs|
    List.fold(dirs, [], |acc, dir| slide_dir(board, index, index, piece, dir, acc))

slide_dir : Str, U64, U64, U8, U64, List(Move) -> List(Move)
slide_dir = |board, from, pos, piece, dir, acc|
    match step_idx(pos, dir) {
        Ok(land) => {
            cell = cell_at(board, land)
            if cell == empty_cell {
                slide_dir(board, from, land, piece, dir, List.append(acc, plain_move(from, land)))
            } else if is_enemy(piece, cell) {
                List.append(acc, capture_move(from, land))
            } else {
                acc
            }
        }

        Err(_) => acc
    }

## The four promotion moves for a pawn arriving on its last rank.
promo_moves : U64, U64, U64, Bool -> List(Move)
promo_moves = |from, to, captured, white| {
    bytes = if white { [81, 82, 66, 78] } else { [113, 114, 98, 110] }
    List.map(bytes, |promo| { from: from, to: to, captured: captured, promo: promo, kind: 0 })
}

pawn_moves : State, U64, U8 -> List(Move)
pawn_moves = |state, index, piece| {
    board = state.board
    white = is_white(piece)
    forward = if white { 1 } else { 6 }
    cap_dirs = if white { [0, 2] } else { [5, 7] }
    start_row = if white { 6 } else { 1 }
    last_row = if white { 0 } else { 7 }
    quiet =
        match step_idx(index, forward) {
            Ok(one) =>
                if cell_at(board, one) == empty_cell {
                    if one / 8 == last_row {
                        promo_moves(index, one, no_square, white)
                    } else {
                        rest =
                            if index / 8 == start_row {
                                match step_idx(one, forward) {
                                    Ok(two) =>
                                        if cell_at(board, two) == empty_cell {
                                            [plain_move(index, two)]
                                        } else {
                                            []
                                        }

                                    Err(_) => []
                                }
                            } else {
                                []
                            }
                        List.prepend(rest, plain_move(index, one))
                    }
                } else {
                    []
                }

            Err(_) => []
        }
    captures = List.fold(cap_dirs, [], |acc, dir|
        match step_idx(index, dir) {
            Ok(land) => {
                cell = cell_at(board, land)
                if is_enemy(piece, cell) {
                    if land / 8 == last_row {
                        List.concat(acc, promo_moves(index, land, land, white))
                    } else {
                        List.append(acc, capture_move(index, land))
                    }
                } else if land == state.ep and cell == empty_cell {
                    captured = if white { land + 8 } else { land - 8 }
                    List.append(acc, { from: index, to: land, captured: captured, promo: 0, kind: 3 })
                } else {
                    acc
                }
            }

            Err(_) => acc
        })
    List.concat(quiet, captures)
}

rights_has : Str, U8 -> Bool
rights_has = |rights, byte| List.any(Str.to_utf8(rights), |b| b == byte)

rights_remove : Str, U8 -> Str
rights_remove = |rights, byte|
    match Str.from_utf8(List.keep_if(Str.to_utf8(rights), |b| b != byte)) {
        Ok(text) => text
        Err(_) => rights
    }

## Castling: the rights are held, the squares between king and rook
## are empty, and neither the king nor the squares it crosses are
## attacked. (The legality filter would catch the destination; the
## transit and in-check conditions are stated here explicitly.)
castle_moves : State, U64, U8 -> List(Move)
castle_moves = |state, index, piece| {
    board = state.board
    if piece == 75 and index == 60 {
        short =
            if rights_has(state.castling, 75)
                and cell_at(board, 63) == 82
                and cell_at(board, 61) == empty_cell
                and cell_at(board, 62) == empty_cell
                and attacked(board, 60, Bool.False) == Bool.False
                and attacked(board, 61, Bool.False) == Bool.False
                and attacked(board, 62, Bool.False) == Bool.False {
                [{ from: 60, to: 62, captured: no_square, promo: 0, kind: 1 }]
            } else {
                []
            }
        long =
            if rights_has(state.castling, 81)
                and cell_at(board, 56) == 82
                and cell_at(board, 57) == empty_cell
                and cell_at(board, 58) == empty_cell
                and cell_at(board, 59) == empty_cell
                and attacked(board, 60, Bool.False) == Bool.False
                and attacked(board, 59, Bool.False) == Bool.False
                and attacked(board, 58, Bool.False) == Bool.False {
                [{ from: 60, to: 58, captured: no_square, promo: 0, kind: 2 }]
            } else {
                []
            }
        List.concat(short, long)
    } else if piece == 107 and index == 4 {
        short =
            if rights_has(state.castling, 107)
                and cell_at(board, 7) == 114
                and cell_at(board, 5) == empty_cell
                and cell_at(board, 6) == empty_cell
                and attacked(board, 4, Bool.True) == Bool.False
                and attacked(board, 5, Bool.True) == Bool.False
                and attacked(board, 6, Bool.True) == Bool.False {
                [{ from: 4, to: 6, captured: no_square, promo: 0, kind: 1 }]
            } else {
                []
            }
        long =
            if rights_has(state.castling, 113)
                and cell_at(board, 0) == 114
                and cell_at(board, 1) == empty_cell
                and cell_at(board, 2) == empty_cell
                and cell_at(board, 3) == empty_cell
                and attacked(board, 4, Bool.True) == Bool.False
                and attacked(board, 3, Bool.True) == Bool.False
                and attacked(board, 2, Bool.True) == Bool.False {
                [{ from: 4, to: 2, captured: no_square, promo: 0, kind: 2 }]
            } else {
                []
            }
        List.concat(short, long)
    } else {
        []
    }
}

## Pseudo-legal moves for the piece at `index` (king safety is the
## caller's filter, except castling which states its own).
piece_moves : State, U64, U8 -> List(Move)
piece_moves = |state, index, piece|
    if piece == 80 or piece == 112 {
        pawn_moves(state, index, piece)
    } else if piece == 78 or piece == 110 {
        knight_moves(state.board, index, piece)
    } else if piece == 66 or piece == 98 {
        slide_moves(state.board, index, piece, diag_dirs)
    } else if piece == 82 or piece == 114 {
        slide_moves(state.board, index, piece, straight_dirs)
    } else if piece == 81 or piece == 113 {
        slide_moves(state.board, index, piece, all_dirs)
    } else if piece == 75 or piece == 107 {
        List.concat(step_moves(state.board, index, piece), castle_moves(state, index, piece))
    } else {
        []
    }

pseudo_moves : State, Str -> List(Move)
pseudo_moves = |state, side|
    List.fold(side_indices(state.board, side), [], |acc, index|
        List.concat(acc, piece_moves(state, index, cell_at(state.board, index))))

# --- attack detection ----------------------------------------------------------

## Is `target` attacked by the given colour? Pawns attack from the
## row behind them relative to their travel; knights and the king by
## their steps; sliders by the first piece met along each ray.
attacked : Str, U64, Bool -> Bool
attacked = |board, target, by_white| {
    pawn_dirs = if by_white { [5, 7] } else { [0, 2] }
    pawn_byte = if by_white { 80 } else { 112 }
    pawn_hit = List.any(pawn_dirs, |dir|
        match step_idx(target, dir) {
            Ok(src) => cell_at(board, src) == pawn_byte
            Err(_) => Bool.False
        })
    knight_byte = if by_white { 78 } else { 110 }
    knight_hit = List.any([0, 1, 2, 3, 4, 5, 6, 7], |tag|
        match knight_step(target, tag) {
            Ok(src) => cell_at(board, src) == knight_byte
            Err(_) => Bool.False
        })
    king_byte = if by_white { 75 } else { 107 }
    king_hit = List.any(all_dirs, |dir|
        match step_idx(target, dir) {
            Ok(src) => cell_at(board, src) == king_byte
            Err(_) => Bool.False
        })
    diag_hit = List.any(diag_dirs, |dir| ray_attacked(board, target, dir, by_white, Bool.True))
    straight_hit = List.any(straight_dirs, |dir| ray_attacked(board, target, dir, by_white, Bool.False))
    pawn_hit or knight_hit or king_hit or diag_hit or straight_hit
}

## Walk a ray from the target: the first piece met attacks when it
## is the enemy queen, or the enemy bishop on a diagonal / rook on a
## straight.
ray_attacked : Str, U64, U64, Bool, Bool -> Bool
ray_attacked = |board, pos, dir, by_white, diagonal|
    match step_idx(pos, dir) {
        Ok(next) => {
            cell = cell_at(board, next)
            if cell == empty_cell {
                ray_attacked(board, next, dir, by_white, diagonal)
            } else if by_white != is_white(cell) {
                Bool.False
            } else if cell == 81 or cell == 113 {
                Bool.True
            } else if diagonal {
                cell == 66 or cell == 98
            } else {
                cell == 82 or cell == 114
            }
        }

        Err(_) => Bool.False
    }

king_index : Str, Str -> U64
king_index = |board, side| {
    king = if side == "white" { 75 } else { 107 }
    List.fold(
        List.map_with_index(Str.to_utf8(board), |cell, index| { cell: cell, index: index }),
        no_square,
        |acc, entry| if entry.cell == king { entry.index } else { acc },
    )
}

in_check : Str, Str -> Bool
in_check = |board, side| attacked(board, king_index(board, side), side == "black")

# --- applying moves --------------------------------------------------------------

## Apply a move to the move-generation state: the piece moves (or
## the promotion byte lands), a captured piece is cleared (for en
## passant from its own square), castling also moves the rook, the
## castling rights shrink exactly when a king moves, a rook leaves
## its home square, or a rook is captured on its home square, and
## the en-passant target is set exactly by a double push.
apply_move : State, Move -> State
apply_move = |state, move| {
    board = state.board
    piece = cell_at(board, move.from)
    captured_piece = if move.captured == no_square { empty_cell } else { cell_at(board, move.captured) }
    cleared_from = set_cell(board, move.from, empty_cell)
    cleared_cap = if move.captured == no_square { cleared_from } else { set_cell(cleared_from, move.captured, empty_cell) }
    placed = if move.promo != 0 { move.promo } else { piece }
    landed = set_cell(cleared_cap, move.to, placed)
    rook_moved =
        if move.kind == 1 and piece == 75 {
            set_cell(set_cell(landed, 63, empty_cell), 61, 82)
        } else if move.kind == 1 and piece == 107 {
            set_cell(set_cell(landed, 7, empty_cell), 5, 114)
        } else if move.kind == 2 and piece == 75 {
            set_cell(set_cell(landed, 56, empty_cell), 59, 82)
        } else if move.kind == 2 and piece == 107 {
            set_cell(set_cell(landed, 0, empty_cell), 3, 114)
        } else {
            landed
        }
    rights1 = if piece == 75 { rights_remove(rights_remove(state.castling, 75), 81) } else if piece == 107 { rights_remove(rights_remove(state.castling, 107), 113) } else { state.castling }
    rights2 =
        if move.from == 63 and piece == 82 {
            rights_remove(rights1, 75)
        } else if move.from == 56 and piece == 82 {
            rights_remove(rights1, 81)
        } else if move.from == 7 and piece == 114 {
            rights_remove(rights1, 107)
        } else if move.from == 0 and piece == 114 {
            rights_remove(rights1, 113)
        } else {
            rights1
        }
    rights3 =
        if move.captured == 63 and captured_piece == 82 {
            rights_remove(rights2, 75)
        } else if move.captured == 56 and captured_piece == 82 {
            rights_remove(rights2, 81)
        } else if move.captured == 7 and captured_piece == 114 {
            rights_remove(rights2, 107)
        } else if move.captured == 0 and captured_piece == 114 {
            rights_remove(rights2, 113)
        } else {
            rights2
        }
    delta = if move.to > move.from { move.to - move.from } else { move.from - move.to }
    ep_next = if (piece == 80 or piece == 112) and delta == 16 { (move.from + move.to) / 2 } else { no_square }
    { board: rook_moved, castling: rights3, ep: ep_next }
}

## All fully legal moves for a side: pseudo-legal moves that do not
## leave the mover own king attacked. (This also settles the pinned
## en-passant edge case, where both pawns leave a rank.)
legal_moves : State, Str -> List(Move)
legal_moves = |state, side|
    List.keep_if(pseudo_moves(state, side), |move| in_check(apply_move(state, move).board, side) == Bool.False)

# --- lifecycle ---------------------------------------------------------------

model_state : Model -> State
model_state = |model| { board: model.board, castling: model.castling, ep: model.ep }

init : U64 -> Model
init = |seed| {
    rng = if seed == 0 { 2685821657736338717 } else { seed }
    {
        phase: "playing",
        board: start_board,
        cursor: 52,
        selected: 0,
        sel: 0,
        castling: "KQkq",
        ep: no_square,
        promo_from: 0,
        promo_to: 0,
        promo_idx: 0,
        rng: rng,
        plies: 0,
    }
}

update : Model, U64, Str -> Model
update = |model, _pid, ev|
    match RocJson.decode(ev) {
        Ok(event) =>
            match RocJson.get(event, "cap") {
                Ok(cap_value) =>
                    match RocJson.as_string(cap_value) {
                        Ok(_name) => model
                        Err(_) => input(model, ev)
                    }

                Err(_) => input(model, ev)
            }

        Err(_) => input(model, ev)
    }

## Participant input: raw terminal bytes, decoded into keys.
input : Model, Str -> Model
input = |model, ev| List.fold(Key.parse(ev), model, apply_key)

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
    } else if model.phase == "promo" {
        match key {
            Left | Char("a") | Char("A") => { ..model, promo_idx: (model.promo_idx + 3) % 4 }
            Right | Char("d") | Char("D") => { ..model, promo_idx: (model.promo_idx + 1) % 4 }
            Char("q") | Char("Q") => promote(model, 0)
            Char("r") | Char("R") => promote(model, 1)
            Char("b") | Char("B") => promote(model, 2)
            Char("n") | Char("N") => promote(model, 3)
            Enter | Char(" ") => promote(model, model.promo_idx)
            _ => model
        }
    } else {
        match key {
            Enter | Char("r") | Char("R") => init(model.rng)
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

## Enter/Space: select the white piece under the cursor, deselect it
## by confirming on it again, or confirm the landing square under
## the cursor. A landing that promotes opens the promotion choice
## instead of moving at once.
confirm : Model -> Model
confirm = |model|
    if model.phase != "playing" {
        init(model.rng)
    } else if model.selected == 0 {
        if is_white(cell_at(model.board, model.cursor)) and List.any(legal_moves(model_state(model), "white"), |move| move.from == model.cursor) {
            { ..model, selected: 1, sel: model.cursor }
        } else {
            model
        }
    } else if model.cursor == model.sel {
        { ..model, selected: 0 }
    } else {
        candidates = List.keep_if(legal_moves(model_state(model), "white"), |move| move.from == model.sel and move.to == model.cursor)
        if List.is_empty(candidates) {
            model
        } else if List.any(candidates, |move| move.promo != 0) {
            { ..model, phase: "promo", promo_from: model.sel, promo_to: model.cursor, promo_idx: 0 }
        } else {
            chosen = first_or_empty(candidates)
            complete_turn(apply_to_model(model, chosen))
        }
    }

## The promotion piece is chosen: apply the matching move and hand
## over to the AI.
promote : Model, U64 -> Model
promote = |model, idx| {
    byte = promo_byte(idx, Bool.True)
    candidates = List.keep_if(
        legal_moves(model_state(model), "white"),
        |move| move.from == model.promo_from and move.to == model.promo_to and move.promo == byte,
    )
    match List.first(candidates) {
        Ok(chosen) => complete_turn(apply_to_model({ ..model, phase: "playing" }, chosen))
        Err(_) => { ..model, phase: "playing" }
    }
}

promo_byte : U64, Bool -> U8
promo_byte = |idx, white| {
    bytes = if white { [81, 82, 66, 78] } else { [113, 114, 98, 110] }
    match List.get(bytes, idx) {
        Ok(byte) => byte
        Err(_) => if white { 81 } else { 113 }
    }
}

first_or_empty : List(Move) -> Move
first_or_empty = |moves|
    match List.first(moves) {
        Ok(move) => move
        Err(_) => { from: 0, to: 0, captured: no_square, promo: 0, kind: 0 }
    }

## Put a chosen move onto the model: board, castling rights, and the
## en-passant target all move together.
apply_to_model : Model, Move -> Model
apply_to_model = |model, move| {
    next = apply_move(model_state(model), move)
    {
        ..model,
        board: next.board,
        castling: next.castling,
        ep: next.ep,
        selected: 0,
        plies: model.plies + 1,
    }
}

## A move is on the board: if black has no legal move the game is
## won (checkmate) or drawn (stalemate); otherwise the AI replies at
## once, and if that leaves white with no legal move the game is
## lost or drawn the same way.
complete_turn : Model -> Model
complete_turn = |model| {
    black_moves = legal_moves(model_state(model), "black")
    if List.is_empty(black_moves) {
        if in_check(model.board, "black") {
            { ..model, phase: "won" }
        } else {
            { ..model, phase: "draw" }
        }
    } else {
        choice = ai_choose(model_state(model), model.rng)
        settled = apply_to_model({ ..model, rng: choice.rng }, choice.move)
        if List.is_empty(legal_moves(model_state(settled), "white")) {
            if in_check(settled.board, "white") {
                { ..settled, phase: "lost" }
            } else {
                { ..settled, phase: "draw" }
            }
        } else {
            settled
        }
    }
}

# --- the weak AI ---------------------------------------------------------------

piece_value : U8 -> U64
piece_value = |cell|
    if cell == 80 or cell == 112 {
        100
    } else if cell == 78 or cell == 110 or cell == 66 or cell == 98 {
        300
    } else if cell == 82 or cell == 114 {
        500
    } else if cell == 81 or cell == 113 {
        900
    } else {
        0
    }

## One ply of greed, nothing more: the material captured by this
## move, a bonus for promoting (the new piece value over the pawn),
## and a whisper for advancing a pawn. Whether the moved piece
## hangs, whether the move opens a line, what the opponent
## threatens — none of it is considered.
score_move : State, Move -> U64
score_move = |state, move| {
    captured_value = if move.captured == no_square { 0 } else { piece_value(cell_at(state.board, move.captured)) }
    promo_bonus = if move.promo != 0 { piece_value(move.promo) - 100 } else { 0 }
    piece = cell_at(state.board, move.from)
    advance = if piece == 112 { (move.to / 8) * 2 } else { 0 }
    captured_value + promo_bonus + advance
}

## Pick black move. One draw in four plays a uniformly random legal
## move (the blunder draw). Otherwise the highest one-ply score
## wins, with uniform jitter of 0..40 on every score.
ai_choose : State, U64 -> { move : Move, rng : U64 }
ai_choose = |state, rng| {
    moves = legal_moves(state, "black")
    fallback = { from: 0, to: 0, captured: no_square, promo: 0, kind: 0 }
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
                score = score_move(state, move) + (drawn % 41)
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
    hud = "CHESS  WHITE ${count_side(model.board, "white").to_str()}  BLACK ${count_side(model.board, "black").to_str()}"
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

## Pieces render as their letter: uppercase for white, lowercase
## for black. A dot marks an empty dark square.
glyph_for : U8, U64 -> Str
glyph_for = |cell, index|
    if cell == empty_cell {
        if (index / 8 + index % 8) % 2 == 1 {
            " . "
        } else {
            "   "
        }
    } else {
        match Str.from_utf8([cell]) {
            Ok(letter) => " ${letter} "
            Err(_) => " ? "
        }
    }

style_for : U8 -> { fg : U8, bg : U8, bold : Bool }
style_for = |cell|
    if is_white(cell) {
        { fg: 255, bg: 0, bold: cell == 75 }
    } else if is_black(cell) {
        { fg: 245, bg: 0, bold: cell == 107 }
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
status_text = |model|
    if model.phase == "won" {
        "CHECKMATE - YOU WIN!"
    } else if model.phase == "lost" {
        "CHECKMATE - AI WINS"
    } else if model.phase == "draw" {
        "STALEMATE - DRAW"
    } else if model.phase == "promo" {
        options = ["Q", "R", "B", "N"]
        rendered = List.map_with_index(options, |name, idx|
            if idx == model.promo_idx { "[${name}]" } else { " ${name} " })
        "promote: ${Str.join_with(rendered, " ")}"
    } else if model.selected == 1 {
        "selected: pick a landing"
    } else {
        "your move: white to play"
    }

help_text : Model -> Str
help_text = |model|
    if model.phase == "promo" {
        "arrows choose, ENTER ok"
    } else if model.phase == "playing" {
        "arrows move, ENTER select"
    } else {
        "ENTER = new game"
    }

# --- records, wants, snapshot --------------------------------------------------

records : Model -> Str
records = |model| encode_or_crash(
    RocJson.Object([
        { key: "white", value: RocJson.Number(count_side(model.board, "white").to_str()) },
        { key: "black", value: RocJson.Number(count_side(model.board, "black").to_str()) },
        { key: "phase", value: RocJson.String(model.phase) },
        { key: "plies", value: RocJson.Number(model.plies.to_str()) },
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
        { key: "castling", value: RocJson.String(model.castling) },
        { key: "ep", value: RocJson.Number(model.ep.to_str()) },
        { key: "promo_from", value: RocJson.Number(model.promo_from.to_str()) },
        { key: "promo_to", value: RocJson.Number(model.promo_to.to_str()) },
        { key: "promo_idx", value: RocJson.Number(model.promo_idx.to_str()) },
        { key: "rng", value: RocJson.Number(model.rng.to_str()) },
        { key: "plies", value: RocJson.Number(model.plies.to_str()) },
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
                phase: if phase == "won" or phase == "lost" or phase == "draw" or phase == "promo" { phase } else { "playing" },
                board: board_field(event),
                cursor: clamp(num_field(event, "cursor", fresh.cursor), 0, 63),
                selected: if num_field(event, "selected", fresh.selected) == 1 { 1 } else { 0 },
                sel: clamp(num_field(event, "sel", fresh.sel), 0, 63),
                castling: castling_field(event),
                ep: clamp(num_field(event, "ep", fresh.ep), 0, 64),
                promo_from: clamp(num_field(event, "promo_from", fresh.promo_from), 0, 63),
                promo_to: clamp(num_field(event, "promo_to", fresh.promo_to), 0, 63),
                promo_idx: clamp(num_field(event, "promo_idx", fresh.promo_idx), 0, 3),
                rng: num_field(event, "rng", fresh.rng),
                plies: num_field(event, "plies", fresh.plies),
            }
        }
    }

## A restored board must be exactly 64 cells of board bytes; anything
## else falls back to the opening position.
board_field = |event| {
    text = string_field(event, "board")
    bytes = Str.to_utf8(text)
    valid = List.fold(bytes, Bool.True, |acc, byte|
        acc and (byte == empty_cell or is_white(byte) or is_black(byte)))
    if List.len(bytes) == 64 and valid {
        text
    } else {
        start_board
    }
}

## Castling rights are a subsequence of "KQkq"; anything else in the
## stored string is dropped.
castling_field = |event| {
    text = string_field(event, "castling")
    match Str.from_utf8(List.keep_if(Str.to_utf8(text), |b| b == 75 or b == 81 or b == 107 or b == 113)) {
        Ok(rights) => rights
        Err(_) => "KQkq"
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
    Err(_) => crash "chess attempted to encode an invalid or oversized JSON value"
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

find_move : List(Move), U64, U64 -> Move
find_move = |moves, from, to| first_or_empty(List.keep_if(moves, |move| move.from == from and move.to == to))

start_state : State
start_state = { board: start_board, castling: "KQkq", ep: no_square }

expect Str.count_utf8_bytes(start_board) == 64
expect count_side(start_board, "white") == 16 and count_side(start_board, "black") == 16
expect init(0).phase == "playing" and init(0).cursor == 52 and init(0).castling == "KQkq"

# The opening position gives each side the standard twenty moves.
expect List.len(legal_moves(start_state, "white")) == 20
expect List.len(legal_moves(start_state, "black")) == 20

# Cursor movement clamps at the board edges.
expect apply_key(init(0), Left).cursor == 51
expect apply_key(init(0), Down).cursor == 60

# A host tick is not a subscription this game holds; it changes nothing.
expect update(init(0), 0, "{\"cap\":\"time.tick\",\"interval\":1000,\"now\":1}") == init(0)

# --- the double push and the en-passant target ---------------------------------

after_e4 = apply_move(start_state, find_move(legal_moves(start_state, "white"), 52, 36))

expect cell_at(after_e4.board, 36) == 80 and cell_at(after_e4.board, 52) == empty_cell
expect after_e4.ep == 44 and after_e4.castling == "KQkq"

# --- a full opening exchange through the keys ------------------------------------

picked = apply_key(init(0), Enter)
opened = List.fold([Enter, Up, Up, Enter], init(0), apply_key)

expect picked.selected == 1
expect confirm(picked).selected == 0
expect opened.plies == 2 and opened.phase == "playing"
expect cell_at(opened.board, 36) == 80 and cell_at(opened.board, 52) == empty_cell
expect count_side(opened.board, "white") == 16 and count_side(opened.board, "black") == 16

# --- a forced mate line ------------------------------------------------------------

mate_board = board_from_rows([
    "......k.",
    ".....ppp",
    "........",
    "........",
    "........",
    "........",
    "........",
    "R.....K.",
])

mate_state = { board: mate_board, castling: "", ep: no_square }
mate_move = find_move(legal_moves(mate_state, "white"), 56, 0)
mated = apply_move(mate_state, mate_move)

# The rook lift to the back rank exists, and after it black is
# checkmated: no legal move and the king attacked. (The f/g/h pawns
# can still push, but a push does not lift a back-rank check.)
expect mate_move.from == 56 and mate_move.to == 0
expect List.len(legal_moves(mated, "black")) == 0 and in_check(mated.board, "black")

# The same mate driven through the cursor ends the game as won.
mate_model = { phase: "playing", board: mate_board, cursor: 0, selected: 1, sel: 56, castling: "", ep: no_square, promo_from: 0, promo_to: 0, promo_idx: 0, rng: 1, plies: 0 }
mate_game = confirm(mate_model)

expect mate_game.phase == "won" and cell_at(mate_game.board, 0) == 82

# --- castling ------------------------------------------------------------------------

castle_board = board_from_rows([
    "....k...",
    "........",
    "........",
    "........",
    "........",
    "........",
    "........",
    "....K..R",
])

castle_state = { board: castle_board, castling: "K", ep: no_square }
castle_move = first_or_empty(List.keep_if(legal_moves(castle_state, "white"), |move| move.kind == 1))
castled = apply_move(castle_state, castle_move)

expect castle_move.to == 62
expect cell_at(castled.board, 62) == 75 and cell_at(castled.board, 61) == 82
expect cell_at(castled.board, 60) == empty_cell and cell_at(castled.board, 63) == empty_cell
expect castled.castling == ""

# Castling through check is illegal: the black rook on f8 covers f1,
# the square the king must cross, while the king itself is not in
# check — the transit condition alone forbids the castle.
transit_board = board_from_rows([
    "....kr..",
    "........",
    "........",
    "........",
    "........",
    "........",
    "........",
    "....K..R",
])

transit_state = { board: transit_board, castling: "K", ep: no_square }

expect in_check(transit_board, "white") == Bool.False
expect List.any(legal_moves(transit_state, "white"), |move| move.kind == 1) == Bool.False

# Castling out of check is illegal too: the rook on e8 checks the
# king down the open file.
check_board = board_from_rows([
    "k...r...",
    "........",
    "........",
    "........",
    "........",
    "........",
    "........",
    "....K..R",
])

check_state = { board: check_board, castling: "K", ep: no_square }

expect in_check(check_board, "white") == Bool.True
expect List.any(legal_moves(check_state, "white"), |move| move.kind == 1) == Bool.False
expect List.is_empty(legal_moves(check_state, "white")) == Bool.False

# --- en passant ------------------------------------------------------------------------

ep_board = board_from_rows([
    "....k...",
    "........",
    "........",
    "...pP...",
    "........",
    "........",
    "........",
    "....K...",
])

ep_state = { board: ep_board, castling: "", ep: 19 }
ep_move = find_move(legal_moves(ep_state, "white"), 28, 19)
ep_after = apply_move(ep_state, ep_move)

expect ep_move.kind == 3 and ep_move.captured == 27
expect cell_at(ep_after.board, 19) == 80
expect cell_at(ep_after.board, 27) == empty_cell and cell_at(ep_after.board, 28) == empty_cell

# Without the target set by a double push, the capture does not exist.
expect List.any(legal_moves({ ..ep_state, ep: no_square }, "white"), |move| move.kind == 3) == Bool.False

# The pinned en-passant edge case: capturing would remove both pawns
# from the fifth rank and expose the white king to the rook, so the
# capture is illegal even though the target is set.
pin_board = board_from_rows([
    "....k...",
    "........",
    "........",
    "K..pP..r",
    "........",
    "........",
    "........",
    "........",
])

pin_state = { board: pin_board, castling: "", ep: 19 }

expect List.any(legal_moves(pin_state, "white"), |move| move.kind == 3) == Bool.False

# --- promotion ---------------------------------------------------------------------------

promo_board = board_from_rows([
    "....k...",
    "P.......",
    "........",
    "........",
    "........",
    "........",
    "........",
    "....K...",
])

promo_state = { board: promo_board, castling: "", ep: no_square }
promo_list = List.keep_if(legal_moves(promo_state, "white"), |move| move.from == 8)
queen_after = apply_move(promo_state, first_or_empty(List.keep_if(promo_list, |move| move.promo == 81)))

expect List.len(promo_list) == 4
expect List.any(promo_list, |move| move.promo == 81) and List.any(promo_list, |move| move.promo == 78)
expect cell_at(queen_after.board, 0) == 81

# Through the cursor the promotion asks first; the letter key then
# picks the knight and the AI replies.
promo_model = { phase: "playing", board: promo_board, cursor: 0, selected: 1, sel: 8, castling: "", ep: no_square, promo_from: 0, promo_to: 0, promo_idx: 0, rng: 1, plies: 0 }
promo_ask = confirm(promo_model)
knighted = apply_key(promo_ask, Char("n"))

expect promo_ask.phase == "promo" and promo_ask.promo_from == 8 and promo_ask.promo_to == 0
expect apply_key(promo_ask, Right).promo_idx == 1
expect cell_at(knighted.board, 0) == 78 and knighted.phase == "playing" and knighted.plies == 2

# --- stalemate ------------------------------------------------------------------------------

stale_board = board_from_rows([
    "k.......",
    "........",
    ".Q......",
    "........",
    "........",
    "........",
    "........",
    ".......K",
])

stale_state = { board: stale_board, castling: "", ep: no_square }

# Black to move has no legal move and is not in check: stalemate.
expect List.len(legal_moves(stale_state, "black")) == 0 and in_check(stale_board, "black") == Bool.False

# Delivered through the cursor, the same shape ends the game drawn.
stale_model_board = board_from_rows([
    "k.......",
    "........",
    "........",
    ".Q......",
    "........",
    "........",
    "........",
    ".......K",
])

stale_model = { phase: "playing", board: stale_model_board, cursor: 17, selected: 1, sel: 25, castling: "", ep: no_square, promo_from: 0, promo_to: 0, promo_idx: 0, rng: 1, plies: 0 }
stale_game = confirm(stale_model)

expect stale_game.phase == "draw"

# --- the weak AI ------------------------------------------------------------------------------

ai_open = ai_choose(start_state, 7)

# Whatever the jitter or the blunder draw picks, it is a legal move.
expect List.any(legal_moves(start_state, "black"), |move| move == ai_open.move)

greed_board = board_from_rows([
    "....k...",
    "........",
    "........",
    "..p.....",
    "...Q....",
    "........",
    "........",
    "....K...",
])

greed_state = { board: greed_board, castling: "", ep: no_square }
greed_capture = find_move(legal_moves(greed_state, "black"), 26, 35)
greed_quiet = find_move(legal_moves(greed_state, "black"), 26, 34)

# One ply of greed: taking the queen outscores the quiet pawn push
# by more than the queen value less the jitter band.
expect greed_capture.captured == 35
expect score_move(greed_state, greed_capture) > score_move(greed_state, greed_quiet) + 500

# --- snapshot round-trip --------------------------------------------------------------------------

mid_game = { phase: "playing", board: ep_board, cursor: 19, selected: 1, sel: 28, castling: "Kq", ep: 19, promo_from: 0, promo_to: 0, promo_idx: 0, rng: 12345, plies: 9 }

expect restore(snapshot(mid_game)) == mid_game
expect restore("not json at all") == init(0)
