app [program] { pf: platform "../roc-world/platform.roc" }

# A lounge world: the first example that uses the server's capability
# system. The program subscribes to arrivals and ticks, greets every
# newcomer through `chat.say`, and polls the clock through `time.now`.
#
# Everything the server sends arrives as a host event (participant 0) whose
# `update` argument is JSON text; parsing it here is deliberately plain
# string work, because Roc gets no JSON decoder from this platform. Each
# pending request is stored as the complete JSON object, so `wants` only
# has to join them and results are matched by their id.

Model : {
    joins : U64,
    denied : U64,
    next : U64,
    last_now : Str,
    pending : List(Str),
}

program = { init, update, view, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { joins: 0, denied: 0, next: 0, last_now: "", pending: [] }

update : Model, U64, Str -> Model
update = |model, _pid, ev| {
    if Str.contains(ev, "\"cap\":\"players\"") and Str.contains(ev, "\"event\":\"joined\"") {
        name = sanitize(between(ev, "\"name\":\"", "\"}"))
        next = model.next + 1
        request = "{\"id\":\"r${next.to_str()}\",\"cap\":\"chat.say\",\"args\":{\"text\":\"Welcome, ${name}!\"}}"
        { joins: model.joins + 1, denied: model.denied, next: next, last_now: model.last_now, pending: List.append(model.pending, request) }
    } else if Str.contains(ev, "\"cap\":\"time.tick\"") {
        next = model.next + 1
        request = "{\"id\":\"r${next.to_str()}\",\"cap\":\"time.now\"}"
        { joins: model.joins, denied: model.denied, next: next, last_now: model.last_now, pending: List.append(model.pending, request) }
    } else if Str.contains(ev, "\"cap\":\"result\"") {
        id = between(ev, "\"id\":\"", "\",\"ok\"")
        line = pending_line(model.pending, id)
        denied = if Str.contains(line, "chat.say") and Str.contains(ev, "\"ok\":false") {
            model.denied + 1
        } else {
            model.denied
        }
        last_now = if Str.contains(line, "time.now") and Str.contains(ev, "\"ok\":true") {
            between(ev, "\"now\":", "}")
        } else {
            model.last_now
        }
        { joins: model.joins, denied: denied, next: model.next, last_now: last_now, pending: drop_pending(model.pending, id) }
    } else {
        model
    }
}

view : Model, Str -> Str
view = |model, _viewer|
    "lounge joins=${model.joins.to_str()} denied=${model.denied.to_str()} last=${model.last_now}"

records : Model -> Str
records = |model| {
    now = num_of(model.last_now)
    "{\"joins\":${model.joins.to_str()},\"denied\":${model.denied.to_str()},\"last_now\":${now.to_str()}}"
}

## What the lounge wants right now: the two subscriptions, plus every
## pending request, verbatim.
wants : Model -> Str
wants = |model|
    "{\"v\":1,\"subscribe\":[\"players\",\"time.tick:5000\"],\"requests\":[${Str.join_with(model.pending, ",")}]}"

snapshot : Model -> Str
snapshot = |model|
    "${model.joins.to_str()} ${model.denied.to_str()} ${model.next.to_str()} ${model.last_now}\n${Str.join_with(model.pending, "\n")}"

restore : Str -> Model
restore = |text| {
    parts = Str.split_on(text, "\n")
    head = match List.first(parts) {
        Ok(line) => line
        Err(_) => ""
    }
    fields = Str.split_on(head, " ")
    {
        joins: num(fields, 0),
        denied: num(fields, 1),
        next: num(fields, 2),
        last_now: part(fields, 3),
        pending: List.drop_first(parts, 1),
    }
}

# --- helpers ---------------------------------------------------------------

## Requests retire when their result arrives; `id` includes the "r" prefix.
drop_pending = |pending, id|
    List.keep_if(pending, |line| !Str.contains(line, "\"id\":\"${id}\""))

pending_line = |pending, id|
    match List.first(List.keep_if(pending, |line| Str.contains(line, "\"id\":\"${id}\""))) {
        Ok(line) => line
        Err(_) => ""
    }

## Keep only characters that are safe inside a JSON string.
sanitize = |text| {
    kept = List.keep_if(Str.to_utf8(text), |byte| {
        (byte >= 48 and byte <= 57)
            or (byte >= 65 and byte <= 90)
            or (byte >= 97 and byte <= 122)
            or byte == 45
            or byte == 95
            or byte == 32
    })
    match Str.from_utf8(kept) {
        Ok(clean) => clean
        Err(_) => ""
    }
}

## The text between the first `open` and the next `close`.
between = |text, open, close|
    match List.first(Str.split_on(after(text, open), close)) {
        Ok(found) => found
        Err(_) => ""
    }

after = |text, open|
    match List.get(Str.split_on(text, open), 1) {
        Ok(rest) => rest
        Err(_) => ""
    }

part = |fields, index|
    match List.get(fields, index) {
        Ok(field) => field
        Err(_) => ""
    }

num = |fields, index| num_of(part(fields, index))

num_of = |text|
    match U64.from_str(text) {
        Ok(value) => value
        Err(_) => 0
    }
