app [program] { pf: platform "../roc-world/platform.roc" }

import pf.Screen
import pf.RocText

Model := { count : I64 }

program = { init, update, view, layout, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { count: 0 }

## Events are matched after a RocText trim, so a journaled or typed
## " inc " counts the same as "inc".
update : Model, U64, Str -> Model
update = |model, _pid, ev| if RocText.trim(ev) == "inc" { count: model.count + 1 } else model

## The counter renders through Screen, like roc-screen: a boxed 20x3
## screen whose JSON the host draws, instead of a bare text view.
view : Model -> Str
view = |model| render(model, 20, 3)

# Reflow: a wide viewport gets a wider box, a short one keeps three rows.
layout : Model, U64, U64 -> Str
layout = |model, width, height| {
    w = if width >= 30 30 else 20
    h = if height >= 4 3 else 3
    render(model, w, h)
}

render : Model, U64, U64 -> Str
render = |model, w, h| {
    base = Screen.boxed(Screen.new(w, h), 0, 0, w, h, "counter")
    Screen.render_json(Screen.put_text(base, 2, 1, "count=${model.count.to_str()}", { fg: 0, bg: 0, bold: Bool.False }))
}

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
