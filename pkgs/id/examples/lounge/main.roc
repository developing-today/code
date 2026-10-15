app [program] { pf: platform "../roc-world/platform.roc" }

import pf.JsonLib

# A lounge world: the first example that uses the server's capability
# system. It subscribes to arrivals and ticks, greets every newcomer through
# `chat.say`, and polls the clock through `time.now`.
#
# Host events are decoded into JSON values and requests/records are encoded
# from values with the pure-Roc library provided by the platform.

Model : {
	joins : U64,
	denied : U64,
	next : U64,
	last_now : Str,
	pending : List(JsonLib),
}

program = { init, update, view, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { joins: 0, denied: 0, next: 0, last_now: "", pending: [] }

update : Model, U64, Str -> Model
update = |model, _pid, ev| {
	match JsonLib.decode(ev) {
		Err(_) => model
		Ok(event) => {
			cap = string_field(event, "cap")
			event_name = string_field(event, "event")
			if cap == "players" and event_name == "joined" {
				name = sanitize(string_field(event, "name"))
				next = model.next + 1
				request = chat_request(next, Str.concat("Welcome, ", Str.concat(name, "!")))
				{ joins: model.joins + 1, denied: model.denied, next: next, last_now: model.last_now, pending: List.append(model.pending, request) }
			} else if cap == "time.tick" {
				next = model.next + 1
				request = time_request(next)
				{ joins: model.joins, denied: model.denied, next: next, last_now: model.last_now, pending: List.append(model.pending, request) }
			} else if cap == "result" {
				id = string_field(event, "id")
				request_cap = string_field(pending_request(model.pending, id), "cap")
				ok = bool_field(event, "ok")
				denied = if request_cap == "chat.say" and ok == Bool.False {
					model.denied + 1
				} else {
					model.denied
				}
				last_now = if request_cap == "time.now" and ok == Bool.True {
					number_field(field(event, "value"), "now")
				} else {
					model.last_now
				}
				{ joins: model.joins, denied: denied, next: model.next, last_now: last_now, pending: drop_pending(model.pending, id) }
			} else {
				model
			}
		}
	}
}

view : Model -> Str
view = |model|
	"lounge joins=${model.joins.to_str()} denied=${model.denied.to_str()} last=${model.last_now}"

records : Model -> Str
records = |model| encode_or_crash(
	JsonLib.Object([
		{ key: "joins", value: JsonLib.Number(model.joins.to_str()) },
		{ key: "denied", value: JsonLib.Number(model.denied.to_str()) },
		{ key: "last_now", value: JsonLib.Number(num_of(model.last_now).to_str()) },
	]),
)

## The capability protocol: subscriptions and currently outstanding requests.
wants : Model -> Str
wants = |model| encode_or_crash(
	JsonLib.Object([
		{ key: "v", value: JsonLib.Number("1") },
		{ key: "subscribe", value: JsonLib.Array([JsonLib.String("players"), JsonLib.String("time.tick:5000")]) },
		{ key: "requests", value: JsonLib.Array(model.pending) },
	]),
)

## Keep the existing checkpoint format stable; each pending line is compact JSON.
snapshot : Model -> Str
snapshot = |model|
	"${model.joins.to_str()} ${model.denied.to_str()} ${model.next.to_str()} ${model.last_now}\n${Str.join_with(List.map(model.pending, encode_or_crash), "\n")}"

restore : Str -> Model
restore = |text| {
	parts = Str.split_on(text, "\n")
	head = match List.first(parts) {
		Ok(line) => line
		Err(_) => ""
	}
	fields = Str.split_on(head, " ")
	pending = List.fold(
		List.drop_first(parts, 1),
		[],
		|acc, line| match JsonLib.decode(line) {
			Ok(value) => List.append(acc, value)
			Err(_) => acc
		},
	)
	{
		joins: num(fields, 0),
		denied: num(fields, 1),
		next: num(fields, 2),
		last_now: part(fields, 3),
		pending: pending,
	}
}

# --- JSON/event helpers ----------------------------------------------------

## JSON generation is part of the app's output contract. Never replace an
## encoding failure with empty or malformed JSON.
encode_or_crash = |value| match JsonLib.encode(value) {
	Ok(text) => text
	Err(_) => crash "lounge attempted to encode an invalid or oversized JSON value"
}

field = |value, key| match JsonLib.get(value, key) {
	Ok(found) => found
	Err(_) => JsonLib.Null
}

string_field = |value, key| match JsonLib.as_string(field(value, key)) {
	Ok(text) => text
	Err(_) => ""
}

number_field = |value, key| match JsonLib.as_number(field(value, key)) {
	Ok(text) => text
	Err(_) => ""
}

bool_field = |value, key| match JsonLib.as_bool(field(value, key)) {
	Ok(boolean) => boolean
	Err(_) => Bool.False
}

sanitize = |text| {
	kept = List.keep_if(
		Str.to_utf8(text),
		|byte| {
			(byte >= 48 and byte <= 57)
				or (byte >= 65 and byte <= 90)
					or (byte >= 97 and byte <= 122)
						or byte == 45
							or byte == 95
								or byte == 32
		},
	)
	match Str.from_utf8(kept) {
		Ok(clean) => clean
		Err(_) => ""
	}
}

chat_request = |next, text| JsonLib.Object([
	{ key: "id", value: JsonLib.String(Str.concat("r", next.to_str())) },
	{ key: "cap", value: JsonLib.String("chat.say") },
	{ key: "args", value: JsonLib.Object([{ key: "text", value: JsonLib.String(text) }]) },
])

time_request = |next| JsonLib.Object([
	{ key: "id", value: JsonLib.String(Str.concat("r", next.to_str())) },
	{ key: "cap", value: JsonLib.String("time.now") },
])

pending_request = |pending, id|
	match List.first(List.keep_if(pending, |request| string_field(request, "id") == id)) {
		Ok(request) => request
		Err(_) => JsonLib.Null
	}

drop_pending = |pending, id|
	List.keep_if(pending, |request| string_field(request, "id") != id)

# --- snapshot scalar helpers ----------------------------------------------

part = |fields, index|
	match List.get(fields, index) {
		Ok(found) => found
		Err(_) => ""
	}

num = |fields, index| num_of(part(fields, index))

num_of = |text|
	match U64.from_str(text) {
		Ok(value) => value
		Err(_) => 0
	}
