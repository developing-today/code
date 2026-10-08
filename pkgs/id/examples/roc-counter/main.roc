app [program] { pf: platform "../roc-world/platform.roc" }

Model := { count : I64 }

program = { init, update, view, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { count: 0 }

update : Model, U64, Str -> Model
update = |model, _pid, ev| if ev == "inc" { count: model.count + 1 } else model

view : Model -> Str
view = |model| Str.concat("count=", model.count.to_str())

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
