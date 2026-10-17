app [program] { pf: platform "../roc-world/platform.roc" }

import pf.RocGrid

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
view = |model| "cells=${RocGrid.count(model.grid, 46).to_str()} cursor=${RocGrid.get(model.grid, 1, 1, 63).to_str()} outside=${RocGrid.get(model.grid, 9, 9, 63).to_str()}"

records : Model -> Str
records = |_model| "{}"

wants : Model -> Str
wants = |_model| "{}"

snapshot : Model -> Str
snapshot = |model| "${model.grid.width.to_str()},${model.grid.height.to_str()}"

restore : Str -> Model
restore = |_text| { grid: RocGrid.new(4, 3, 46) }
