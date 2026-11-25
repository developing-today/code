app [program] { pf: platform "../../../examples/roc-world/platform.roc" }

import pf.RocJson
import pf.Screen
import pf.Key

# Space Invaders, as a Roc world program (#14938, slice 4 of #14861).
#
# One player defends the bottom of a 40x20 field against a 4x8 alien
# formation that marches sideways, drops a row at each edge, and drops
# bombs. The game advances on the host's `time.tick:1000` subscription;
# the player moves and fires with key events. Everything is a pure
# function of (model, event): ticks and keystrokes are journaled like
# any other world input, the bomb pattern comes from a xorshift PRNG
# carried in the model, and the journal replays a game exactly.
#
# Controls: Left/Right arrows or A/D move, Space or Up fires, Enter (or
# R) starts the next wave after a clear and retries after a loss.
#
# The frame is data: `view`/`layout` render a `Screen` and the shell
# paints it in whichever presentation mode the session negotiated
# (alternate, primary, or low-write text, #14936/#14937). The game
# never changes when the mode does.

Model : {
    phase : Str,
    score : U64,
    lives : U64,
    wave : U64,
    ticks : U64,
    rng : U64,
    player_x : U64,
    dir : U64,
    ox : U64,
    oy : U64,
    aliens : Str,
    bullet_on : U64,
    bullet_x : U64,
    bullet_y : U64,
    bombs : List({ x : U64, y : U64 }),
}

program = { init, update, view, layout, records, wants, snapshot, restore }

# --- field geometry ----------------------------------------------------------

field_width : U64
field_width = 40

field_rows : U64
field_rows = 20

player_row : U64
player_row = 19

alien_rows : U64
alien_rows = 4

alien_cols : U64
alien_cols = 8

frame_height : U64
frame_height = 22

## The full formation: alien_rows x alien_cols live cells, row-major.
formation : Str
formation = "11111111111111111111111111111111"

# --- lifecycle ---------------------------------------------------------------

init : U64 -> Model
init = |seed| {
    rng = if seed == 0 { 2685821657736338717 } else { seed }
    {
        phase: "playing",
        score: 0,
        lives: 3,
        wave: 1,
        ticks: 0,
        rng: rng,
        player_x: 20,
        dir: 1,
        ox: 1,
        oy: 1,
        aliens: formation,
        bullet_on: 0,
        bullet_x: 0,
        bullet_y: 0,
        bombs: [],
    }
}

## A fresh formation for `wave`, keeping score, lives, and the PRNG.
wave_start : Model, U64 -> Model
wave_start = |model, wave| {
    oy = if wave < 4 { wave } else { 4 }
    {
        phase: "playing",
        score: model.score,
        lives: model.lives,
        wave: wave,
        ticks: model.ticks,
        rng: model.rng,
        player_x: 20,
        dir: 1,
        ox: 1,
        oy: oy,
        aliens: formation,
        bullet_on: 0,
        bullet_x: 0,
        bullet_y: 0,
        bombs: [],
    }
}

# --- update ------------------------------------------------------------------

update : Model, U64, Str -> Model
update = |model, _pid, ev|
    match RocJson.decode(ev) {
        Ok(event) =>
            match RocJson.get(event, "cap") {
                Ok(cap_value) =>
                    match RocJson.as_string(cap_value) {
                        Ok(name) => if name == "time.tick" { tick(model) } else { model }
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
            Left | Char("a") | Char("A") => move_player(model, 0)
            Right | Char("d") | Char("D") => move_player(model, 1)
            Up | Char(" ") => fire(model)
            _ => model
        }
    } else {
        match key {
            Enter | Char("r") | Char("R") => advance_after_end(model)
            _ => model
        }
    }

## After a wave clear the next wave starts; after a loss, a fresh game.
advance_after_end : Model -> Model
advance_after_end = |model|
    if model.phase == "won" {
        wave_start(model, model.wave + 1)
    } else if model.phase == "lost" {
        wave_start({ ..model, score: 0, lives: 3 }, 1)
    } else {
        model
    }

move_player : Model, U64 -> Model
move_player = |model, dir|
    if dir == 0 {
        if model.player_x > 1 { { ..model, player_x: model.player_x - 1 } } else { model }
    } else {
        if model.player_x + 1 < field_width { { ..model, player_x: model.player_x + 1 } } else { model }
    }

fire : Model -> Model
fire = |model|
    if model.bullet_on == 1 {
        model
    } else {
        started = { ..model, bullet_on: 1, bullet_x: model.player_x, bullet_y: player_row - 1 }
        collide_bullet(started)
    }

# --- the tick ----------------------------------------------------------------

## One host tick: bullet, formation march, bombs, in that order.
tick : Model -> Model
tick = |model|
    if model.phase != "playing" {
        model
    } else {
        stepped = { ..model, ticks: model.ticks + 1 }
        after_bullet = advance_bullet(stepped)
        if after_bullet.phase != "playing" {
            after_bullet
        } else {
            after_march = march(after_bullet)
            if after_march.phase != "playing" {
                after_march
            } else {
                spawn_bomb(move_bombs(after_march))
            }
        }
    }

## The bullet climbs three cells per tick, checking each cell it enters.
advance_bullet : Model -> Model
advance_bullet = |model|
    if model.bullet_on == 0 {
        model
    } else {
        step1 = bullet_step(model)
        step2 = if step1.bullet_on == 1 { bullet_step(step1) } else { step1 }
        if step2.bullet_on == 1 { bullet_step(step2) } else { step2 }
    }

bullet_step : Model -> Model
bullet_step = |model|
    if model.bullet_y == 0 {
        { ..model, bullet_on: 0 }
    } else {
        collide_bullet({ ..model, bullet_y: model.bullet_y - 1 })
    }

## If the bullet's cell is covered by a live alien, the alien dies.
collide_bullet : Model -> Model
collide_bullet = |model|
    match alien_at(model, model.bullet_x, model.bullet_y) {
        Ok(index) => {
            killed = { ..model, aliens: set_alien(model.aliens, index, 0), bullet_on: 0, score: model.score + points_for_row(index / alien_cols) }
            if aliens_left(killed) == 0 { { ..killed, phase: "won" } } else { killed }
        }

        Err(_) => model
    }

## The formation marches one cell; at the field edge it drops a row and
## turns around. A formation whose lowest live row reaches the player
## row has invaded: the game is lost.
march : Model -> Model
march = |model| {
    extent = live_extent(model)
    if extent.count == 0 {
        model
    } else if model.dir == 1 and model.ox + extent.max_col * 4 + 1 >= field_width - 1 {
        descend(model, extent)
    } else if model.dir == 0 and model.ox == 0 {
        descend(model, extent)
    } else if model.dir == 1 {
        { ..model, ox: model.ox + 1 }
    } else {
        { ..model, ox: model.ox - 1 }
    }
}

descend : Model, { count : U64, min_col : U64, max_col : U64, max_row : U64 } -> Model
descend = |model, extent| {
    dropped = { ..model, oy: model.oy + 1, dir: if model.dir == 1 { 0 } else { 1 } }
    if dropped.oy + extent.max_row >= player_row {
        { ..dropped, phase: "lost" }
    } else {
        dropped
    }
}

## Bombs fall one cell per tick. A bomb that reaches the player's row
## inside the ship's span costs a life; one that falls past it is gone.
move_bombs : Model -> Model
move_bombs = |model| {
    fallen = List.map(model.bombs, |bomb| { x: bomb.x, y: bomb.y + 1 })
    hit = List.any(fallen, |bomb| bomb.y == player_row and bomb.x + 1 >= model.player_x and bomb.x <= model.player_x + 1)
    if hit {
        lives = if model.lives > 0 { model.lives - 1 } else { 0 }
        cleared = { ..model, bombs: [], bullet_on: 0, lives: lives }
        if lives == 0 { { ..cleared, phase: "lost" } } else { cleared }
    } else {
        { ..model, bombs: List.keep_if(fallen, |bomb| bomb.y < player_row) }
    }
}

## One bomb per tick at most, from a deterministic PRNG draw: roughly
## one tick in three, from the bottom alien of a drawn column.
spawn_bomb : Model -> Model
spawn_bomb = |model| {
    rng = xorshift(model.rng)
    drawn = { ..model, rng: rng }
    col = (rng / 8) % alien_cols
    if rng % 3 == 0 and List.len(model.bombs) < 3 {
        match bottom_alien(drawn, col) {
            Ok(alien) =>
                if alien.y + 1 < player_row {
                    { ..drawn, bombs: List.append(model.bombs, { x: alien.x + 1, y: alien.y + 1 }) }
                } else {
                    drawn
                }

            Err(_) => drawn
        }
    } else {
        drawn
    }
}

## xorshift64: shifts and xors only, so no arithmetic can overflow.
xorshift : U64 -> U64
xorshift = |state| {
    a = Num.bitwise_xor(state, Num.shift_left_by(state, 13))
    b = Num.bitwise_xor(a, Num.shift_right_zf_by(a, 7))
    Num.bitwise_xor(b, Num.shift_left_by(b, 17))
}

# --- aliens ------------------------------------------------------------------

## The index of the live alien covering field cell (x, y), if any.
## Alien (row, col) sits at (ox + col * 4, oy + row) and is two cells wide.
alien_at : Model, U64, U64 -> Result(U64, {})
alien_at = |model, x, y|
    if y < model.oy or y >= model.oy + alien_rows or x < model.ox {
        Err({})
    } else {
        row = y - model.oy
        offset = x - model.ox
        col = offset / 4
        if col >= alien_cols or offset % 4 > 1 {
            Err({})
        } else {
            index = row * alien_cols + col
            if alien_alive(model.aliens, index) { Ok(index) } else { Err({}) }
        }
    }

## The bottom-most live alien of a column, as a field position.
bottom_alien : Model, U64 -> Result({ x : U64, y : U64 }, {})
bottom_alien = |model, col|
    List.fold([3, 2, 1, 0], Err({}), |acc, row|
        match acc {
            Ok(_) => acc
            Err(_) =>
                if alien_alive(model.aliens, row * alien_cols + col) {
                    Ok({ x: model.ox + col * 4, y: model.oy + row })
                } else {
                    Err({})
                }
        })

## Live formation extent: columns and lowest row still alive.
live_extent : Model -> { count : U64, min_col : U64, max_col : U64, max_row : U64 }
live_extent = |model|
    List.fold(
        List.map_with_index(Str.to_utf8(model.aliens), |cell, index| { cell: cell, index: index }),
        { count: 0, min_col: alien_cols - 1, max_col: 0, max_row: 0 },
        |acc, entry|
            if entry.cell == 49 {
                col = entry.index % alien_cols
                row = entry.index / alien_cols
                {
                    count: acc.count + 1,
                    min_col: if col < acc.min_col { col } else { acc.min_col },
                    max_col: if col > acc.max_col { col } else { acc.max_col },
                    max_row: if row > acc.max_row { row } else { acc.max_row },
                }
            } else {
                acc
            },
    )

aliens_left : Model -> U64
aliens_left = |model| live_extent(model).count

alien_alive : Str, U64 -> Bool
alien_alive = |aliens, index|
    match List.get(Str.to_utf8(aliens), index) {
        Ok(cell) => cell == 49
        Err(_) => Bool.False
    }

set_alien : Str, U64, U64 -> Str
set_alien = |aliens, index, alive| {
    cells = Str.to_utf8(aliens)
    cell = if alive == 1 { 49 } else { 48 }
    match List.set(cells, index, cell) {
        Ok(updated) =>
            match Str.from_utf8(updated) {
                Ok(text) => text
                Err(_) => aliens
            }

        Err(_) => aliens
    }
}

## Row values, top row worth most.
points_for_row : U64 -> U64
points_for_row = |row|
    if row == 0 {
        30
    } else if row == 3 {
        10
    } else {
        20
    }

# --- rendering ---------------------------------------------------------------

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
    hud = "SCORE ${model.score.to_str()}  LIVES ${model.lives.to_str()}  WAVE ${model.wave.to_str()}"
    base = Screen.put_text(Screen.new(field_width, frame_height), 0, 0, hud, plain)
    with_aliens = List.fold(
        List.map_with_index(Str.to_utf8(model.aliens), |cell, index| { cell: cell, index: index }),
        base,
        |screen, entry|
            if entry.cell == 49 {
                row = entry.index / alien_cols
                col = entry.index % alien_cols
                Screen.put_text(screen, model.ox + col * 4, 1 + model.oy + row, glyph_for_row(row), { fg: color_for_row(row), bg: 0, bold: Bool.False })
            } else {
                screen
            },
    )
    with_bombs = List.fold(model.bombs, with_aliens, |screen, bomb| Screen.put_text(screen, bomb.x, 1 + bomb.y, ":", { fg: 201, bg: 0, bold: Bool.False }))
    with_bullet = if model.bullet_on == 1 {
        Screen.put_text(with_bombs, model.bullet_x, 1 + model.bullet_y, "|", { fg: 231, bg: 0, bold: Bool.True })
    } else {
        with_bombs
    }
    with_player = Screen.put_text(with_bullet, model.player_x - 1, 1 + player_row, "/A\\", { fg: 51, bg: 0, bold: Bool.True })
    grounded = Screen.rule(with_player, 0, frame_height - 1, field_width, "=", { fg: 240, bg: 0, bold: Bool.False })
    if model.phase == "won" {
        Screen.put_text(grounded, 5, 10, "WAVE CLEAR! ENTER = NEXT WAVE", { fg: 46, bg: 0, bold: Bool.True })
    } else if model.phase == "lost" {
        Screen.put_text(grounded, 7, 10, "GAME OVER - ENTER TO RETRY", { fg: 196, bg: 0, bold: Bool.True })
    } else {
        grounded
    }
}

glyph_for_row : U64 -> Str
glyph_for_row = |row|
    if row == 0 {
        "<>"
    } else if row == 1 {
        "/\\"
    } else if row == 2 {
        "[]"
    } else {
        "{}"
    }

color_for_row : U64 -> U8
color_for_row = |row|
    if row == 0 {
        196
    } else if row == 1 {
        208
    } else if row == 2 {
        226
    } else {
        46
    }

# --- records, wants, snapshot ------------------------------------------------

records : Model -> Str
records = |model| encode_or_crash(
    RocJson.Object([
        { key: "score", value: RocJson.Number(model.score.to_str()) },
        { key: "lives", value: RocJson.Number(model.lives.to_str()) },
        { key: "wave", value: RocJson.Number(model.wave.to_str()) },
        { key: "phase", value: RocJson.String(model.phase) },
        { key: "aliens_left", value: RocJson.Number(aliens_left(model).to_str()) },
        { key: "ticks", value: RocJson.Number(model.ticks.to_str()) },
    ]),
)

## The game runs on a one-second tick (the host's fastest default).
wants : Model -> Str
wants = |_model| encode_or_crash(
    RocJson.Object([
        { key: "v", value: RocJson.Number("1") },
        { key: "subscribe", value: RocJson.Array([RocJson.String("time.tick:1000")]) },
        { key: "requests", value: RocJson.Array([]) },
    ]),
)

## The whole model as one JSON object; `restore` reads it back with
## per-field defaults, so a checkpoint the game cannot parse restarts
## the current wave's shape rather than trapping.
snapshot : Model -> Str
snapshot = |model| encode_or_crash(
    RocJson.Object([
        { key: "phase", value: RocJson.String(model.phase) },
        { key: "score", value: RocJson.Number(model.score.to_str()) },
        { key: "lives", value: RocJson.Number(model.lives.to_str()) },
        { key: "wave", value: RocJson.Number(model.wave.to_str()) },
        { key: "ticks", value: RocJson.Number(model.ticks.to_str()) },
        { key: "rng", value: RocJson.Number(model.rng.to_str()) },
        { key: "player_x", value: RocJson.Number(model.player_x.to_str()) },
        { key: "dir", value: RocJson.Number(model.dir.to_str()) },
        { key: "ox", value: RocJson.Number(model.ox.to_str()) },
        { key: "oy", value: RocJson.Number(model.oy.to_str()) },
        { key: "aliens", value: RocJson.String(model.aliens) },
        { key: "bullet_on", value: RocJson.Number(model.bullet_on.to_str()) },
        { key: "bullet_x", value: RocJson.Number(model.bullet_x.to_str()) },
        { key: "bullet_y", value: RocJson.Number(model.bullet_y.to_str()) },
        { key: "bombs", value: RocJson.Array(List.map(model.bombs, |bomb| RocJson.Array([RocJson.Number(bomb.x.to_str()), RocJson.Number(bomb.y.to_str())]))) },
    ]),
)

restore : Str -> Model
restore = |text|
    match RocJson.decode(text) {
        Err(_) => init(0)
        Ok(event) => {
            fresh = init(0)
            aliens = string_field(event, "aliens")
            phase = string_field(event, "phase")
            {
                phase: if phase == "won" or phase == "lost" { phase } else { "playing" },
                score: num_field(event, "score", fresh.score),
                lives: num_field(event, "lives", fresh.lives),
                wave: num_field(event, "wave", fresh.wave),
                ticks: num_field(event, "ticks", fresh.ticks),
                rng: num_field(event, "rng", fresh.rng),
                player_x: clamp(num_field(event, "player_x", fresh.player_x), 1, field_width - 2),
                dir: if num_field(event, "dir", fresh.dir) == 1 { 1 } else { 0 },
                ox: clamp(num_field(event, "ox", fresh.ox), 0, field_width - 2),
                oy: clamp(num_field(event, "oy", fresh.oy), 0, player_row - alien_rows),
                aliens: if Str.count_utf8_bytes(aliens) == 32 { aliens } else { formation },
                bullet_on: if num_field(event, "bullet_on", fresh.bullet_on) == 1 { 1 } else { 0 },
                bullet_x: num_field(event, "bullet_x", fresh.bullet_x),
                bullet_y: num_field(event, "bullet_y", fresh.bullet_y),
                bombs: bombs_field(event),
            }
        }
    }

bombs_field = |event|
    match RocJson.get(event, "bombs") {
        Ok(value) =>
            match RocJson.as_array(value) {
                Ok(items) =>
                    List.keep_if(
                        List.map(items, |item|
                            match RocJson.as_array(item) {
                                Ok(pair) => {
                                    x = match List.get(pair, 0) {
                                        Ok(coord) => num_value(coord)
                                        Err(_) => 0
                                    }
                                    y = match List.get(pair, 1) {
                                        Ok(coord) => num_value(coord)
                                        Err(_) => 0
                                    }
                                    { x: x, y: y }
                                }

                                Err(_) => { x: 0, y: 0 }
                            }),
                        |bomb| bomb.x < field_width and bomb.y < field_rows,
                    )

                Err(_) => []
            }

        Err(_) => []
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
    Err(_) => crash "space-invaders attempted to encode an invalid or oversized JSON value"
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

# --- tests -------------------------------------------------------------------

expect aliens_left(init(0)) == 32
expect init(0).lives == 3
expect init(0).phase == "playing"

# Firing starts a bullet just above the ship; a second fire is a no-op.
expect fire(init(0)).bullet_on == 1
expect fire(init(0)).bullet_y == player_row - 1
expect fire(fire(init(0))).bullet_y == player_row - 1

# The player clamps at both field edges.
expect List.fold(List.repeat(Left, 30), init(0), apply_key).player_x == 1
expect List.fold(List.repeat(Right, 30), init(0), apply_key).player_x == field_width - 2

# A tick event is JSON; a key event is not. Both route through `update`.
expect update(init(0), 0, "{\"cap\":\"time.tick\",\"interval\":1000,\"now\":1}").ticks == 1
expect update(init(0), 0, " ").bullet_on == 1
expect update(init(0), 0, "\u(1b)[D").player_x == 19

# A bullet fired up a clear column leaves the field and despawns.
# (The pinned compiler takes single-expression expects in app files —
# the platform libraries' convention — so staged scenarios are named
# defs and the expects assert over them.)
cleared_column : Model
cleared_column = List.fold(List.repeat(0, 8), fire({ ..init(0), player_x: 2, ox: 20 }), |model, _| tick(model))

expect cleared_column.bullet_on == 0 and aliens_left(cleared_column) == 32

# Marching right, the formation turns around by descending, not by
# walking off the field.
marched_12 : Model
marched_12 = List.fold(List.repeat(0, 12), init(0), |model, _| tick(model))

expect marched_12.oy == 2 and marched_12.dir == 0

# Killing the last alien wins the wave; Enter starts the next one.
lone_shot : Model
lone_shot = collide_bullet({ ..init(0), aliens: "10000000000000000000000000000000", score: 0, bullet_on: 1, bullet_x: 2, bullet_y: 1 })

expect lone_shot.phase == "won" and lone_shot.score == 30

next_wave : Model
next_wave = advance_after_end({ ..init(0), phase: "won", score: 500, wave: 1 })

expect next_wave.phase == "playing" and next_wave.wave == 2 and next_wave.score == 500 and aliens_left(next_wave) == 32

# A bomb on the ship's span costs a life and clears the sky.
bombed : Model
bombed = move_bombs({ ..init(0), bombs: [{ x: 20, y: player_row - 1 }] })

expect bombed.lives == 2 and List.is_empty(bombed.bombs)

# Losing the last life ends the game.
expect move_bombs({ ..init(0), lives: 1, bombs: [{ x: 20, y: player_row - 1 }] }).phase == "lost"

# Snapshot/restore round-trips a mid-game model exactly, and restore
# is total on garbage.
mid_game : Model
mid_game = spawn_bomb(tick(fire({ ..init(7), player_x: 13, score: 120 })))

expect snapshot(restore(snapshot(mid_game))) == snapshot(mid_game)

expect restore("not json at all").phase == "playing"
expect restore("not json at all").score == 0
