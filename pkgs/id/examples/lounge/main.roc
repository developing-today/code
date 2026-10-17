app [program] { pf: platform "../roc-world/platform.roc" }

import pf.RocJson
import pf.RocCap

# A lounge world: the first example that uses the server's capability
# system. It subscribes to arrivals and ticks, greets every newcomer through
# `chat.say`, and polls the clock through `time.now`.
#
# Host events are decoded into JSON values and requests/records are encoded
# from values with the pure-Roc library provided by the platform.

Model : {
	joins : U64,
	denied : U64,
	cap : RocCap,
	last_now : Str,
}

program = { init, update, view, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { joins: 0, denied: 0, cap: RocCap.empty, last_now: "" }

update : Model, U64, Str -> Model
update = |model, _pid, ev| {
	match RocJson.decode(ev) {
		Err(_) => model
		Ok(event) => {
			cap = string_field(event, "cap")
			event_name = string_field(event, "event")
			if cap == "players" and event_name == "joined" {
				name = sanitize(string_field(field(event, "participant"), "name"))
				issued = RocCap.issue(model.cap, "chat.say")
				request = RocCap.chat_request(issued.id, Str.concat("Welcome, ", Str.concat(name, "!")))
				{ joins: model.joins + 1, denied: model.denied, cap: RocCap.attach(issued.session, issued.id, request), last_now: model.last_now }
			} else if cap == "time.tick" {
				issued = RocCap.issue(model.cap, "time.now")
				request = RocCap.now_request(issued.id)
				{ joins: model.joins, denied: model.denied, cap: RocCap.attach(issued.session, issued.id, request), last_now: model.last_now }
			} else if cap == "result" {
				id = string_field(event, "id")
				answered = RocCap.answered(model.cap, id)
				ok = bool_field(event, "ok")
				if answered == "chat.say" {
					denied = if ok { model.denied } else { model.denied + 1 }
					{ joins: model.joins, denied: denied, cap: RocCap.done(model.cap, id), last_now: model.last_now }
				} else if answered == "time.now" {
					last_now = if ok { number_field(field(event, "value"), "now") } else { model.last_now }
					{ joins: model.joins, denied: model.denied, cap: RocCap.done(model.cap, id), last_now: last_now }
				} else {
					model
				}
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
	RocJson.Object([
		{ key: "joins", value: RocJson.Number(model.joins.to_str()) },
		{ key: "denied", value: RocJson.Number(model.denied.to_str()) },
		{ key: "last_now", value: RocJson.Number(num_of(model.last_now).to_str()) },
	]),
)

## The capability protocol: subscriptions and currently outstanding requests.
wants : Model -> Str
wants = |model| encode_or_crash(
	RocJson.Object([
		{ key: "v", value: RocJson.Number("1") },
		{ key: "subscribe", value: RocJson.Array([RocJson.String("players"), RocJson.String("time.tick:5000")]) },
		{ key: "requests", value: RocJson.Array(RocCap.requests(model.cap)) },
	]),
)

## Keep the existing checkpoint format stable; each pending line is compact JSON.
snapshot : Model -> Str
snapshot = |model|
	"${model.joins.to_str()} ${model.denied.to_str()} ${RocCap.counter(model.cap).to_str()} ${model.last_now}\n${Str.join_with(List.map(RocCap.requests(model.cap), encode_or_crash), "\n")}"

restore : Str -> Model
restore = |text| {
	parts = Str.split_on(text, "\n")
	head = match List.first(parts) {
		Ok(line) => line
		Err(_) => ""
	}
	fields = Str.split_on(head, " ")
	documents = List.fold(
		List.drop_first(parts, 1),
		[],
		|acc, line| match RocJson.decode(line) {
			Ok(value) => List.append(acc, value)
			Err(_) => acc
		},
	)
	{
		joins: num(fields, 0),
		denied: num(fields, 1),
		cap: RocCap.from_pending(num(fields, 2), documents),
		last_now: part(fields, 3),
	}
}

# --- JSON/event helpers ----------------------------------------------------

## JSON generation is part of the app's output contract. Never replace an
## encoding failure with empty or malformed JSON.
encode_or_crash = |value| match RocJson.encode(value) {
	Ok(text) => text
	Err(_) => crash "lounge attempted to encode an invalid or oversized JSON value"
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

bool_field = |value, key| match RocJson.as_bool(field(value, key)) {
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
