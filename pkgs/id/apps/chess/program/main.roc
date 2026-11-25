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
# Pseudo-legal moves are generated per piece and filtered by king
# safety, so pins, discovered checks, and moving into check are all
# handled by the one filter.
#
# The AI is weak on purpose (the issue's acceptance bar, same
# doctrine as checkers slice 1): it looks exactly one ply ahead and
# scores a move only by the material it captures this turn, plus the
# promotion bonus and a whisper for advancing a pawn. Uniform jitter
# of 0..40 rides on every score, and one draw in four ignores the
# scores entirely and plays a uniformly random legal move. It never
# checks whether the piece it just moved can be recaptured, never
# looks at the opponent's threats, and has no opening book. A
# beginner is genuinely hampered; a competent child beats it easily.
#
# Everything is a pure function of (model, event): keystrokes are
# journaled like any other world input, the AI's randomness comes
# from a xorshift PRNG carried in the model, and the journal replays
# a game exactly. The AI replies inside the same `update` that
# completes the human's turn, so at rest it is always white to move.
#
# The frame is data: `view`/`layout` render a `Screen` and the shell
# paints it in whichever presentation mode the session negotiated
# (alternate, primary, or low-write text, #14936/#14937). The game
# never changes when the mode does.
#
# Controls: arrow keys or WASD move the cursor, Enter (or Space)
# selects a piece and confirms a landing square. Confirming a pawn
# move onto the last rank opens the promotion choice: Left/Right
# (or A/D) cycle queen, rook, bishop, knight and Enter confirms.
# Enter (or R) starts a new game after a win, a loss, or a draw.
#
# Later slices of #14862: id-network multiplayer (one binary hosts
# and the other joins, or a central server both players join), and
# the three-machine demonstration.

Model : {
    phase : Str,
    board : Str,
    cursor : U64,
    selected : U64,
    sel : U64,
    promo : U64,
    promo_choice : U64,
    castling : U64,
    ep : U64,
    rng : U64,
    plies : U64,
}

## One move. `promo` is the piece byte to promote to, or 0 for an
## ordinary move. Everything else about a move — the captured piece,
## the castling rook, the en-passant victim, the new en-passant
## square — is derived from the position in `apply_move`.
Move : { from : U64, to : U64, promo : U8 }

## A position: the board plus the two pieces of state move legality
## needs beyond it (castling rights, en-passant target).
Pos : { board : Str, castling : U64, ep : U64 }

program = { init, update, view, layout, records, wants, snapshot, restore }

# --- board basics ------------------------------------------------------------
#
# The board is a 64-byte string, row-major, row 0 at the top (rank 8,
# black's home rows) and row 7 at the bottom (rank 1, white's).
# Bytes: '.' empty; white pieces are uppercase 'K', 'Q', 'R', 'B',
# 'N', 'P'; black pieces are the lowercase twins.

empty_cell : U8
empty_cell = 46

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

## Castling rights bitmask: 1 = white kingside, 2 = white queenside,
## 4 = black kingside, 8 = black queenside. The opening position
## holds all four; `ep` is the en-passant target square index, or 64
## when no pawn has just double-pushed.
start_pos : Pos
start_pos = { board: start_board, castling: 15, ep: 64 }

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
is_white = |cell| cell == 75 or cell == 81 or cell == 82 or cell == 66 or cell == 78 or cell == 80

is_black : U8 -> Bool
is_black = |cell| cell == 107 or cell == 113 or cell == 114 or cell == 98 or cell == 110 or cell == 112

is_pawn : U8 -> Bool
is_pawn = |cell| cell == 80 or cell == 112

side_of : U8 -> Str
side_of = |cell|
    if is_white(cell) {
        "white"
    } else if is_black(cell) {
        "black"
    } else {
        ""
    }

enemy_of : Str -> Str
enemy_of = |side| if side == "white" { "black" } else { "white" }

king_byte : Str -> U8
king_byte = |side| if side == "white" { 75 } else { 107 }

count_side : Str, Str -> U64
count_side = |board, side|
    List.fold(Str.to_utf8(board), 0, |acc, cell|
        if side_of(cell) == side {
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
            if side_of(entry.cell) == side {
                List.append(acc, entry.index)
            } else {
                acc
            },
    )

## One step in one of the eight compass directions, as tags:
## 0 = N, 1 = NE, 2 = E, 3 = SE, 4 = S, 5 = SW, 6 = W, 7 = NW
## (north = toward row 0, white's direction of travel).
step_rc : U64, U64 -> Try(U64, {})
step_rc = |index, dir| {
    row = index / 8
    col = index % 8
    if dir == 0 and row > 0 {
        Ok(index - 8)
    } else if dir == 1 and row > 0 and col < 7 {
        Ok(index - 7)
    } else if dir == 2 and col < 7 {
        Ok(index + 1)
    } else if dir == 3 and row < 7 and col < 7 {
        Ok(index + 9)
    } else if dir == 4 and row < 7 {
        Ok(index + 8)
    } else if dir == 5 and row < 7 and col > 0 {
        Ok(index + 7)
    } else if dir == 6 and col > 0 {
        Ok(index - 1)
    } else if dir == 7 and row > 0 and col > 0 {
        Ok(index - 9)
    } else {
        Err({})
    }
}

## One knight jump, as tags 0..7 in reading order of (dr, dc):
## (-2,-1), (-2,+1), (-1,-2), (-1,+2), (+1,-2), (+1,+2), (+2,-1), (+2,+1).
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

knight_targets : U64 -> List(U64)
knight_targets = |index|
    List.fold([0, 1, 2, 3, 4, 5, 6, 7], [], |acc, tag|
        match knight_step(index, tag) {
            Ok(target) => List.append(acc, target)
            Err(_) => acc
        })

# --- attack detection ----------------------------------------------------------

## Walk a ray from `from` in direction `dir`: the ray attacks through
## empty squares and stops at the first piece; it is an attack if
## that piece is one of the two attacker bytes.
ray_attacked : Str, U64, U64, U8, U8 -> Bool
ray_attacked = |board, from, dir, a, b|
    match step_rc(from, dir) {
        Err(_) => Bool.False
        Ok(next) => {
            cell = cell_at(board, next)
            if cell == empty_cell {
                ray_attacked(board, next, dir, a, b)
            } else {
                cell == a or cell == b
            }
        }
    }

## Is `target` attacked by any piece of side `by_side`?
is_attacked : Str, U64, Str -> Bool
is_attacked = |board, target, by_side| {
    row = target / 8
    col = target % 8
    pawn = if by_side == "white" { 80 } else { 112 }
    knight = if by_side == "white" { 78 } else { 110 }
    king = king_byte(by_side)
    bishop = if by_side == "white" { 66 } else { 98 }
    rook = if by_side == "white" { 82 } else { 114 }
    queen = if by_side == "white" { 81 } else { 113 }
    # A white pawn attacks one row up, so it sits one row below its
    # target; a black pawn sits one row above.
    pawn_hit =
        if by_side == "white" {
            (row < 7 and col > 0 and cell_at(board, target + 7) == pawn)
            or (row < 7 and col < 7 and cell_at(board, target + 9) == pawn)
        } else {
            (row > 0 and col > 0 and cell_at(board, target - 9) == pawn)
            or (row > 0 and col < 7 and cell_at(board, target - 7) == pawn)
        }
    knight_hit = List.any(knight_targets(target), |sq| cell_at(board, sq) == knight)
    king_hit = List.any([0, 1, 2, 3, 4, 5, 6, 7], |dir|
        match step_rc(target, dir) {
            Ok(sq) => cell_at(board, sq) == king
            Err(_) => Bool.False
        })
    diag_hit = List.any([1, 3, 5, 7], |dir| ray_attacked(board, target, dir, bishop, queen))
    line_hit = List.any([0, 2, 4, 6], |dir| ray_attacked(board, target, dir, rook, queen))
    pawn_hit or knight_hit or king_hit or diag_hit or line_hit
}

king_index : Str, Str -> U64
king_index = |board, side|
    List.fold(
        List.map_with_index(Str.to_utf8(board), |cell, index| { cell: cell, index: index }),
        64,
        |acc, entry|
            if entry.cell == king_byte(side) {
                entry.index
            } else {
                acc
            },
    )

in_check : Str, Str -> Bool
in_check = |board, side| is_attacked(board, king_index(board, side), enemy_of(side))

# --- move generation -----------------------------------------------------------

## Append a pawn move, expanding a move onto the promotion rank into
## the four promotion choices.
add_pawn_move : List(Move), U64, U64, Str -> List(Move)
add_pawn_move = |acc, from, to, side| {
    promo_row = if side == "white" { 0 } else { 7 }
    if to / 8 == promo_row {
        promos = if side == "white" { [81, 82, 66, 78] } else { [113, 114, 98, 110] }
        List.fold(promos, acc, |inner, promo| List.append(inner, { from: from, to: to, promo: promo }))
    } else {
        List.append(acc, { from: from, to: to, promo: 0 })
    }
}

pawn_moves : Pos, U64, Str -> List(Move)
pawn_moves = |pos, index, side| {
    row = index / 8
    col = index % 8
    if side == "white" {
        pushed =
            if row > 0 and cell_at(pos.board, index - 8) == empty_cell {
                one = add_pawn_move([], index, index - 8, side)
                if row == 6 and cell_at(pos.board, index - 16) == empty_cell {
                    List.append(one, { from: index, to: index - 16, promo: 0 })
                } else {
                    one
                }
            } else {
                []
            }
        captures = List.fold([0, 1], pushed, |acc, which| {
            diag =
                if which == 0 and col > 0 {
                    index - 9
                } else if which == 1 and col < 7 {
                    index - 7
                } else {
                    64
                }
            if diag < 64 and (side_of(cell_at(pos.board, diag)) == "black" or diag == pos.ep) {
                add_pawn_move(acc, index, diag, side)
            } else {
                acc
            }
        })
        captures
    } else {
        pushed =
            if row < 7 and cell_at(pos.board, index + 8) == empty_cell {
                one = add_pawn_move([], index, index + 8, side)
                if row == 1 and cell_at(pos.board, index + 16) == empty_cell {
                    List.append(one, { from: index, to: index + 16, promo: 0 })
                } else {
                    one
                }
            } else {
                []
            }
        captures = List.fold([0, 1], pushed, |acc, which| {
            diag =
                if which == 0 and col > 0 {
                    index + 7
                } else if which == 1 and col < 7 {
                    index + 9
                } else {
                    64
                }
            if diag < 64 and (side_of(cell_at(pos.board, diag)) == "white" or diag == pos.ep) {
                add_pawn_move(acc, index, diag, side)
            } else {
                acc
            }
        })
        captures
    }
}

## Leaper moves (knight, king): every target square that is empty or
## holds an enemy.
leaper_moves : Str, U64, Str, List(U64) -> List(Move)
leaper_moves = |board, index, side, targets|
    List.fold(targets, [], |acc, target| {
        occupant = side_of(cell_at(board, target))
        if occupant == "" or occupant == enemy_of(side) {
            List.append(acc, { from: index, to: target, promo: 0 })
        } else {
            acc
        }
    })

king_steps : U64 -> List(U64)
king_steps = |index|
    List.fold([0, 1, 2, 3, 4, 5, 6, 7], [], |acc, dir|
        match step_rc(index, dir) {
            Ok(target) => List.append(acc, target)
            Err(_) => acc
        })

## Slider moves (bishop, rook, queen): walk each ray until the edge
## or the first piece; an enemy piece is a capture and ends the ray.
slider_moves : Str, U64, Str, List(U64) -> List(Move)
slider_moves = |board, index, side, dirs|
    List.fold(dirs, [], |acc, dir| ray_moves(board, index, index, side, dir, acc))

ray_moves : Str, U64, U64, Str, U64, List(Move) -> List(Move)
ray_moves = |board, from, at, side, dir, acc|
    match step_rc(at, dir) {
        Err(_) => acc
        Ok(next) => {
            occupant = side_of(cell_at(board, next))
            if occupant == "" {
                ray_moves(board, from, next, side, dir, List.append(acc, { from: from, to: next, promo: 0 }))
            } else if occupant == enemy_of(side) {
                List.append(acc, { from: from, to: next, promo: 0 })
            } else {
                acc
            }
        }
    }

## Castling, when the rights bit stands: king and rook unmoved (the
## bit is exactly that record), the squares between empty, and the
## king's path — origin, transit, destination — under no attack.
castle_moves : Pos, Str -> List(Move)
castle_moves = |pos, side| {
    home = if side == "white" { 60 } else { 4 }
    if cell_at(pos.board, home) != king_byte(side) {
        []
    } else {
        enemy = enemy_of(side)
        ks_bit = if side == "white" { 1 } else { 4 }
        qs_bit = if side == "white" { 2 } else { 8 }
        rook = if side == "white" { 82 } else { 114 }
        kingside =
            if (pos.castling % (ks_bit * 2)) / ks_bit == 1
            and cell_at(pos.board, home + 3) == rook
            and cell_at(pos.board, home + 1) == empty_cell
            and cell_at(pos.board, home + 2) == empty_cell
            and !is_attacked(pos.board, home, enemy)
            and !is_attacked(pos.board, home + 1, enemy)
            and !is_attacked(pos.board, home + 2, enemy) {
                [{ from: home, to: home + 2, promo: 0 }]
            } else {
                []
            }
        queenside =
            if (pos.castling % (qs_bit * 2)) / qs_bit == 1
            and cell_at(pos.board, home - 4) == rook
            and cell_at(pos.board, home - 1) == empty_cell
            and cell_at(pos.board, home - 2) == empty_cell
            and cell_at(pos.board, home - 3) == empty_cell
            and !is_attacked(pos.board, home, enemy)
            and !is_attacked(pos.board, home - 1, enemy)
            and !is_attacked(pos.board, home - 2, enemy) {
                [{ from: home, to: home - 2, promo: 0 }]
            } else {
                []
            }
        List.concat(kingside, queenside)
    }
}

piece_moves : Pos, U64, U8, Str -> List(Move)
piece_moves = |pos, index, piece, side|
    if is_pawn(piece) {
        pawn_moves(pos, index, side)
    } else if piece == 78 or piece == 110 {
        leaper_moves(pos.board, index, side, knight_targets(index))
    } else if piece == 75 or piece == 107 {
        List.concat(leaper_moves(pos.board, index, side, king_steps(index)), castle_moves(pos, side))
    } else if piece == 66 or piece == 98 {
        slider_moves(pos.board, index, side, [1, 3, 5, 7])
    } else if piece == 82 or piece == 114 {
        slider_moves(pos.board, index, side, [0, 2, 4, 6])
    } else {
        slider_moves(pos.board, index, side, [0, 1, 2, 3, 4, 5, 6, 7])
    }

pseudo_moves : Pos, Str -> List(Move)
pseudo_moves = |pos, side|
    List.fold(side_indices(pos.board, side), [], |acc, index|
        List.concat(acc, piece_moves(pos, index, cell_at(pos.board, index), side)))

## Apply a move, deriving everything the move record does not carry:
## the en-passant victim (a pawn moving diagonally onto the empty
## en-passant square), the castling rook, the new castling rights
## (king moves, rook moves, and rooks captured on their home squares
## all clear bits), and the new en-passant target (set exactly by a
## double push).
apply_move : Pos, Move -> Pos
apply_move = |pos, move| {
    piece = cell_at(pos.board, move.from)
    captured_cell = cell_at(pos.board, move.to)
    ep_victim =
        if is_pawn(piece) and pos.ep < 64 and move.to == pos.ep and captured_cell == empty_cell and (move.to % 8) != (move.from % 8) {
            (move.from / 8) * 8 + (move.to % 8)
        } else {
            64
        }
    cleared = set_cell(pos.board, move.from, empty_cell)
    victim_cleared = if ep_victim < 64 { set_cell(cleared, ep_victim, empty_cell) } else { cleared }
    placed = if move.promo != 0 { move.promo } else { piece }
    landed = set_cell(victim_cleared, move.to, placed)
    rook_moved =
        if (piece == 75 or piece == 107) and move.to == move.from + 2 {
            set_cell(set_cell(landed, move.from + 3, empty_cell), move.from + 1, cell_at(landed, move.from + 3))
        } else if (piece == 75 or piece == 107) and move.from >= 2 and move.to == move.from - 2 {
            set_cell(set_cell(landed, move.from - 4, empty_cell), move.from - 1, cell_at(landed, move.from - 4))
        } else {
            landed
        }
    rights0 =
        if piece == 75 {
            U64.bitwise_and(pos.castling, 12)
        } else if piece == 107 {
            U64.bitwise_and(pos.castling, 3)
        } else {
            pos.castling
        }
    rights1 = if move.from == 63 or move.to == 63 { U64.bitwise_and(rights0, 14) } else { rights0 }
    rights2 = if move.from == 56 or move.to == 56 { U64.bitwise_and(rights1, 13) } else { rights1 }
    rights3 = if move.from == 7 or move.to == 7 { U64.bitwise_and(rights2, 11) } else { rights2 }
    rights4 = if move.from == 0 or move.to == 0 { U64.bitwise_and(rights3, 7) } else { rights3 }
    new_ep =
        if piece == 80 and move.from >= 16 and move.to == move.from - 16 {
            move.from - 8
        } else if piece == 112 and move.to == move.from + 16 {
            move.from + 8
        } else {
            64
        }
    { board: rook_moved, castling: rights4, ep: new_ep }
}

## All legal moves for a side: pseudo-legal moves that do not leave
## the side's own king under attack.
legal_moves : Pos, Str -> List(Move)
legal_moves = |pos, side|
    List.keep_if(pseudo_moves(pos, side), |move| !in_check(apply_move(pos, move).board, side))

## The state of the game for the side to move.
position_status : Pos, Str -> Str
position_status = |pos, side|
    if List.is_empty(legal_moves(pos, side)) {
        if in_check(pos.board, side) {
            "checkmate"
        } else {
            "stalemate"
        }
    } else {
        "playing"
    }

# --- lifecycle ---------------------------------------------------------------

init : U64 -> Model
init = |seed| {
    rng = if seed == 0 { 2685821657736338717 } else { seed }
    {
        phase: "playing",
        board: start_board,
        cursor: 52,
        selected: 0,
        sel: 0,
        promo: 0,
        promo_choice: 0,
        castling: 15,
        ep: 64,
        rng: rng,
        plies: 0,
    }
}

model_pos : Model -> Pos
model_pos = |model| { board: model.board, castling: model.castling, ep: model.ep }

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
        if model.promo == 1 {
            match key {
                Left | Char("a") | Char("A") => { ..model, promo_choice: (model.promo_choice + 3) % 4 }
                Right | Char("d") | Char("D") => { ..model, promo_choice: (model.promo_choice + 1) % 4 }
                Enter | Char(" ") => confirm_promo(model)
                _ => model
            }
        } else {
            match key {
                Up | Char("w") | Char("W") => move_cursor(model, 0)
                Right | Char("d") | Char("D") => move_cursor(model, 1)
                Down | Char("s") | Char("S") => move_cursor(model, 2)
                Left | Char("a") | Char("A") => move_cursor(model, 3)
                Enter | Char(" ") => confirm(model)
                _ => model
            }
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
## the cursor. A landing on the promotion rank opens the promotion
## choice instead of moving at once.
confirm : Model -> Model
confirm = |model|
    if model.phase != "playing" {
        init(model.rng)
    } else if model.selected == 0 {
        if is_white(cell_at(model.board, model.cursor)) and List.any(legal_moves(model_pos(model), "white"), |move| move.from == model.cursor) {
            { ..model, selected: 1, sel: model.cursor }
        } else {
            model
        }
    } else if model.cursor == model.sel {
        { ..model, selected: 0 }
    } else {
        candidates = List.keep_if(legal_moves(model_pos(model), "white"), |move| move.from == model.sel and move.to == model.cursor)
        if List.is_empty(candidates) {
            model
        } else if List.any(candidates, |move| move.promo != 0) {
            { ..model, promo: 1, promo_choice: 0 }
        } else {
            chosen = first_or_empty(candidates)
            after = apply_move(model_pos(model), chosen)
            complete_turn({ ..model, board: after.board, castling: after.castling, ep: after.ep, selected: 0, plies: model.plies + 1 })
        }
    }

## The promotion choice is confirmed: play the pending move with the
## cycled-to promotion piece.
confirm_promo : Model -> Model
confirm_promo = |model| {
    wanted = promo_byte(model.promo_choice)
    candidates = List.keep_if(
        legal_moves(model_pos(model), "white"),
        |move| move.from == model.sel and move.to == model.cursor and move.promo == wanted,
    )
    match List.first(candidates) {
        Ok(chosen) => {
            after = apply_move(model_pos(model), chosen)
            complete_turn({ ..model, board: after.board, castling: after.castling, ep: after.ep, selected: 0, promo: 0, plies: model.plies + 1 })
        }

        Err(_) => { ..model, promo: 0 }
    }
}

promo_byte : U64 -> U8
promo_byte = |choice|
    if choice == 0 {
        81
    } else if choice == 1 {
        82
    } else if choice == 2 {
        66
    } else {
        78
    }

promo_letter : U64 -> Str
promo_letter = |choice|
    if choice == 0 {
        "Q"
    } else if choice == 1 {
        "R"
    } else if choice == 2 {
        "B"
    } else {
        "N"
    }

## The human's move is on the board: if black is mated the game is
## won, if black is stalemated it is drawn; otherwise the AI replies
## at once, and if that mates white the game is lost (stalemate
## again draws).
complete_turn : Model -> Model
complete_turn = |model| {
    black_status = position_status(model_pos(model), "black")
    if black_status == "checkmate" {
        { ..model, phase: "won" }
    } else if black_status == "stalemate" {
        { ..model, phase: "draw" }
    } else {
        choice = ai_choose(model_pos(model), model.rng)
        after = apply_move(model_pos(model), choice.move)
        settled = { ..model, board: after.board, castling: after.castling, ep: after.ep, rng: choice.rng, plies: model.plies + 1 }
        white_status = position_status(model_pos(settled), "white")
        if white_status == "checkmate" {
            { ..settled, phase: "lost" }
        } else if white_status == "stalemate" {
            { ..settled, phase: "draw" }
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
## move (an en-passant victim counts, though it does not sit on the
## landing square), the promotion bonus, and a whisper for advancing
## a pawn. Whether the moved piece hangs, whether the move opens a
## line, what the opponent threatens — none of it is considered.
score_move : Pos, Move -> U64
score_move = |pos, move| {
    piece = cell_at(pos.board, move.from)
    landed_cell = cell_at(pos.board, move.to)
    captured_value =
        if is_pawn(piece) and pos.ep < 64 and move.to == pos.ep and landed_cell == empty_cell and (move.to % 8) != (move.from % 8) {
            100
        } else {
            piece_value(landed_cell)
        }
    promo_bonus = if move.promo != 0 { piece_value(move.promo) - 100 } else { 0 }
    advance = if piece == 112 { (move.to / 8) * 2 } else { 0 }
    captured_value + promo_bonus + advance
}

## Pick black's move. One draw in four plays a uniformly random
## legal move (the blunder draw). Otherwise the highest one-ply
## score wins, with uniform jitter of 0..40 on every score — enough
## that the AI regularly prefers a worse capture or a pointless
## advance, and never enough to make it look ahead.
ai_choose : Pos, U64 -> { move : Move, rng : U64 }
ai_choose = |pos, rng| {
    moves = legal_moves(pos, "black")
    fallback = { from: 0, to: 0, promo: 0 }
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
                score = score_move(pos, move) + (drawn % 41)
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

## Uppercase letters are white (the human), lowercase are black (the
## AI). A dot marks an empty dark square.
glyph_for : U8, U64 -> Str
glyph_for = |cell, index|
    if cell == 75 {
        " K "
    } else if cell == 81 {
        " Q "
    } else if cell == 82 {
        " R "
    } else if cell == 66 {
        " B "
    } else if cell == 78 {
        " N "
    } else if cell == 80 {
        " P "
    } else if cell == 107 {
        " k "
    } else if cell == 113 {
        " q "
    } else if cell == 114 {
        " r "
    } else if cell == 98 {
        " b "
    } else if cell == 110 {
        " n "
    } else if cell == 112 {
        " p "
    } else if (index / 8 + index % 8) % 2 == 1 {
        " . "
    } else {
        "   "
    }

style_for : U8 -> { fg : U8, bg : U8, bold : Bool }
style_for = |cell|
    if is_white(cell) {
        { fg: 231, bg: 0, bold: Bool.True }
    } else if is_black(cell) {
        { fg: 245, bg: 0, bold: Bool.False }
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
        "YOU WIN BY CHECKMATE!"
    } else if model.phase == "lost" {
        "CHECKMATE - AI WINS"
    } else if model.phase == "draw" {
        "STALEMATE - DRAW"
    } else if model.promo == 1 {
        "PROMOTE: ${promo_letter(model.promo_choice)} (arrows+ENTER)"
    } else if model.selected == 1 {
        "selected: pick a landing"
    } else {
        "your move: white to play"
    }

help_text : Model -> Str
help_text = |model|
    if model.phase == "playing" {
        "arrows move, ENTER select"
    } else {
        "ENTER for a new game"
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
        { key: "promo", value: RocJson.Number(model.promo.to_str()) },
        { key: "promo_choice", value: RocJson.Number(model.promo_choice.to_str()) },
        { key: "castling", value: RocJson.Number(model.castling.to_str()) },
        { key: "ep", value: RocJson.Number(model.ep.to_str()) },
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
                phase: if phase == "won" or phase == "lost" or phase == "draw" { phase } else { "playing" },
                board: board_field(event),
                cursor: clamp(num_field(event, "cursor", fresh.cursor), 0, 63),
                selected: if num_field(event, "selected", fresh.selected) == 1 { 1 } else { 0 },
                sel: clamp(num_field(event, "sel", fresh.sel), 0, 63),
                promo: if num_field(event, "promo", fresh.promo) == 1 { 1 } else { 0 },
                promo_choice: clamp(num_field(event, "promo_choice", fresh.promo_choice), 0, 3),
                castling: clamp(num_field(event, "castling", fresh.castling), 0, 15),
                ep: clamp(num_field(event, "ep", fresh.ep), 0, 64),
                rng: num_field(event, "rng", fresh.rng),
                plies: num_field(event, "plies", fresh.plies),
            }
        }
    }

## A restored board must be exactly 64 cells of piece bytes; anything
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

first_or_empty : List(Move) -> Move
first_or_empty = |moves|
    match List.first(moves) {
        Ok(move) => move
        Err(_) => { from: 0, to: 0, promo: 0 }
    }

## Play the named move if it is legal in the position; an illegal
## "move" returns the position unchanged, so the expects below fail
## loudly if a line they rely on is not actually playable.
play : Pos, Str, U64, U64, U8 -> Pos
play = |pos, side, from, to, promo| {
    found = List.keep_if(legal_moves(pos, side), |move| move.from == from and move.to == to and move.promo == promo)
    match List.first(found) {
        Ok(move) => apply_move(pos, move)
        Err(_) => pos
    }
}

expect Str.count_utf8_bytes(start_board) == 64
expect count_side(start_board, "white") == 16 and count_side(start_board, "black") == 16
expect init(0).phase == "playing" and init(0).cursor == 52

# The opening position gives each side the standard twenty moves,
# and a double push sets the en-passant target on the skipped square.
expect List.len(legal_moves(start_pos, "white")) == 20
expect List.len(legal_moves(start_pos, "black")) == 20
expect play(start_pos, "white", 52, 36, 0).ep == 44
expect cell_at(play(start_pos, "white", 52, 36, 0).board, 36) == 80

# Cursor movement clamps at the board edges.
expect apply_key(init(0), Up).cursor == 44
expect apply_key(init(0), Down).cursor == 60

# A host tick is not a subscription this game holds; it changes nothing.
expect update(init(0), 0, "{\"cap\":\"time.tick\",\"interval\":1000,\"now\":1}") == init(0)

# --- scholar's mate: a forced mate line -----------------------------------------

mate1 = play(start_pos, "white", 52, 36, 0)
mate2 = play(mate1, "black", 12, 28, 0)
mate3 = play(mate2, "white", 61, 34, 0)
mate4 = play(mate3, "black", 1, 18, 0)
mate5 = play(mate4, "white", 59, 39, 0)
mate6 = play(mate5, "black", 6, 21, 0)
mate7 = play(mate6, "white", 39, 13, 0)

expect position_status(mate6, "white") == "playing"
expect in_check(mate7.board, "black") and position_status(mate7, "black") == "checkmate"
expect count_side(mate7.board, "black") == 15

# --- castling ---------------------------------------------------------------------

castle_board = board_from_rows([
    "....k...",
    "........",
    "........",
    "........",
    "........",
    "........",
    "........",
    "R...K..R",
])

castle_pos = { board: castle_board, castling: 15, ep: 64 }
castle_moves_white = legal_moves(castle_pos, "white")

expect List.any(castle_moves_white, |move| move.from == 60 and move.to == 62)
expect List.any(castle_moves_white, |move| move.from == 60 and move.to == 58)

castled = play(castle_pos, "white", 60, 62, 0)

expect cell_at(castled.board, 62) == 75 and cell_at(castled.board, 61) == 82
expect cell_at(castled.board, 60) == empty_cell and cell_at(castled.board, 63) == empty_cell
expect castled.castling == 12

# A rook on the f-file covers f1, so kingside castling — which must
# cross f1 — is illegal, while queenside castling stays legal.
castle_threat_board = board_from_rows([
    "....kr..",
    "........",
    "........",
    "........",
    "........",
    "........",
    "........",
    "R...K..R",
])

castle_threat_moves = legal_moves({ board: castle_threat_board, castling: 15, ep: 64 }, "white")

expect !List.any(castle_threat_moves, |move| move.from == 60 and move.to == 62)
expect List.any(castle_threat_moves, |move| move.from == 60 and move.to == 58)

# --- en passant ---------------------------------------------------------------------

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

ep_pos = { board: ep_board, castling: 0, ep: 19 }

expect List.any(legal_moves(ep_pos, "white"), |move| move.from == 28 and move.to == 19)

ep_after = play(ep_pos, "white", 28, 19, 0)

expect cell_at(ep_after.board, 19) == 80
expect cell_at(ep_after.board, 27) == empty_cell and cell_at(ep_after.board, 28) == empty_cell
expect ep_after.ep == 64

# --- promotion ------------------------------------------------------------------------

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

promo_pos = { board: promo_board, castling: 0, ep: 64 }
promo_from_a7 = List.keep_if(legal_moves(promo_pos, "white"), |move| move.from == 8)

# The push to a8 expands into exactly the four promotion choices.
expect List.len(promo_from_a7) == 4
expect cell_at(play(promo_pos, "white", 8, 0, 81).board, 0) == 81
expect cell_at(play(promo_pos, "white", 8, 0, 78).board, 0) == 78

# The same promotion driven through the cursor: confirming the
# landing opens the choice, Right cycles queen to rook, Enter plays
# it, and the AI answers (its king must flee the new rook's rank).
promo_start = { phase: "playing", board: board_from_rows(["....k..p", "P.......", "........", "........", "........", "........", "........", "....K..."]), cursor: 0, selected: 1, sel: 8, promo: 0, promo_choice: 0, castling: 0, ep: 64, rng: 1, plies: 0 }
promo_open = confirm(promo_start)
promo_cycled = apply_key(promo_open, Right)
promo_done = apply_key(promo_cycled, Enter)

expect promo_open.promo == 1 and promo_cycled.promo_choice == 1
expect cell_at(promo_done.board, 0) == 82 and promo_done.phase == "playing" and promo_done.plies == 2

# --- stalemate --------------------------------------------------------------------------

stale_board = board_from_rows([
    "k.......",
    "........",
    ".Q......",
    "........",
    "........",
    "........",
    "........",
    "....K...",
])

stale_pos = { board: stale_board, castling: 0, ep: 64 }

expect !in_check(stale_pos.board, "black")
expect position_status(stale_pos, "black") == "stalemate"

# --- pins ---------------------------------------------------------------------------------

pin_board = board_from_rows([
    "k...r...",
    "........",
    "........",
    "........",
    "........",
    "........",
    "....N...",
    "....K...",
])

pin_pos = { board: pin_board, castling: 0, ep: 64 }

# The knight on e2 shields its king from the rook on e8: every
# knight move would expose check, so it has no legal move at all.
expect List.is_empty(List.keep_if(legal_moves(pin_pos, "white"), |move| move.from == 52))

# --- the weak AI ----------------------------------------------------------------------------

ai_board = board_from_rows([
    "....k...",
    "........",
    "........",
    "...p....",
    "....Q...",
    "........",
    "........",
    "....K...",
])

ai_pos = { board: ai_board, castling: 0, ep: 64 }

# One ply of greed: capturing the queen dwarfs any quiet push.
expect score_move(ai_pos, { from: 27, to: 36, promo: 0 }) > score_move(ai_pos, { from: 27, to: 35, promo: 0 }) + 100

ai_open = ai_choose(start_pos, 7)

# Whatever the jitter or the blunder draw picks, it is a legal move.
expect List.any(legal_moves(start_pos, "black"), |move| move == ai_open.move)

# --- a full opening move through the keys ------------------------------------------------------

picked = apply_key(init(0), Enter)
opened = List.fold([Enter, Up, Up, Enter], init(0), apply_key)

expect picked.selected == 1
expect confirm(picked).selected == 0
expect opened.plies == 2 and opened.phase == "playing"
expect cell_at(opened.board, 36) == 80 and cell_at(opened.board, 52) == empty_cell
expect count_side(opened.board, "white") == 16 and count_side(opened.board, "black") == 16

# --- snapshot round-trip --------------------------------------------------------------------------

mid_game = { phase: "playing", board: start_board, cursor: 36, selected: 0, sel: 0, promo: 0, promo_choice: 0, castling: 15, ep: 44, rng: 12345, plies: 1 }

expect restore(snapshot(mid_game)) == mid_game
expect restore("not json at all") == init(0)
