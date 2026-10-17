## Helpers for the world capability protocol.
##
## A world program answers the server with `wants(model)`: a JSON object with
## `subscribe` and `requests`. Each request carries a program-chosen `id`; the
## server answers with `{"cap":"result","id":...,"ok":true,"value":...}` or
## `{"cap":"result","id":...,"ok":false,"error":...}`. Apps otherwise re-derive
## the same bookkeeping — ids, pending lists, result matching, argument
## documents — in every program. This module keeps that in one place.
##
## Everything here is pure; the app owns its model and decides when to send.
import RocJson
RocCap := {
	next : U64,
	pending : List({ id : Str, cap : Str, document : RocJson }),
}.{
	## A session with no outstanding requests.
	empty : RocCap
	empty = { next: 0, pending: [] }

	## Reserve the next request id under `cap`. Build the document with
	## `chat_request`/`now_request`/`players_request`/`world_request`, then pass
	## it to `attach` so `requests` can re-announce it after replay.
	issue : RocCap, Str -> { session : RocCap, id : Str }
	issue = |session, cap| {
		id = Str.concat("r", session.next.to_str())
		{
			session: {
				next: session.next + 1,
				pending: List.append(session.pending, { id: id, cap: cap, document: RocJson.Null }),
			},
			id: id,
		}
	}

	## Store the request document under its id.
	attach : RocCap, Str, RocJson -> RocCap
	attach = |session, id, document| {
		pending = List.map(
			session.pending,
			|entry| if entry.id == id { { id: entry.id, cap: entry.cap, document: document } } else { entry },
		)
		{ next: session.next, pending: pending }
	}

	## The outstanding request documents, in issue order, for `wants`.
	requests : RocCap -> List(RocJson)
	requests = |session| List.map(session.pending, |entry| entry.document)

	## The request counter, for snapshotting.
	counter : RocCap -> U64
	counter = |session| session.next

	## Rebuild a session from a counter and its pending documents.
	from_pending : U64, List(RocJson) -> RocCap
	from_pending = |next, documents| {
		entries = List.map(documents, |document| {
			id = string_field(document, "id")
			cap = string_field(document, "cap")
			{ id: id, cap: cap, document: document }
		})
		{ next: next, pending: entries }
	}

	## A string field of a JSON object, or `""` when missing or not a string.
	string_field : RocJson, Str -> Str
	string_field = |value, key| match RocJson.get(value, key) {
		Ok(found) => match RocJson.as_string(found) {
			Ok(text) => text
			Err(_) => ""
		}
		Err(_) => ""
	}

	## A `chat.say` request document.
	chat_request : Str, Str -> RocJson
	chat_request = |id, text|
		RocJson.Object([
			{ key: "id", value: RocJson.String(id) },
			{ key: "cap", value: RocJson.String("chat.say") },
			{ key: "args", value: RocJson.Object([{ key: "text", value: RocJson.String(text) }]) },
		])

	## A `time.now` request document.
	now_request : Str -> RocJson
	now_request = |id|
		RocJson.Object([
			{ key: "id", value: RocJson.String(id) },
			{ key: "cap", value: RocJson.String("time.now") },
		])

	## A `players.list` request document.
	players_request : Str -> RocJson
	players_request = |id|
		RocJson.Object([
			{ key: "id", value: RocJson.String(id) },
			{ key: "cap", value: RocJson.String("players.list") },
		])

	## A `world.info` request document.
	world_request : Str -> RocJson
	world_request = |id|
		RocJson.Object([
			{ key: "id", value: RocJson.String(id) },
			{ key: "cap", value: RocJson.String("world.info") },
		])

	## A `chat.history` request document. `count` is clamped by the server to
	## `MAX_CHAT_HISTORY_LINES` (64).
	history_request : Str, U64 -> RocJson
	history_request = |id, count|
		RocJson.Object([
			{ key: "id", value: RocJson.String(id) },
			{ key: "cap", value: RocJson.String("chat.history") },
			{ key: "args", value: RocJson.Object([{ key: "count", value: RocJson.Number(count.to_str()) }]) },
		])

	## The lines of a `chat.history` result value (`{"lines":[...]}`), or `[]`.
	history_lines : RocJson -> List(RocJson)
	history_lines = |value| list_field(value, "lines")

	## A `records.read` request document. `after` is exclusive; `""` starts at
	## the first key. Pages are byte-bounded; follow `next_after` while it is
	## non-empty, passing it as the next `after`.
	records_request : Str, Str, Str -> RocJson
	records_request = |id, prefix, after|
		RocJson.Object([
			{ key: "id", value: RocJson.String(id) },
			{ key: "cap", value: RocJson.String("records.read") },
			{
				key: "args",
				value: RocJson.Object([
					{ key: "prefix", value: RocJson.String(prefix) },
					{ key: "after", value: RocJson.String(after) },
				]),
			},
		])

	## The `{"key":...,"value":...}` entries of a `records.read` result.
	records_page : RocJson -> List(RocJson)
	records_page = |value| list_field(value, "records")

	## The cursor for the next `records.read`, or `""` when the page is the last.
	next_after : RocJson -> Str
	next_after = |value| string_field(value, "next_after")

	## An array field of a JSON object, or `[]` when missing or not an array.
	list_field : RocJson, Str -> List(RocJson)
	list_field = |value, key| match RocJson.get(value, key) {
		Ok(found) => match RocJson.as_array(found) {
			Ok(items) => items
			Err(_) => []
		}
		Err(_) => []
	}

	## The capability name a result `id` answers, or `""` when the id is not
	## pending (an earlier model's retry, or somebody else's request). Use it to
	## route the result exactly as the version that sent it would have.
	answered : RocCap, Str -> Str
	answered = |session, id| route(session.pending, id)

	## Retire the request a result answers.
	done : RocCap, Str -> RocCap
	done = |session, id|
		{ next: session.next, pending: List.keep_if(session.pending, |entry| entry.id != id) }

	## Whether any request is still outstanding.
	is_empty : RocCap -> Bool
	is_empty = |session| List.is_empty(session.pending)

	route : List({ id : Str, cap : Str, document : RocJson }), Str -> Str
	route = |pending, id|
		List.fold(pending, "", |found, entry| if found == "" and entry.id == id { entry.cap } else { found })
}
