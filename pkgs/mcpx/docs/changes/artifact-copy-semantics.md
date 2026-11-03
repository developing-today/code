sections: delivery, the-script-api-is-the-same-everywhere, what-a-remote-endpoint-is-not
kind: Changed
summary: artifact delivery prose aligned with the implemented copy semantics (#3064): the exec and remote docs no longer promise hardlink delivery or zero-copy cost where the artifacts package copies bytes; the #1660 stored-blob guarantee (in-place edits to outside files cannot mutate stored bytes) is unchanged
