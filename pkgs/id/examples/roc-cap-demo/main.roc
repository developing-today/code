app [program] { pf: platform "../roc-world/platform.roc", caplib: "./lib/main.roc" }

import pf.RocJson
import caplib.Greeter

# A demo app for the Greeter library (lib/): on init it asks the
# platform for the participant-name records through the library, and
# its view greets every participant by name once the records.read
# result arrives. All capability protocol handling — issuing the
# request, announcing it in wants, routing the result, retiring it —
# lives in the library; this app only decodes host events and renders.

Model : { greeter : Greeter }

program = { init, update, view, layout, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| { greeter: Greeter.ask_for_names(Greeter.new).greeter }

update : Model, U64, Str -> Model
update = |model, pid, ev|
	if pid == 0 { host(model, ev) } else { model }

## Host events (participant 0): capability results are routed by the
## library, keyed on the request id it issued.
host : Model, Str -> Model
host = |model, ev| match RocJson.decode(ev) {
	Err(_) => model
	Ok(event) => if string_field(event, "cap") == "result" {
		{ greeter: Greeter.absorb_result(model.greeter, event) }
	} else {
		model
	}
}

view : Model -> Str
view = |model| {
	greetings = Greeter.greetings(model.greeter)
	if List.is_empty(greetings) {
		"cap-demo: awaiting participant names"
	} else {
		Str.concat("cap-demo: ", Str.join_with(greetings, ", "))
	}
}

## This program renders the same layout at any viewport.
layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

records : Model -> Str
records = |model| encode_or_crash(
	RocJson.Object([{ key: "greeted", value: RocJson.Number(List.len(Greeter.known_names(model.greeter)).to_str()) }]),
)

## The library announces its outstanding capability requests.
wants : Model -> Str
wants = |model| Greeter.wants(model.greeter)

snapshot : Model -> Str
snapshot = |model| {
	documents = Str.join_with(List.map(Greeter.pending_documents(model.greeter), encode_or_crash), "\n")
	names = encode_or_crash(RocJson.Array(List.map(Greeter.known_names(model.greeter), |name| RocJson.String(name))))
	"${Greeter.counter(model.greeter).to_str()}\n${names}\n${documents}"
}

restore : Str -> Model
restore = |text| {
	parts = Str.split_on(text, "\n")
	documents = List.fold(List.drop_first(parts, 2), [], |acc, line| match RocJson.decode(line) {
		Ok(value) => List.append(acc, value)
		Err(_) => acc
	})
	{ greeter: Greeter.from_parts(num(parts, 0), documents, names_from_line(part(parts, 1))) }
}

# --- helpers ---------------------------------------------------------------

encode_or_crash = |value| match RocJson.encode(value) {
	Ok(text) => text
	Err(_) => crash "roc-cap-demo attempted to encode an invalid or oversized JSON value"
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

names_from_line = |line| match RocJson.decode(line) {
	Ok(value) => match RocJson.as_array(value) {
		Ok(items) => List.fold(items, [], |names, item| match RocJson.as_string(item) {
			Ok(name) => List.append(names, name)
			Err(_) => names
		})
		Err(_) => []
	}
	Err(_) => []
}
