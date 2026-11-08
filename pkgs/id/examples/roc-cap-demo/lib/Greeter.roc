## Demo library: greet world participants by name.
##
## The names live in the world records, not in this library, so the
## library is NOT pure: it depends on the platform providing the
## records.read capability (through the RocCap protocol helpers) to
## fetch them. A platform without that capability cannot host this
## library at all — see the negative tests in ../roc-cap-demo-test.sh, which are
## wired into scripts/roc-test.sh (#5957).
import pf.RocCap
import pf.RocJson

Greeter := {
	session : RocCap,
	names : List(Str),
}.{
	## A greeter that has not asked for names yet.
	new : Greeter
	new = { session: RocCap.empty, names: [] }

	## Ask the platform for the participant-name records (prefix
	## "names/"). Returns the updated greeter and the request id; the
	## app announces the greeter requests (see wants) and routes the
	## result back through absorb_result.
	ask_for_names : Greeter -> { greeter : Greeter, id : Str }
	ask_for_names = |greeter| {
		issued = RocCap.issue(greeter.session, "records.read")
		{
			greeter: { session: RocCap.attach(issued.session, issued.id, RocCap.records_request(issued.id, "names/", "")), names: greeter.names },
			id: issued.id,
		}
	}

	## Absorb one decoded {"cap":"result",...} event. A records.read
	## answer teaches the greeter the participant names; any other
	## outstanding request is simply retired. Results for ids this
	## greeter never issued are ignored, so journal replay and stale
	## answers cannot corrupt the model.
	absorb_result : Greeter, RocJson -> Greeter
	absorb_result = |greeter, event| {
		id = string_field(event, "id")
		answered = RocCap.answered(greeter.session, id)
		if answered == "records.read" and bool_field(event, "ok") {
			{ session: RocCap.done(greeter.session, id), names: names_from_page(field(event, "value")) }
		} else if answered != "" {
			{ session: RocCap.done(greeter.session, id), names: greeter.names }
		} else {
			greeter
		}
	}

	## The wants document announcing the outstanding requests, in the
	## shape the world host expects (compare roc-chat).
	wants : Greeter -> Str
	wants = |greeter| encode_or_crash(
		RocJson.Object([
			{ key: "v", value: RocJson.Number("1") },
			{ key: "requests", value: RocJson.Array(RocCap.requests(greeter.session)) },
		]),
	)

	## The greeting for every participant name learned so far.
	greetings : Greeter -> List(Str)
	greetings = |greeter| List.map(greeter.names, greet)

	## The greeting for one participant name.
	greet : Str -> Str
	greet = |name| Str.concat("hello, ", name)

	## The participant names from a records.read result page: each
	## record entry carries its name as the string value.
	names_from_page : RocJson -> List(Str)
	names_from_page = |page|
		List.fold(RocCap.records_page(page), [], |names, entry|
			match RocJson.as_string(field(entry, "value")) {
				Ok(name) => List.append(names, name)
				Err(_) => names
			})

	## The names learned so far.
	known_names : Greeter -> List(Str)
	known_names = |greeter| greeter.names

	## The session request counter, for snapshotting.
	counter : Greeter -> U64
	counter = |greeter| RocCap.counter(greeter.session)

	## The outstanding request documents, for snapshotting.
	pending_documents : Greeter -> List(RocJson)
	pending_documents = |greeter| RocCap.requests(greeter.session)

	## Rebuild a greeter from its snapshot parts (compare
	## RocCap.from_pending).
	from_parts : U64, List(RocJson), List(Str) -> Greeter
	from_parts = |next, documents, names| { session: RocCap.from_pending(next, documents), names: names }

	field : RocJson, Str -> RocJson
	field = |value, key| match RocJson.get(value, key) {
		Ok(found) => found
		Err(_) => RocJson.Null
	}

	string_field : RocJson, Str -> Str
	string_field = |value, key| match RocJson.as_string(field(value, key)) {
		Ok(text) => text
		Err(_) => ""
	}

	bool_field : RocJson, Str -> Bool
	bool_field = |value, key| match RocJson.as_bool(field(value, key)) {
		Ok(flag) => flag
		Err(_) => Bool.False
	}

	encode_or_crash : RocJson -> Str
	encode_or_crash = |value| match RocJson.encode(value) {
		Ok(text) => text
		Err(_) => crash "Greeter attempted to encode an invalid or oversized JSON value"
	}
}

sample_page : RocJson
sample_page = RocJson.Object([
	{
		key: "records",
		value: RocJson.Array([
			RocJson.Object([
				{ key: "key", value: RocJson.String("names/1") },
				{ key: "value", value: RocJson.String("ada") },
			]),
			RocJson.Object([
				{ key: "key", value: RocJson.String("names/2") },
				{ key: "value", value: RocJson.String("grace") },
			]),
		]),
	},
	{ key: "next_after", value: RocJson.String("") },
])

sample_result : RocJson
sample_result = RocJson.Object([
	{ key: "cap", value: RocJson.String("result") },
	{ key: "id", value: RocJson.String("r0") },
	{ key: "ok", value: RocJson.Bool(Bool.True) },
	{ key: "value", value: sample_page },
])

expect Greeter.greet("ada") == "hello, ada"
expect Greeter.ask_for_names(Greeter.new).id == "r0"
expect Greeter.names_from_page(sample_page) == ["ada", "grace"]
expect Greeter.greetings(Greeter.absorb_result(Greeter.ask_for_names(Greeter.new).greeter, sample_result)) == ["hello, ada", "hello, grace"]
expect Str.contains(Greeter.wants(Greeter.ask_for_names(Greeter.new).greeter), "records.read") == Bool.True
