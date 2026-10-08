app [program] { pf: platform "../roc-world/platform.roc" }

import pf.Screen

Model := { count : I64 }

program = { init, update, view, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { count: 0 }

update : Model, U64, Str -> Model
update = |model, _pid, ev| if ev == "inc" { count: model.count + 1 } else model

view : Model -> Str
view = |model| {
    base = Screen.boxed(Screen.new(20, 3), 0, 0, 20, 3, "counter")
    Screen.render_plain(Screen.put_text(base, 2, 1, "count=${model.count.to_str()}", { fg: 0, bg: 0, bold: Bool.False }))
}

records : Model -> Str
records = |model| "{\"count\":${model.count.to_str()}}"

wants : Model -> Str
wants = |_model| "{}"

snapshot : Model -> Str
snapshot = |model| model.count.to_str()

restore : Str -> Model
restore = |text| match I64.from_str(text) {
    Ok(n) => { count: n }
    Err(_) => { count: 0 }
}
