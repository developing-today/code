# Note template

A directed note from one concept's owner to another's, for any change or notice that belongs in a file the sender does not own. `note` writes it once as `notes/<from>__to__<to>__<id>.md`; it is never edited afterwards. The receiver answers with a [receipt](receipt.md).

````markdown
# C001 to C002

```workflow-note
{
  "at": "<UTC>",
  "body_sha256": "<sha256 of the body below>",
  "from": "C001",
  "id": "C001__to__C002__<id>",
  "to": "C002"
}
```

<What the receiver should know or change, and why, with links to the evidence. One note per topic.>
````
