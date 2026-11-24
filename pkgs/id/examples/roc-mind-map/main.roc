app [program] { pf: platform "../roc-world/platform.roc" }

import pf.RocJson
import pf.Screen

# A mind map for mind-map/same-label-distinct-nodes: nodes are identified by
# stable IDs, so two nodes can share the label "Archive". A typed cross-link
# keeps pointing at its node through reparent and rename. A reparent that
# would put a node under its own descendant is refused and leaves the tree
# unchanged. Positions are never changed by an operation.
#
# Each player event is one JSON command. Host events (participant 0) are ignored.
# `records` publishes the domain state, and `snapshot` is the same document.

Node : { id : Str, name : Str, parent : Str, x : U64, y : U64 }
Link : { from : Str, to : Str }
Model : { root : Str, nodes : List(Node), links : List(Link), outcome : Str }

program = { init, update, view, layout, records, wants, snapshot, restore }

init : U64 -> Model
init = |_seed| initial

initial : Model
initial = {
    root: "R",
    nodes: [
        { id: "P", name: "North", parent: "R", x: 0, y: 1 },
        { id: "Q", name: "South", parent: "R", x: 2, y: 1 },
        { id: "R", name: "World", parent: "", x: 1, y: 0 },
        { id: "U", name: "Archive", parent: "P", x: 0, y: 2 },
        { id: "V", name: "Archive", parent: "Q", x: 2, y: 2 },
    ],
    links: [],
    outcome: "",
}

update : Model, U64, Str -> Model
update = |model, pid, ev|
    if pid == 0 {
        model
    } else {
        command(model, ev)
    }

command : Model, Str -> Model
command = |model, text| match RocJson.decode(text) {
    Err(_) => { ..model, outcome: "invalid" }
    Ok(cmd) => {
        kind = string_field(cmd, "kind")
        if kind == "AddCrossLink" {
            add_link(model, string_field(cmd, "source"), string_field(cmd, "target"))
        } else if kind == "Reparent" {
            reparent(model, string_field(cmd, "id"), string_field(cmd, "new_parent"))
        } else if kind == "Rename" {
            rename(model, string_field(cmd, "id"), string_field(cmd, "name"))
        } else {
            { ..model, outcome: "invalid" }
        }
    }
}

add_link : Model, Str, Str -> Model
add_link = |model, source, target|
    if source != target and has_node(model.nodes, source) and has_node(model.nodes, target) and !has_link(model.links, source, target) {
        { ..model, links: List.append(model.links, { from: source, to: target }), outcome: "linked" }
    } else {
        { ..model, outcome: "refused" }
    }

reparent : Model, Str, Str -> Model
reparent = |model, id, new_parent|
    if id == model.root or !has_node(model.nodes, id) or !has_node(model.nodes, new_parent) {
        { ..model, outcome: "refused" }
    } else if reaches(model.nodes, new_parent, id, List.len(model.nodes)) {
        { ..model, outcome: "refused-cycle" }
    } else {
        { ..model, nodes: List.map(model.nodes, |n| if n.id == id { { ..n, parent: new_parent } } else { n }), outcome: "moved" }
    }

rename : Model, Str, Str -> Model
rename = |model, id, name|
    if has_node(model.nodes, id) {
        { ..model, nodes: List.map(model.nodes, |n| if n.id == id { { ..n, name: name } } else { n }), outcome: "renamed" }
    } else {
        { ..model, outcome: "refused" }
    }

# Walks up from `start`; true when `target` is reached. `fuel` bounds the walk.
reaches : List(Node), Str, Str, U64 -> Bool
reaches = |nodes, start, target, fuel|
    if start == target {
        Bool.True
    } else if fuel == 0 {
        Bool.False
    } else match find_node(nodes, start) {
        Ok(found) => reaches(nodes, found.parent, target, fuel - 1)
        Err(_) => Bool.False
    }

has_node : List(Node), Str -> Bool
has_node = |nodes, id| List.any(nodes, |n| n.id == id)

has_link : List(Link), Str, Str -> Bool
has_link = |links, from, to| List.any(links, |l| l.from == from and l.to == to)

find_node : List(Node), Str -> Try(Node, [NotFound])
find_node = |nodes, id| match List.find_first(nodes, |n| n.id == id) {
    Ok(found) => Ok(found)
    Err(_) => Err(NotFound)
}

plain = { fg: 0, bg: 0, bold: Bool.False }

view : Model -> Str
view = |model| {
    frame = Screen.boxed(Screen.new(40, 10), 0, 0, 40, 10, "mind map")
    drawn = List.fold(model.nodes, frame, |screen, n|
        Screen.put_text(screen, n.x * 14 + 2, n.y * 2 + 1, "${n.id} ${n.name}", plain))
    links = Str.join_with(List.map(model.links, |l| "${l.from}-${l.to}"), " ")
    Screen.render_json(
        Screen.put_text(Screen.put_text(drawn, 2, 7, "last=${model.outcome}", plain), 2, 8, "links=${links}", plain),
    )
}

## This program renders the same layout at any viewport.
layout : Model, U64, U64 -> Str
layout = |model, _width, _height| view(model)

records : Model -> Str
records = |model| encode_or_crash(document(model))

## No subscriptions and no requests.
wants : Model -> Str
wants = |_model| "{}"

snapshot : Model -> Str
snapshot = |model| encode_or_crash(document(model))

restore : Str -> Model
restore = |text| match RocJson.decode(text) {
    Err(_) => initial
    Ok(doc) =>
        if string_field(doc, "root") == "" {
            initial
        } else {
            {
                root: string_field(doc, "root"),
                nodes: List.map(array_field(doc, "nodes"), node_of),
                links: List.map(array_field(doc, "cross_links"), link_of),
                outcome: string_field(doc, "outcome"),
            }
        }
}

# --- helpers ---------------------------------------------------------------

document : Model -> RocJson
document = |model| RocJson.Object([
    { key: "root", value: RocJson.String(model.root) },
    { key: "nodes", value: RocJson.Array(List.map(model.nodes, node_json)) },
    { key: "cross_links", value: RocJson.Array(List.map(model.links, link_json)) },
    { key: "outcome", value: RocJson.String(model.outcome) },
])

node_json : Node -> RocJson
node_json = |n| RocJson.Object([
    { key: "id", value: RocJson.String(n.id) },
    { key: "name", value: RocJson.String(n.name) },
    { key: "parent", value: if n.parent == "" { RocJson.Null } else { RocJson.String(n.parent) } },
    { key: "position", value: RocJson.Array([RocJson.Number(n.x.to_str()), RocJson.Number(n.y.to_str())]) },
])

link_json : Link -> RocJson
link_json = |l| RocJson.Object([
    { key: "from", value: RocJson.String(l.from) },
    { key: "to", value: RocJson.String(l.to) },
])

node_of : RocJson -> Node
node_of = |item| {
    id: string_field(item, "id"),
    name: string_field(item, "name"),
    parent: string_field(item, "parent"),
    x: position_at(item, 0),
    y: position_at(item, 1),
}

link_of : RocJson -> Link
link_of = |item| { from: string_field(item, "from"), to: string_field(item, "to") }

encode_or_crash = |value| match RocJson.encode(value) {
    Ok(text) => text
    Err(_) => crash "mind-map attempted to encode an invalid or oversized JSON value"
}

field = |value, key| match RocJson.get(value, key) {
    Ok(found) => found
    Err(_) => RocJson.Null
}

string_field = |value, key| match RocJson.as_string(field(value, key)) {
    Ok(text) => text
    Err(_) => ""
}

array_field = |value, key| match RocJson.as_array(field(value, key)) {
    Ok(items) => items
    Err(_) => []
}

position_at = |item, index| match RocJson.as_array(field(item, "position")) {
    Ok(pos) => match List.get(pos, index) {
        Ok(found) => number(found)
        Err(_) => 0
    }
    Err(_) => 0
}

number = |value| match RocJson.as_number(value) {
    Ok(text) => match U64.from_str(text) {
        Ok(found) => found
        Err(_) => 0
    }
    Err(_) => 0
}
