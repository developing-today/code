app [program] { pf: platform "../roc-world/platform.roc" }

import pf.Screen
import pf.Key

Model := { count : I64 }

program = { init, update, view, layout, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { count: 0 }

update : Model, U64, Str -> Model
update = |model, _pid, ev| {
    presses = List.fold(Key.parse(ev), 0, |n, key| if key == Char("+") { n + 1 } else { n })
    { count: model.count + presses }
}

view : Model -> Str
view = |model| {
    base = Screen.boxed(Screen.new(20, 3), 0, 0, 20, 3, "counter")
    Screen.render_json(Screen.put_text(base, 2, 1, "count=${model.count.to_str()}", { fg: 0, bg: 0, bold: Bool.False }))
}

# Reflow: a wide viewport gets a wider box, a short one keeps three rows.
layout : Model, U64, U64 -> Str
layout = |model, width, height| {
    w = if width >= 30 30 else 20
    h = if height >= 4 3 else 3
    base = Screen.boxed(Screen.new(w, h), 0, 0, w, h, "counter")
    Screen.render_json(Screen.put_text(base, 2, 1, "count=${model.count.to_str()}", { fg: 0, bg: 0, bold: Bool.False }))
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
