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
	pending : List({ id : Str, cap : Str }),
}.{
	## A session with no outstanding requests.
	empty : RocCap
	empty = { next: 0, pending: [] }

	## Track a pending request under its capability name and get the request
	## `id` to use. Call once per request document you append to `wants`.
	issue : RocCap, Str -> { session : RocCap, id : Str }
	issue = |session, cap| {
		id = Str.concat("r", session.next.to_str())
		{
			session: {
				next: session.next + 1,
				pending: List.append(session.pending, { id: id, cap: cap }),
			},
			id: id,
		}
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

	## The capability name a result `id` answers, or `""` when the id is not
	## pending (an earlier model's retry, or somebody else's request). Use it
	## to route the result exactly as the version that sent it would have.
	answered : RocCap, Str -> Str
	answered = |session, id|
		route(session.pending, id)

	## Retire the request a result answers.
	done : RocCap, Str -> RocCap
	done = |session, id|
		{ next: session.next, pending: List.keep_if(session.pending, |entry| entry.id != id) }

	## Whether any request is still outstanding.
	is_empty : RocCap -> Bool
	is_empty = |session| List.is_empty(session.pending)

	route : List({ id : Str, cap : Str }), Str -> Str
	route = |pending, id|
		List.fold(pending, "", |found, entry| if found == "" and entry.id == id { entry.cap } else { found })
}
