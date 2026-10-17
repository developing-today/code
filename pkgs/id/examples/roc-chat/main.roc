app [program] { pf: platform "../roc-world/platform.roc" }

import pf.Key
import pf.RocCap
import pf.RocJson

# A chat client built on `pf.RocCap` and `chat.history`: type a line and press
# enter to say it; type `hist` and press enter to ask for the newest lines the
# world has seen. The view reports how many lines came back and the newest one.
#
# Chat lines arrive from the server as `{"cap":"result","id":...}` events and
# are routed by `RocCap.answered`, so the model stays correct when the journal
# replays the session.

Model : {
	cap : RocCap,
	seen : U64,
	last : Str,
	line : Str,
}

program = { init, update, view, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { cap: RocCap.empty, seen: 0, last: "", line: "" }

update : Model, U64, Str -> Model
update = |model, pid, ev|
	if pid == 0 {
		host(model, ev)
	} else {
		List.fold(Key.parse(ev), model, |m, key| match key {
			Char(c) => { cap: m.cap, seen: m.seen, last: m.last, line: Str.concat(m.line, c) }
			Enter => send(m, m.line)
			_ => m
		})
	}

## A typed line: `hist` asks for history, anything else is said.
send : Model, Str -> Model
send = |model, text|
	if text == "" {
		{ ..model, line: "" }
	} else if text == "hist" {
		issued = RocCap.issue(model.cap, "chat.history")
		request = RocCap.history_request(issued.id, 8)
		{ cap: RocCap.attach(issued.session, issued.id, request), seen: model.seen, last: model.last, line: "" }
	} else {
		issued = RocCap.issue(model.cap, "chat.say")
		request = RocCap.chat_request(issued.id, text)
		{ cap: RocCap.attach(issued.session, issued.id, request), seen: model.seen, last: model.last, line: "" }
	}

## Host events (participant 0): results are routed by id.
host : Model, Str -> Model
host = |model, ev| match RocJson.decode(ev) {
	Err(_) => model
	Ok(event) => {
		if string_field(event, "cap") != "result" {
			model
		} else {
			id = string_field(event, "id")
			answered = RocCap.answered(model.cap, id)
			ok = match RocJson.as_bool(field(event, "ok")) {
				Ok(boolean) => boolean
				Err(_) => Bool.False
			}
			if answered == "chat.history" and ok {
				lines = RocCap.history_lines(field(event, "value"))
				last = List.fold(lines, model.last, |_, line| string_field(line, "text"))
				{ cap: RocCap.done(model.cap, id), seen: List.len(lines), last: last, line: model.line }
			} else if answered != "" {
				{ cap: RocCap.done(model.cap, id), seen: model.seen, last: model.last, line: model.line }
			} else {
				model
			}
		}
	}
}

view : Model -> Str
view = |model|
	"chat seen=${model.seen.to_str()} last=${model.last} input=${model.line}"

records : Model -> Str
records = |model| encode_or_crash(
	RocJson.Object([{ key: "seen", value: RocJson.Number(model.seen.to_str()) }]),
)

## No subscriptions; the outstanding requests are re-announced on every replay.
wants : Model -> Str
wants = |model| encode_or_crash(
	RocJson.Object([
		{ key: "v", value: RocJson.Number("1") },
		{ key: "requests", value: RocJson.Array(RocCap.requests(model.cap)) },
	]),
)

snapshot : Model -> Str
snapshot = |model| {
	documents = Str.join_with(List.map(RocCap.requests(model.cap), encode_or_crash), "\n")
	"${model.seen.to_str()}\n${RocCap.counter(model.cap).to_str()}\n${model.last}\n${documents}"
}

restore : Str -> Model
restore = |text| {
	parts = Str.split_on(text, "\n")
	documents = List.fold(List.drop_first(parts, 3), [], |acc, line| match RocJson.decode(line) {
		Ok(value) => List.append(acc, value)
		Err(_) => acc
	})
	{
		cap: RocCap.from_pending(num(parts, 1), documents),
		seen: num(parts, 0),
		last: part(parts, 2),
		line: "",
	}
}

# --- helpers ---------------------------------------------------------------

encode_or_crash = |value| match RocJson.encode(value) {
	Ok(text) => text
	Err(_) => crash "roc-chat attempted to encode an invalid or oversized JSON value"
}

field = |value, key| match RocJson.get(value, key) {
	Ok(found) => found
	Err(_) => RocJson.Null
}

string_field = |value, key| match RocJson.as_string(field(value, key)) {
	Ok(text) => text
	Err(_) => ""
}

part = |fields, index| match List.get(fields, index) {
	Ok(found) => found
	Err(_) => ""
}

num = |fields, index| match U64.from_str(part(fields, index)) {
	Ok(value) => value
	Err(_) => 0
}
