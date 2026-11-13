app [program] { pf: platform "../roc-world/platform.roc" }

## The no-Screen legacy-text world: this program imports no platform
## modules at all. `view` returns plain UTF-8 text (`count=N`), which the
## host presents as-is instead of a screen document.

Model := { count : I64 }

program = { init, update, view, layout, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { count: 0 }

## Events are matched exactly: only "inc" increments.
update : Model, U64, Str -> Model
update = |model, _pid, ev| if ev == "inc" { count: model.count + 1 } else model

## The counter renders as plain text, not a screen document.
view : Model -> Str
view = |model| "count=${model.count.to_str()}"

## Legacy text does not reflow: every viewport gets the same plain text.
layout : Model, U64, U64 -> Str
layout = |model, _width, _height| "count=${model.count.to_str()}"

## Structured data the host stores and replicates: a JSON object of records.
records : Model -> Str
records = |model| "{\"count\":${model.count.to_str()}}"

## This program wants nothing from the server.
wants : Model -> Str
wants = |_model| "{}"

## The model as text. `restore(snapshot(m))` must behave exactly like `m`; the
## host checks this before it trims the world's journal.
snapshot : Model -> Str
snapshot = |model| model.count.to_str()

restore : Str -> Model
restore = |text| match I64.from_str(text) {
    Ok(count) => { count: count }
    Err(_) => { count: 0 }
}
