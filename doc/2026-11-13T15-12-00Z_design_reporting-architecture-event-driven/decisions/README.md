# Reporting-architecture decision record index

**Package:** Reporting architecture — event-driven reporting, quota
forecasting, living report pages (#9568), design in
[`../note.md`](../note.md).
**Package record home:** this directory. **Canonical fleet-wide home:**
[`doc/decisions/`](../../decisions/README.md), per
[#9180](https://github.com/developing-today/code/issues/9180); this package
does not allocate or reserve fleet-wide IDs.
**Namespace:** `FDR-RA-NNN` for this package's records, following the
`FDR-PMO-NNN` precedent. The unprefixed `FDR-NNNN` series in the canonical
index is a distinct namespace and is not reused here.

## Records

| ID          | Record                                                                                             | Status                          | Domain / current links |
| ----------- | -------------------------------------------------------------------------------------------------- | ------------------------------- | ---------------------- |
| FDR-RA-001  | [Chat operating model: category side chats, insert-only job writes](FDR-RA-001-chat-operating-model-category-sinks.md) | accepted; implementation partially complete | #9569, #9568, #9180    |

## Adoption and maintenance

Records here follow the #9180 checklist (ID, title, date, status, owner,
decision maker, context, source links, options, operational decision,
rationale with fact/inference separation, consequences, revisit triggers,
supersession links). A record's `accepted` status cites the dated ruling it
rests on; open implementation items are carried in the record itself and on
its implementation issue, not left as implied state.
