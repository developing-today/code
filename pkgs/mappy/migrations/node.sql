CREATE TABLE nodes (
    --
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    --
    graph_id UUID REFERENCES nodes(id),
    class_id UUID REFERENCES nodes(id),
    match_id UUID REFERENCES nodes(id),
    merge_id UUID REFERENCES nodes(id),

    --
    name_id UUID REFERENCES nodes(id),
    type_id UUID REFERENCES nodes(id),
    kind_id UUID REFERENCES nodes(id),
    sort_id UUID REFERENCES nodes(id),
    tier_id UUID REFERENCES nodes(id),

    --
    mode_id UUID REFERENCES nodes(id),
    code_id UUID REFERENCES nodes(id),
    hash_id UUID REFERENCES nodes(id),

    --
    item_id UUID REFERENCES nodes(id),
    part_id UUID REFERENCES nodes(id),
    slot_id UUID REFERENCES nodes(id),

    --
    lead_id UUID REFERENCES nodes(id),
    peer_id UUID REFERENCES nodes(id),

    --
    link_id UUID REFERENCES nodes(id),
    root_id UUID REFERENCES nodes(id),
    twig_id UUID REFERENCES nodes(id),
    leaf_id UUID REFERENCES nodes(id),

    --
    universal TEXT,
    generic TEXT,
    identifier TEXT,
    specific TEXT,

    --
    short TEXT,
    string TEXT,
    summary TEXT,
    extended TEXT,

    --
    priority INTEGER,
    weight INTEGER,
    score INTEGER,
    location POINT,
    locations MULTIPOINT,
    shape GEOMETRYCOLLECTION,
    map GEOMETRYCOLLECTION,

    --
    acl JSON,

    --
    kind JSON,
    kinds JSON,
    scope JSON,
    scopes JSON,

    --
    alias JSON,
    aliases JSON,

    --
    details JSON,
    content JSON,
    contents JSON,

    --
    arguments JSON,
    properties JSON,
    annotations JSON,

    --
    public JSON,
    unlisted JSON,
    personal JSON,

    --
    owners JSON,
    admins JSON,

    --
    note JSON,
    notes JSON,

    --
    created_at TIMESTAMPTZ DEFAULT now(),
    created_by JSON,

    --
    updated_at TIMESTAMPTZ DEFAULT now(),
    updated_by JSON,

    --
    expired_at TIMESTAMPTZ DEFAULT now(),
    expired_by JSON

);
