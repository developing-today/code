app [program] { pf: platform "../roc-world/platform.roc" }

import pf.RocGrid
import pf.Screen

# A checked-in example for `pf.RocGrid` and `Screen.put_grid`: a 4x3 board is
# drawn as a portable screen inside a box, with a status line reporting counts
# and an out-of-bounds read served by its fallback.

Model : { grid : RocGrid }

program = { init, update, view, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { grid: RocGrid.new(4, 3, 46) }

update : Model, U64, Str -> Model
update = |model, _pid, ev|
    if ev == "set" and RocGrid.in_bounds(model.grid, 1, 1) {
        { grid: RocGrid.set(model.grid, 1, 1, 65) }
    } else {
        model
    }

view : Model -> Str
view = |model| {
    screen = Screen.boxed(Screen.new(24, 6), 0, 0, 7, 5, "grid")
    drawn = Screen.put_grid(screen, 1, 1, model.grid, glyph, { fg: 2, bg: 0, bold: Bool.True })
    cursor = RocGrid.get(model.grid, 1, 1, 63)
    outside = RocGrid.get(model.grid, 9, 9, 63)
    info = "cells=${RocGrid.count(model.grid, 46).to_str()} at11=${cursor.to_str()} out=${outside.to_str()}"
    Screen.render_json(Screen.put_text(drawn, 0, 5, info, { fg: 0, bg: 0, bold: Bool.False }))
}

glyph : U8 -> Str
glyph = |byte| match Str.from_utf8([byte]) {
    Ok(text) => text
    Err(_) => "?"
}

records : Model -> Str
records = |_model| "{}"

wants : Model -> Str
wants = |_model| "{}"

snapshot : Model -> Str
snapshot = |model| bytes_to_str(RocGrid.bytes(model.grid))

restore : Str -> Model
restore = |text| { grid: RocGrid.from_cells(4, 3, Str.to_utf8(text), 46) }

bytes_to_str : List(U8) -> Str
bytes_to_str = |bytes| match Str.from_utf8(bytes) {
    Ok(text) => text
    Err(_) => ""
}
