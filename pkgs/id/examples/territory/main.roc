app [program] { pf: platform "../roc-world/platform.roc" }

import pf.Key

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
    cells : List(U8),
    seats : List(Seat),
}

program = { init, update, view, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { cells: List.repeat(empty, width * height), seats: [] }

update : Model, U64, Str -> Model
update = |model, pid, ev|
    if pid == 0 {
        presence(model, ev)
    } else {
        List.fold(Key.parse(ev), model, |m, key| act(m, pid, key))
    }

view : Model -> Str
view = |model| {
    free = List.len(List.keep_if(model.cells, |cell| cell == empty))
    rows = List.map(row_indices, |y| "|${row_text(model, y)}|")
    scores = List.map(model.seats, |seat| seat_line(model, seat))
    lines = List.concat(List.concat(List.concat([border], rows), [border, "free=${free.to_str()}  move: arrows/hjkl  claim: enter/space"]), scores)
    Str.join_with(lines, "\n")
}

## Everything here is derived from the grid and the seats, so the record
## is a pure function of the state a snapshot already carries.
records : Model -> Str
records = |model| {
    seats = Str.join_with(List.map(model.seats, |seat| seat_json(model, seat)), ",")
    "{\"width\":${width.to_str()},\"height\":${height.to_str()},\"cells\":\"${bytes_to_str(model.cells)}\",\"seats\":[${seats}]}"
}

## Presence is subscribed so joins name their seats; nothing else is requested.
wants : Model -> Str
wants = |_model| "{\"v\":1,\"subscribe\":[\"players\"],\"requests\":[]}"

snapshot : Model -> Str
snapshot = |model|
    Str.join_with(List.concat([bytes_to_str(model.cells)], List.map(model.seats, |seat| seat.name)), "\n")

restore : Str -> Model
restore = |text| {
    grid = Str.to_utf8(until(text, "\n"))
    names = List.drop_first(Str.split_on(text, "\n"), 1)
    cells = if List.len(grid) == width * height grid else List.repeat(empty, width * height)
    seats = List.fold(names, [], |acc, name|
        if List.len(acc) < max_seats {
            List.append(acc, { mark: mark_for(List.len(acc)), pid: 0, name: name, x: 0, y: 0, online: Bool.False })
        } else {
            acc
        })
    { cells: cells, seats: seats }
}

# --- presence --------------------------------------------------------------

presence : Model, Str -> Model
presence = |model, ev| {
    pid = parse_pid(ev)
    if Str.contains(ev, "\"event\":\"joined\"") {
        join(model, pid, sanitize(until(after(ev, "\"name\":\""), "\"")))
    } else if Str.contains(ev, "\"event\":\"left\"") {
        leave(model, pid)
    } else {
        model
    }
}

parse_pid : Str -> U64
parse_pid = |ev|
    match U64.from_str(until(until(after(ev, "\"id\":"), ","), "}")) {
        Ok(pid) => pid
        Err(_) => 0
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
        { cells: model.cells, seats: List.append(model.seats, seat) }
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
        { cells: model.cells, seats: set_seat_list(model.seats, { ..seat, x: x, y: y }) }
    } else {
        model
    }

claim : Model, Seat -> Model
claim = |model, seat|
    if owner_at(model.cells, seat.x, seat.y) == empty and (owned_by(model.cells, seat.mark) == 0 or touches(model.cells, seat)) {
        match List.set(model.cells, cell_index(seat.x, seat.y), seat.mark) {
            Ok(cells) => { cells: cells, seats: model.seats }
            Err(_) => model
        }
    } else {
        model
    }

touches : List(U8), Seat -> Bool
touches = |cells, seat|
    (seat.x > 0 and owner_at(cells, seat.x - 1, seat.y) == seat.mark) or owner_at(cells, seat.x + 1, seat.y) == seat.mark or (seat.y > 0 and owner_at(cells, seat.x, seat.y - 1) == seat.mark) or owner_at(cells, seat.x, seat.y + 1) == seat.mark

# --- grid and seats --------------------------------------------------------

cell_index : U64, U64 -> U64
cell_index = |x, y| y * width + x

owner_at : List(U8), U64, U64 -> U8
owner_at = |cells, x, y|
    if x < width and y < height {
        match List.get(cells, cell_index(x, y)) {
            Ok(owner) => owner
            Err(_) => empty
        }
    } else {
        empty
    }

owned_by : List(U8), U8 -> U64
owned_by = |cells, mark| List.len(List.keep_if(cells, |cell| cell == mark))

## The lowercase mark of the first online seat whose cursor is on the cell, or 0.
cursor_at : List(Seat), U64, U64 -> U8
cursor_at = |seats, x, y|
    List.fold(seats, 0, |found, seat|
        if found == 0 and seat.online and seat.x == x and seat.y == y { seat.mark + 32 } else { found })

cell_byte : Model, U64, U64 -> U8
cell_byte = |model, x, y| {
    cursor = cursor_at(model.seats, x, y)
    if cursor == 0 { owner_at(model.cells, x, y) } else { cursor }
}

row_text : Model, U64 -> Str
row_text = |model, y|
    bytes_to_str(List.map(column_indices, |x| cell_byte(model, x, y)))

seat_line : Model, Seat -> Str
seat_line = |model, seat| {
    status = if seat.online "online" else "away"
    "${bytes_to_str([seat.mark])} ${seat.name} ${owned_by(model.cells, seat.mark).to_str()} ${status}"
}

seat_json : Model, Seat -> Str
seat_json = |model, seat| {
    online = if seat.online "true" else "false"
    "{\"mark\":\"${bytes_to_str([seat.mark])}\",\"name\":\"${seat.name}\",\"cells\":${owned_by(model.cells, seat.mark).to_str()},\"online\":${online},\"x\":${seat.x.to_str()},\"y\":${seat.y.to_str()}}"
}

find_seat = |seats, pred| List.first(List.keep_if(seats, pred))

row_indices : List(U64)
row_indices = [0, 1, 2, 3, 4, 5, 6, 7]

column_indices : List(U64)
column_indices = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]

set_seat : Model, Seat -> Model
set_seat = |model, seat| { cells: model.cells, seats: set_seat_list(model.seats, seat) }

set_seat_list : List(Seat), Seat -> List(Seat)
set_seat_list = |seats, seat| List.map(seats, |other| if other.mark == seat.mark { seat } else { other })

mark_for : U64 -> U8
mark_for = |n|
    match List.get(Str.to_utf8("ABCDEFGH"), n) {
        Ok(mark) => mark
        Err(_) => 63
    }

# --- text helpers ----------------------------------------------------------

bytes_to_str : List(U8) -> Str
bytes_to_str = |bytes|
    match Str.from_utf8(bytes) {
        Ok(text) => text
        Err(_) => ""
    }

after : Str, Str -> Str
after = |text, marker|
    match List.get(Str.split_on(text, marker), 1) {
        Ok(rest) => rest
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
