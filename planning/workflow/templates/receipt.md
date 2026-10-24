# Receipt template

The receiver's answer to one [note](note.md). `resolve-note` creates `receipts/<note-id>.md` exclusively, so a note can be resolved only once. The receiver writes it, or the reconciler (`notes-reconcile`) on the receiver's behalf with `by` naming the reconciler. Reconcile receipts also record the remote refs they scanned (`check --remote-notes`).

````markdown
# Receipt: C001__to__C002__<id>

```workflow-receipt
{
  "at": "<UTC>",
  "by": "<receiver concept, or the reconciler's name>",
  "disposition": "<incorporated, acknowledged, declined, deferred or superseded>",
  "note": "notes/C001__to__C002__<id>.md",
  "reason": "<why>",
  "scanned": [
    {
      "ref": "origin/<branch>",
      "sha": "<sha>"
    }
  ],
  "target": "concepts/C002-<slug>.md#<anchor>"
}
```

- Note: [C001 to C002](../notes/C001__to__C002__<id>.md)
- Disposition: `<disposition>` — <reason>
- Landed in: [<target>](../concepts/C002-<slug>.md#<anchor>)
````
