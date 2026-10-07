app [program] { pf: platform "../platform/main.roc" }

Model := { count : I64 }

program = { init, update, view }

init : U64 -> Model
init = |_seed| { count: 0 }

update : Model, Str -> Model
update = |model, ev| if ev == "inc" { count: model.count + 1 } else model

view : Model, Str -> Str
view = |model, _viewer| "count=${model.count.to_str()}"
