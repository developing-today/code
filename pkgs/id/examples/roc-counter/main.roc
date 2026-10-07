app [program] { pf: platform "./platform.roc" }

Model := { count : I64 }

program = { init, update, view, records }

init : U64 -> Model
init = |_seed| { count: 0 }

update : Model, Str -> Model
update = |model, ev| if ev == "inc" { count: model.count + 1 } else model

view : Model, Str -> Str
view = |model, _viewer| "count=${model.count.to_str()}"

## Structured data the host stores and replicates: a JSON object of records.
records : Model -> Str
records = |model| "{\"count\":${model.count.to_str()}}"
