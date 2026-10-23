app [program] { pf: platform "../roc-world/platform.roc" }

import pf.Key
import pf.Screen
import pf.RocJson
import pf.RocGrid

# A territory world for up to eight players on a 16x8 grid.
#
# A player who acts gets a seat: a mark (A..H) and a cursor. Arrows or hjkl
# move the cursor, enter or space claims the cell under it. The first claim
# may go anywhere empty; after that a player may only claim empty cells next
# to their own territory.
#
# Presence events arrive with pid 0 as JSON. A seat keeps its cells when its
# player leaves and is taken back by a player who rejoins under the same name,
# so the grid and scoreboard survive a restart. `snapshot` writes the grid and
# one seat name per line; `restore` brings the seats back offline.

width : U64
width = 16

height : U64
height = 8

max_seats : U64
max_seats = 8

empty : U8
empty = 46 # '.'

border = "+----------------+"

Seat : {
    mark : U8,
    pid : U64,
    name : Str,
    x : U64,
    y : U64,
    online : Bool,
}

Model : {
    grid : RocGrid,
    seats : List(Seat),
}

program = { init, update, view, layout, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { grid: RocGrid.new(width, height, empty), seats: [] }

update : Model, U64, Str -> Model
update = |model, pid, ev|
    if pid == 0 {
        presence(model, ev)
    } else {
        List.fold(Key.parse(ev), model, |m, key| act(m, pid, key))
    }

view : Model -> Str
view = |model| {
    plain = { fg: 0, bg: 0, bold: Bool.False }
    edge = [{ text: Screen.box_edge(18, "territory"), style: plain }]
    bottom = [{ text: Screen.box_edge(18, ""), style: plain }]
    grid_rows = List.map(row_indices, |y| grid_row(model, y))
    free = RocGrid.count(model.grid, empty)
    status = [{ text: "free=${free.to_str()} [hjkl/space]", style: plain }]
    scores = List.map(model.seats, |seat| [{ text: seat_line(model, seat), style: mark_style(seat.mark) }])
    rows = List.concat(List.concat([edge], grid_rows), List.concat([bottom, status], scores))
    Screen.render_rows_json(24, 19, rows)
}

## One boxed grid row as styled pieces: the borders, then the cells with equal
## styles merged into runs, so a row costs a handful of pieces and not 16 cells.
grid_row : Model, U64 -> List({ text : Str, style : { fg : U8, bg : U8, bold : Bool } })
grid_row = |model, y| {
    plain = { fg: 0, bg: 0, bold: Bool.False }
    bytes = List.map(column_indices, |x| cell_byte(model, x, y))
    List.concat([{ text: "|", style: plain }], List.append(style_runs(bytes), { text: "|", style: plain }))
}

## Group consecutive bytes that share a style into one piece of text.
style_runs : List(U8) -> List({ text : Str, style : { fg : U8, bg : U8, bold : Bool } })
style_runs = |bytes| {
    start = { runs: [], run: [], style: { fg: 0, bg: 0, bold: Bool.False } }
    done = List.fold(bytes, start, |acc, b| {
        style = style_for_cell(b)
        if List.is_empty(acc.run) or same_style(style, acc.style) {
            { runs: acc.runs, run: List.append(acc.run, b), style: style }
        } else {
            { runs: List.append(acc.runs, { text: bytes_to_str(acc.run), style: acc.style }), run: [b], style: style }
        }
    })
    if List.is_empty(done.run) { done.runs } else { List.append(done.runs, { text: bytes_to_str(done.run), style: done.style }) }
}

same_style : { fg : U8, bg : U8, bold : Bool }, { fg : U8, bg : U8, bold : Bool } -> Bool
same_style = |a, b| a.fg == b.fg and a.bg == b.bg and a.bold == b.bold

## Everything here is derived from the grid and the seats, so the record
## is a pure function of the state a snapshot already carries.
## This program renders the same layout at any viewport.
layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

records : Model -> Str
records = |model| {
    seats = Str.join_with(List.map(model.seats, |seat| seat_json(model, seat)), ",")
    "{\"width\":${width.to_str()},\"height\":${height.to_str()},\"cells\":\"${bytes_to_str(RocGrid.bytes(model.grid))}\",\"seats\":[${seats}]}"
}

## Presence is subscribed so joins name their seats; nothing else is requested.
wants : Model -> Str
wants = |_model| "{\"v\":1,\"subscribe\":[\"players\"],\"requests\":[]}"

snapshot : Model -> Str
snapshot = |model|
    Str.join_with(List.concat([bytes_to_str(RocGrid.bytes(model.grid))], List.map(model.seats, |seat| seat.name)), "\n")

restore : Str -> Model
restore = |text| {
    grid = Str.to_utf8(until(text, "\n"))
    names = List.drop_first(Str.split_on(text, "\n"), 1)
    board = RocGrid.from_cells(width, height, grid, empty)
    seats = List.fold(names, [], |acc, name|
        if List.len(acc) < max_seats {
            List.append(acc, { mark: mark_for(List.len(acc)), pid: 0, name: name, x: 0, y: 0, online: Bool.False })
        } else {
            acc
        })
    { grid: board, seats: seats }
}

# --- presence --------------------------------------------------------------

presence : Model, Str -> Model
presence = |model, ev|
    match RocJson.decode(ev) {
        Ok(event) => {
            event_name = string_field(event, "event")
            part = field(event, "participant")
            pid_str = number_field(part, "id")
            name = sanitize(string_field(part, "name"))
            pid = match U64.from_str(pid_str) {
                Ok(n) => n
                Err(_) => 0
            }
            if event_name == "joined" {
                join(model, pid, name)
            } else if event_name == "left" {
                leave(model, pid)
            } else {
                model
            }
        }
        Err(_) => model
    }

join : Model, U64, Str -> Model
join = |model, pid, name|
    if pid == 0 or name == "" {
        model
    } else {
        match find_seat(model.seats, |seat| seat.pid == pid) {
            Ok(seat) => set_seat(model, { ..seat, name: name, online: Bool.True })
            Err(_) => match find_seat(model.seats, |seat| !seat.online and seat.name == name) {
                Ok(seat) => set_seat(model, { ..seat, pid: pid, online: Bool.True })
                Err(_) => add_seat(model, pid, name)
            }
        }
    }

leave : Model, U64 -> Model
leave = |model, pid|
    if pid == 0 {
        model
    } else {
        match find_seat(model.seats, |seat| seat.pid == pid) {
            Ok(seat) => set_seat(model, { ..seat, pid: 0, online: Bool.False, x: 0, y: 0 })
            Err(_) => model
        }
    }

add_seat : Model, U64, Str -> Model
add_seat = |model, pid, name|
    if List.len(model.seats) < max_seats {
        seat = { mark: mark_for(List.len(model.seats)), pid: pid, name: name, x: 0, y: 0, online: Bool.True }
        { grid: model.grid, seats: List.append(model.seats, seat) }
    } else {
        model
    }

# --- play ------------------------------------------------------------------

act : Model, U64, Key -> Model
act = |model, pid, key| {
    seated = match find_seat(model.seats, |seat| seat.pid == pid) {
        Ok(_) => model
        Err(_) => add_seat(model, pid, "p${pid.to_str()}")
    }
    match find_seat(seated.seats, |seat| seat.pid == pid) {
        Ok(seat) => step(seated, seat, key)
        Err(_) => seated
    }
}

step : Model, Seat, Key -> Model
step = |model, seat, key|
    match key {
        Left | Char("h") => if seat.x > 0 { go(model, seat, seat.x - 1, seat.y) } else { model }
        Right | Char("l") => go(model, seat, seat.x + 1, seat.y)
        Up | Char("k") => if seat.y > 0 { go(model, seat, seat.x, seat.y - 1) } else { model }
        Down | Char("j") => go(model, seat, seat.x, seat.y + 1)
        Enter | Char(" ") => claim(model, seat)
        _ => model
    }

go : Model, Seat, U64, U64 -> Model
go = |model, seat, x, y|
    if x < width and y < height {
        { grid: model.grid, seats: set_seat_list(model.seats, { ..seat, x: x, y: y }) }
    } else {
        model
    }

claim : Model, Seat -> Model
claim = |model, seat|
    if RocGrid.get(model.grid, seat.x, seat.y, empty) == empty and (RocGrid.count(model.grid, seat.mark) == 0 or touches(model.grid, seat)) {
        { grid: RocGrid.set(model.grid, seat.x, seat.y, seat.mark), seats: model.seats }
    } else {
        model
    }

touches : RocGrid, Seat -> Bool
touches = |grid, seat|
    List.fold(RocGrid.neighbors4(grid, seat.x, seat.y), Bool.False, |found, cell|
        found or RocGrid.get(grid, cell.x, cell.y, empty) == seat.mark)

# --- grid and seats --------------------------------------------------------

## The lowercase mark of the first online seat whose cursor is on the cell, or 0.
cursor_at : List(Seat), U64, U64 -> U8
cursor_at = |seats, x, y|
    List.fold(seats, 0, |found, seat|
        if found == 0 and seat.online and seat.x == x and seat.y == y { seat.mark + 32 } else { found })

cell_byte : Model, U64, U64 -> U8
cell_byte = |model, x, y| {
    cursor = cursor_at(model.seats, x, y)
    if cursor == 0 { RocGrid.get(model.grid, x, y, empty) } else { cursor }
}

row_text : Model, U64 -> Str
row_text = |model, y|
    bytes_to_str(List.map(column_indices, |x| cell_byte(model, x, y)))

seat_line : Model, Seat -> Str
seat_line = |model, seat| {
    status = if seat.online "online" else "away"
    "${bytes_to_str([seat.mark])} ${seat.name} ${RocGrid.count(model.grid, seat.mark).to_str()} ${status}"
}

seat_json : Model, Seat -> Str
seat_json = |model, seat| {
    online = if seat.online "true" else "false"
    "{\"mark\":\"${bytes_to_str([seat.mark])}\",\"name\":\"${seat.name}\",\"cells\":${RocGrid.count(model.grid, seat.mark).to_str()},\"online\":${online},\"x\":${seat.x.to_str()},\"y\":${seat.y.to_str()}}"
}

find_seat = |seats, pred| List.first(List.keep_if(seats, pred))

row_indices : List(U64)
row_indices = [0, 1, 2, 3, 4, 5, 6, 7]

column_indices : List(U64)
column_indices = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]

set_seat : Model, Seat -> Model
set_seat = |model, seat| { grid: model.grid, seats: set_seat_list(model.seats, seat) }

set_seat_list : List(Seat), Seat -> List(Seat)
set_seat_list = |seats, seat| List.map(seats, |other| if other.mark == seat.mark { seat } else { other })

mark_for : U64 -> U8
mark_for = |n|
    match List.get(Str.to_utf8("ABCDEFGH"), n) {
        Ok(mark) => mark
        Err(_) => 63
    }

# --- styles and helpers ----------------------------------------------------

mark_style : U8 -> { fg : U8, bg : U8, bold : Bool }
mark_style = |mark|
    match mark {
        65 => { fg: 10, bg: 0, bold: Bool.True }
        66 => { fg: 12, bg: 0, bold: Bool.True }
        67 => { fg: 11, bg: 0, bold: Bool.True }
        68 => { fg: 13, bg: 0, bold: Bool.True }
        69 => { fg: 14, bg: 0, bold: Bool.True }
        70 => { fg: 9,  bg: 0, bold: Bool.True }
        71 => { fg: 15, bg: 0, bold: Bool.True }
        72 => { fg: 8,  bg: 0, bold: Bool.True }
        _  => { fg: 0,  bg: 0, bold: Bool.False }
    }

cursor_style : U8 -> { fg : U8, bg : U8, bold : Bool }
cursor_style = |cursor_mark|
    match cursor_mark {
        97  => { fg: 0, bg: 10, bold: Bool.True }
        98  => { fg: 0, bg: 12, bold: Bool.True }
        99  => { fg: 0, bg: 11, bold: Bool.True }
        100 => { fg: 0, bg: 13, bold: Bool.True }
        101 => { fg: 0, bg: 14, bold: Bool.True }
        102 => { fg: 0, bg: 9,  bold: Bool.True }
        103 => { fg: 0, bg: 15, bold: Bool.True }
        104 => { fg: 15, bg: 8, bold: Bool.True }
        _   => { fg: 0, bg: 7,  bold: Bool.True }
    }

style_for_cell : U8 -> { fg : U8, bg : U8, bold : Bool }
style_for_cell = |b|
    if b >= 97 and b <= 104 {
        cursor_style(b)
    } else if b >= 65 and b <= 72 {
        mark_style(b)
    } else {
        { fg: 8, bg: 0, bold: Bool.False }
    }

field = |value, key| match RocJson.get(value, key) {
    Ok(found) => found
    Err(_) => RocJson.Null
}

string_field = |value, key| match RocJson.as_string(field(value, key)) {
    Ok(text) => text
    Err(_) => ""
}

number_field = |value, key| match RocJson.as_number(field(value, key)) {
    Ok(text) => text
    Err(_) => ""
}

# --- text helpers ----------------------------------------------------------

bytes_to_str : List(U8) -> Str
bytes_to_str = |bytes|
    match Str.from_utf8(bytes) {
        Ok(text) => text
        Err(_) => ""
    }

until : Str, Str -> Str
until = |text, marker|
    match List.get(Str.split_on(text, marker), 0) {
        Ok(head) => head
        Err(_) => ""
    }

## Names are shown in the scoreboard and written into JSON, so only plain
## name bytes survive.
sanitize : Str -> Str
sanitize = |name|
    bytes_to_str(List.take_first(List.keep_if(Str.to_utf8(name), is_name_byte), 16))

is_name_byte : U8 -> Bool
is_name_byte = |b|
    (b >= 48 and b <= 57) or (b >= 65 and b <= 90) or (b >= 97 and b <= 122) or b == 45 or b == 95
