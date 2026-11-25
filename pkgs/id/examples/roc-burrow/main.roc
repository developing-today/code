app [program] { pf: platform "../roc-world/platform.roc" }

import pf.RocJson
import pf.Screen

# A burrow for burrow/detour-to-exit (#13603): a three-cell excavation and
# exit loop on a 4x3 board. Dig enters orthogonally adjacent soil and clears
# it exactly once; the rock at (3,1) is impassable; the exit at (3,2) opens
# only once three soil cells have been cleared. The level configuration is
# fixed and immutable: rock, exit and quota are constants of the guest, and
# the terrain under the digger is never stored as a glyph.
#
# Each player event is one JSON command, {"kind":"Dig"|"Walk",
# "direction":"N"|"S"|"E"|"W", ...}. The "event" tag a caller may carry is
# host journal deduplication metadata; the guest does not read it. Host
# events (participant 0) are ignored. `records` publishes the domain state
# (digger, remaining soil, cleared count, phase, last outcome) and
# `snapshot` is the same document, so a snapshot restored into a fresh
# instance compares byte-identical through the regular host. The view is a
# semantic text grid: A is the digger, s is soil, # is the rock, X is the
# exit and . is open floor, with the remaining soil and quota as text.

Pos : { x : U64, y : U64 }
Model : { digger : Pos, soil : List(Pos), cleared : U64, phase : Str, outcome : Str }

program = { init, update, view, layout, records, wants, snapshot, restore }

board_width : U64
board_width = 4

board_height : U64
board_height = 3

rock_pos : Pos
rock_pos = { x: 3, y: 1 }

exit_pos : Pos
exit_pos = { x: 3, y: 2 }

required_clears : U64
required_clears = 3

init : U64 -> Model
init = |_seed| initial

initial : Model
initial = {
    digger: { x: 0, y: 1 },
    soil: [{ x: 1, y: 1 }, { x: 2, y: 1 }, { x: 2, y: 2 }],
    cleared: 0,
    phase: "digging",
    outcome: "",
}

update : Model, U64, Str -> Model
update = |model, pid, ev|
    if pid == 0 {
        model
    } else {
        command(model, ev)
    }

command : Model, Str -> Model
command = |model, text| match RocJson.decode(text) {
    Err(_) => { ..model, outcome: "invalid" }
    Ok(cmd) => {
        kind = string_field(cmd, "kind")
        direction = string_field(cmd, "direction")
        if kind == "Dig" {
            dig(model, direction)
        } else if kind == "Walk" {
            walk(model, direction)
        } else {
            { ..model, outcome: "invalid" }
        }
    }
}

dig : Model, Str -> Model
dig = |model, direction|
    if model.phase == "escaped" {
        { ..model, outcome: "refused" }
    } else if !is_direction(direction) {
        { ..model, outcome: "invalid" }
    } else match target_of(model.digger, direction) {
        Outside => { ..model, outcome: "blocked-edge" }
        Inside(target) =>
            if target == rock_pos {
                { ..model, outcome: "blocked-rock" }
            } else if target == exit_pos {
                enter_exit(model, target)
            } else if has_soil(model.soil, target) {
                {
                    ..model,
                    digger: target,
                    soil: List.keep_if(model.soil, |cell| cell != target),
                    cleared: model.cleared + 1,
                    outcome: "cleared",
                }
            } else {
                { ..model, outcome: "refused" }
            }
    }

walk : Model, Str -> Model
walk = |model, direction|
    if model.phase == "escaped" {
        { ..model, outcome: "refused" }
    } else if !is_direction(direction) {
        { ..model, outcome: "invalid" }
    } else match target_of(model.digger, direction) {
        Outside => { ..model, outcome: "blocked-edge" }
        Inside(target) =>
            if target == rock_pos {
                { ..model, outcome: "blocked-rock" }
            } else if target == exit_pos {
                enter_exit(model, target)
            } else if has_soil(model.soil, target) {
                { ..model, outcome: "blocked-soil" }
            } else {
                { ..model, digger: target, outcome: "moved" }
            }
    }

# The exit accepts entry only once the clear quota is met; before that it
# is a locked cell and entering it changes nothing.
enter_exit : Model, Pos -> Model
enter_exit = |model, target|
    if model.cleared >= required_clears {
        { ..model, digger: target, phase: "escaped", outcome: "escaped" }
    } else {
        { ..model, outcome: "blocked-exit" }
    }

is_direction : Str -> Bool
is_direction = |direction| direction == "N" or direction == "S" or direction == "E" or direction == "W"

target_of : Pos, Str -> [Inside(Pos), Outside]
target_of = |from, direction|
    if direction == "E" and from.x + 1 < board_width {
        Inside({ x: from.x + 1, y: from.y })
    } else if direction == "W" and from.x > 0 {
        Inside({ x: from.x - 1, y: from.y })
    } else if direction == "S" and from.y + 1 < board_height {
        Inside({ x: from.x, y: from.y + 1 })
    } else if direction == "N" and from.y > 0 {
        Inside({ x: from.x, y: from.y - 1 })
    } else {
        Outside
    }

has_soil : List(Pos), Pos -> Bool
has_soil = |soil, pos| List.any(soil, |cell| cell == pos)

plain = { fg: 0, bg: 0, bold: Bool.False }

view : Model -> Str
view = |model| {
    frame = Screen.boxed(Screen.new(36, 9), 0, 0, 36, 9, "burrow")
    grid = List.fold([0, 1, 2], frame, |screen, y|
        Screen.put_text(screen, 2, y + 2, row_text(model, y), plain))
    status = "soil left=${List.len(model.soil).to_str()} cleared=${model.cleared.to_str()}/${required_clears.to_str()}"
    detail = "phase=${model.phase} last=${model.outcome}"
    Screen.render_json(
        Screen.put_text(Screen.put_text(grid, 2, 6, status, plain), 2, 7, detail, plain),
    )
}

row_text : Model, U64 -> Str
row_text = |model, y| Str.join_with(List.map([0, 1, 2, 3], |x| glyph(model, { x, y })), " ")

glyph : Model, Pos -> Str
glyph = |model, pos|
    if pos == model.digger {
        "A"
    } else if pos == rock_pos {
        "#"
    } else if pos == exit_pos {
        "X"
    } else if has_soil(model.soil, pos) {
        "s"
    } else {
        "."
    }

## This program renders the same layout at any viewport.
layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

records : Model -> Str
records = |model| encode_or_crash(document(model))

## No subscriptions and no requests.
wants : Model -> Str
wants = |_model| "{}"

snapshot : Model -> Str
snapshot = |model| encode_or_crash(document(model))

restore : Str -> Model
restore = |text| match RocJson.decode(text) {
    Err(_) => initial
    Ok(doc) => match RocJson.as_array(field(doc, "digger")) {
        Err(_) => initial
        Ok(_) => {
            digger: pos_of(doc, "digger"),
            soil: List.map(array_field(doc, "soil"), pos_item),
            cleared: number_field(doc, "cleared_count"),
            phase: string_field(doc, "phase"),
            outcome: string_field(doc, "outcome"),
        }
    }
}

# --- helpers ---------------------------------------------------------------

document : Model -> RocJson
document = |model| RocJson.Object([
    { key: "digger", value: pos_json(model.digger) },
    { key: "soil", value: RocJson.Array(List.map(model.soil, pos_json)) },
    { key: "cleared_count", value: RocJson.Number(model.cleared.to_str()) },
    { key: "phase", value: RocJson.String(model.phase) },
    { key: "outcome", value: RocJson.String(model.outcome) },
])

pos_json : Pos -> RocJson
pos_json = |p| RocJson.Array([RocJson.Number(p.x.to_str()), RocJson.Number(p.y.to_str())])

pos_of : RocJson, Str -> Pos
pos_of = |doc, key| match RocJson.as_array(field(doc, key)) {
    Ok(items) => { x: item_number(items, 0), y: item_number(items, 1) }
    Err(_) => { x: 0, y: 0 }
}

pos_item : RocJson -> Pos
pos_item = |item| match RocJson.as_array(item) {
    Ok(items) => { x: item_number(items, 0), y: item_number(items, 1) }
    Err(_) => { x: 0, y: 0 }
}

item_number : List(RocJson), U64 -> U64
item_number = |items, index| match List.get(items, index) {
    Ok(found) => number(found)
    Err(_) => 0
}

encode_or_crash = |value| match RocJson.encode(value) {
    Ok(text) => text
    Err(_) => crash "burrow attempted to encode an invalid or oversized JSON value"
}

field = |value, key| match RocJson.get(value, key) {
    Ok(found) => found
    Err(_) => RocJson.Null
}

string_field = |value, key| match RocJson.as_string(field(value, key)) {
    Ok(text) => text
    Err(_) => ""
}

array_field = |value, key| match RocJson.as_array(field(value, key)) {
    Ok(items) => items
    Err(_) => []
}

number_field = |value, key| number(field(value, key))

number = |value| match RocJson.as_number(value) {
    Ok(text) => match U64.from_str(text) {
        Ok(found) => found
        Err(_) => 0
    }
    Err(_) => 0
}
