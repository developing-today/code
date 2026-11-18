# lane-scratch: code-d task 20078 — #8160 items 1-4 patch (ingest.rs, waiting_rows.rs, mod.rs)
import pathlib

base = pathlib.Path("pkgs/id/crates/id-events/src")

def patch(path, pairs):
    p = base / path
    s = p.read_text()
    for old, new in pairs:
        assert s.count(old) == 1, f"{path}: anchor count {s.count(old)} for {old[:70]!r}"
        s = s.replace(old, new)
    p.write_text(s)
    print(f"{path}: {len(pairs)} edits ok")

# ---------------------------------------------------------------- ingest.rs
patch("event_store/ingest.rs", [
    # 1. ingest_with_body: device op goes INTO the ingest transaction (item 3);
    #    the post-hoc record_device_op call goes away.
    ("""        let result = self.ingest_event(event, action.as_ref(), body, now_ms)?;
        if let Some(body) = retain_body
            && !matches!(result, Ingest::Refused(_))
        {
            self.record_writer_body(&event_id, body)?;
        }
        if let Some((op, group)) = &device
            && !matches!(result, Ingest::Refused(_))
        {
            self.record_device_op(&event_id, op, group.as_ref())?;
        }
        Ok(result)""",
     """        let result = self.ingest_event(event, action.as_ref(), body, device.as_ref(), now_ms)?;
        if let Some(body) = retain_body
            && !matches!(result, Ingest::Refused(_))
        {
            self.record_writer_body(&event_id, body)?;
        }
        Ok(result)"""),
    # 2. record_device_op (own transaction) is replaced by the in-transaction
    #    free functions next to store_action (below); delete the method here.
    ("""    /// Records the device operation a verified `group.event` body carries, by event ID
    /// (issue #5656). Written after the ingest decision, in its own transaction, like
    /// [`EventStore::record_writer_body`]: the row is keyed by the event's ID and read
    /// back only for events the store holds, so recording one for an event that is held
    /// or later released is inert until the event is stored, and a duplicate delivery
    /// simply rewrites the same row.
    ///
    /// # Errors
    ///
    /// Returns a redb error if the write fails, or [`StoreError::Corrupt`] if the op
    /// does not encode.
    fn record_device_op(
        &self,
        id: &Hash,
        op: &crate::groups::DeviceOp,
        group: Option<&Hash>,
    ) -> Result<(), StoreError> {
        let encoded = encode_device_op(op, group)?;
        let txn = self.db.begin_write()?;
        {
            let mut device_ops = txn.open_table(DEVICE_OPS)?;
            device_ops.insert(id.as_bytes().as_slice(), encoded.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

""", ""),
    # 3. ingest_with_action: no device op.
    ("        self.ingest_event(event, action, None, now_ms)",
     "        self.ingest_event(event, action, None, None, now_ms)"),
    # 4. ingest_event threads the device op into the transaction.
    ("""    fn ingest_event(
        &mut self,
        event: Event,
        action: Option<&GroupAction<AuthId, ()>>,
        body: Option<&[u8]>,
        now_ms: u64,
    ) -> Result<Ingest, StoreError> {
        self.transact(now_ms, |mirror| {
            let (staged, changed) =
                ingest_in(mirror, event, action, body, now_ms, Labels::AsStored)?;
            Ok((staged.finish(mirror.txn)?, changed))
        })
    }""",
     """    fn ingest_event(
        &mut self,
        event: Event,
        action: Option<&GroupAction<AuthId, ()>>,
        body: Option<&[u8]>,
        device: Option<&(crate::groups::DeviceOp, Option<Hash>)>,
        now_ms: u64,
    ) -> Result<Ingest, StoreError> {
        self.transact(now_ms, |mirror| {
            let (staged, changed) =
                ingest_in(mirror, event, action, body, device, now_ms, Labels::AsStored)?;
            Ok((staged.finish(mirror.txn)?, changed))
        })
    }"""),
    # 5. ingest_events: tuples gain a device slot (none — no bodies).
    ("        let events = events.into_iter().map(|event| (event, None)).collect();",
     "        let events = events.into_iter().map(|event| (event, None, None)).collect();"),
    # 6. ingest_events_with_bodies: decode each body's device operation in the
    #    pre-pass (item 1); a claimed device body that does not decode refuses
    #    its event in place, exactly as the production path refuses it.
    ("""    pub(crate) fn ingest_events_with_bodies(
        &mut self,
        events: Vec<(Event, Option<Vec<u8>>)>,
        now_ms: u64,
    ) -> Result<Vec<Ingest>, StoreError> {
        let mut refusals: Vec<Option<Ingest>> = Vec::with_capacity(events.len());
        let mut accepted: Vec<(Event, Option<Vec<u8>>)> = Vec::with_capacity(events.len());
        for (event, body) in events {
            match body {
                Some(body) if committed(&event, &body).is_err() => {
                    refusals.push(Some(Ingest::Refused(Refusal::Malformed(
                        "the body is not the one the header commits to, by hash and size"
                            .to_owned(),
                    ))));
                }
                body => {
                    refusals.push(None);
                    accepted.push((event, body));
                }
            }
        }""",
     """    pub(crate) fn ingest_events_with_bodies(
        &mut self,
        events: Vec<(Event, Option<Vec<u8>>)>,
        now_ms: u64,
    ) -> Result<Vec<Ingest>, StoreError> {
        let mut refusals: Vec<Option<Ingest>> = Vec::with_capacity(events.len());
        let mut accepted: Vec<(Event, Option<Vec<u8>>, Option<(crate::groups::DeviceOp, Option<Hash>)>)> =
            Vec::with_capacity(events.len());
        for (event, body) in events {
            match body {
                Some(body) if committed(&event, &body).is_err() => {
                    refusals.push(Some(Ingest::Refused(Refusal::Malformed(
                        "the body is not the one the header commits to, by hash and size"
                            .to_owned(),
                    ))));
                }
                body => match body_device_op(&event, body.as_deref()) {
                    Ok(device) => {
                        refusals.push(None);
                        accepted.push((event, body, device));
                    }
                    Err(refusal) => {
                        refusals.push(Some(Ingest::Refused(refusal)));
                    }
                },
            }
        }"""),
    # 7. ingest_events_inner: tuple + ingest_in call carry the device op.
    ("""    fn ingest_events_inner(
        &mut self,
        events: Vec<(Event, Option<Vec<u8>>)>,
        now_ms: u64,
    ) -> Result<Vec<Ingest>, StoreError> {""",
     """    fn ingest_events_inner(
        &mut self,
        events: Vec<(
            Event,
            Option<Vec<u8>>,
            Option<(crate::groups::DeviceOp, Option<Hash>)>,
        )>,
        now_ms: u64,
    ) -> Result<Vec<Ingest>, StoreError> {"""),
    ("""            for (event, body) in events {
                let (outcome, did_change) =
                    ingest_in(mirror, event, None, body.as_deref(), now_ms, labels)?;""",
     """            for (event, body, device) in events {
                let (outcome, did_change) = ingest_in(
                    mirror,
                    event,
                    None,
                    body.as_deref(),
                    device.as_ref(),
                    now_ms,
                    labels,
                )?;"""),
    # 8. ingest_in: thread the device op to settle.
    ("""fn ingest_in(
    m: &mut Mirror<'_>,
    event: Event,
    action: Option<&GroupAction<AuthId, ()>>,
    body: Option<&[u8]>,
    now: u64,
    labels: Labels,
) -> Result<(Staged, bool), StoreError> {
    let expired = m.expire(now)?;
    let id = event.id;
    let author = author_of(&event);
    let rivals = held_rivals(m.pool, &event);
    let settled = settle(m, event, action, body, &rivals, now, false, labels)?;""",
     """fn ingest_in(
    m: &mut Mirror<'_>,
    event: Event,
    action: Option<&GroupAction<AuthId, ()>>,
    body: Option<&[u8]>,
    device: Option<&(crate::groups::DeviceOp, Option<Hash>)>,
    now: u64,
    labels: Labels,
) -> Result<(Staged, bool), StoreError> {
    let expired = m.expire(now)?;
    let id = event.id;
    let author = author_of(&event);
    let rivals = held_rivals(m.pool, &event);
    let settled = settle(m, event, action, body, device, &rivals, now, false, labels)?;"""),
    # 9. settle signature.
    ("""fn settle(
    m: &mut Mirror<'_>,
    event: Event,
    action: Option<&GroupAction<AuthId, ()>>,
    body: Option<&[u8]>,
    rivals: &[Event],
    now: u64,
    from_pool: bool,
    labels: Labels,
) -> Result<Settled, StoreError> {""",
     """fn settle(
    m: &mut Mirror<'_>,
    event: Event,
    action: Option<&GroupAction<AuthId, ()>>,
    body: Option<&[u8]>,
    device: Option<&(crate::groups::DeviceOp, Option<Hash>)>,
    rivals: &[Event],
    now: u64,
    from_pool: bool,
    labels: Labels,
) -> Result<Settled, StoreError> {"""),
    # 10. Accept branch: the row is written in this transaction, before the
    #     resolution below reads the tables (item 3).
    ("""            if let Some(body) = body {
                store_body(m.txn, &id, body)?;
            }
            // In a store with no group state every stored event applies, so the label is known""",
     """            if let Some(body) = body {
                store_body(m.txn, &id, body)?;
            }
            if let Some((op, group)) = device {
                store_device_op(m.txn, &id, op, group.as_ref())?;
            }
            // In a store with no group state every stored event applies, so the label is known"""),
    # 11. Duplicate branch: a body arriving late brings its device op with it.
    ("""            let mut stored_body = false;
            if let Some(body) = body {
                if is_stored(m.txn, &id)? {
                    store_body(m.txn, &id, body)?;
                    stored_body = true;
                }
            }""",
     """            let mut stored_body = false;
            if let Some(body) = body {
                if is_stored(m.txn, &id)? {
                    store_body(m.txn, &id, body)?;
                    stored_body = true;
                    if let Some((op, group)) = device {
                        store_device_op(m.txn, &id, op, group.as_ref())?;
                    }
                }
            }"""),
    # 12. Fork branch: a released event that forks leaves no row behind.
    ("""            drop_released(m, &event, from_pool)?;
            refresh(m.txn)?;
            let mut released = Vec::new();""",
     """            drop_released(m, &event, from_pool)?;
            if from_pool {
                remove_device_op(m.txn, &id)?;
            }
            refresh(m.txn)?;
            let mut released = Vec::new();"""),
    # 13. Refuse branch: a released event that is refused leaves no row behind.
    ("""        Decision::Refuse(refusal) => {
            drop_released(m, &event, from_pool)?;
            Settled::Refused(refusal)
        }""",
     """        Decision::Refuse(refusal) => {
            drop_released(m, &event, from_pool)?;
            if from_pool {
                remove_device_op(m.txn, &id)?;
            }
            Settled::Refused(refusal)
        }"""),
    # 14. Wait branch: the row is written only once the event is actually
    #     held — the #2916 rule the action already follows (items 2+3).
    ("""        Decision::Wait(deps) => {
            let hold = m.hold(event, deps, false, now)?;
            if hold == Hold::Held {
                // #2916: the action is written only once the event is
                // actually held. Writing it before the hold persisted the
                // refused event's action whenever the same call committed
                // for an unrelated reason (e.g. an expiry).
                if let Some(action) = action {
                    store_action(m.txn, &id, action)?;
                }
                if let Some(body) = body {
                    store_body(m.txn, &id, body)?;
                }
            }
            held(hold)
        }""",
     """        Decision::Wait(deps) => {
            let hold = m.hold(event, deps, false, now)?;
            if hold == Hold::Held {
                // #2916: the action is written only once the event is
                // actually held. Writing it before the hold persisted the
                // refused event's action whenever the same call committed
                // for an unrelated reason (e.g. an expiry).
                if let Some(action) = action {
                    store_action(m.txn, &id, action)?;
                }
                if let Some(body) = body {
                    store_body(m.txn, &id, body)?;
                }
                if let Some((op, group)) = device {
                    store_device_op(m.txn, &id, op, group.as_ref())?;
                }
            }
            held(hold)
        }"""),
    # 15. Park branch: same rule as Wait.
    ("""        Decision::Park => {
            let hold = m.hold(event, Vec::new(), true, now)?;
            if hold == Hold::Held {
                if let Some(action) = action {
                    store_action(m.txn, &id, action)?;
                }
                if let Some(body) = body {
                    store_body(m.txn, &id, body)?;
                }
            }
            held(hold)
        }""",
     """        Decision::Park => {
            let hold = m.hold(event, Vec::new(), true, now)?;
            if hold == Hold::Held {
                if let Some(action) = action {
                    store_action(m.txn, &id, action)?;
                }
                if let Some(body) = body {
                    store_body(m.txn, &id, body)?;
                }
                if let Some((op, group)) = device {
                    store_device_op(m.txn, &id, op, group.as_ref())?;
                }
            }
            held(hold)
        }"""),
    # 16. drain: a released event's device op is re-derived from its stored
    #     (verified) body, so the decision that finally stores it writes the
    #     row in the same transaction.
    ("""        let action = stored_action(m.txn, &id)?;
        match settle(m, event, action.as_ref(), None, &rivals, now, true, labels)? {""",
     """        let action = stored_action(m.txn, &id)?;
        let device = stored_device_op(m.txn, &event)?;
        match settle(
            m,
            event,
            action.as_ref(),
            None,
            device.as_ref(),
            &rivals,
            now,
            true,
            labels,
        )? {"""),
    # 17. body_device_op becomes visible to the parent module (recovery, item 4).
    ("fn body_device_op(", "pub(super) fn body_device_op("),
    # 18. In-transaction device-op row helpers, next to store_action.
    ("""/// Attaches the action of a group operation to a stored event that has none.""",
     """/// Writes the device operation an event's verified body carries, by event ID, in the
/// caller's transaction (issue #8160, item 3): the row commits or aborts with the
/// ingest decision itself — stored, duplicate and waiting paths alike — so a crash
/// or a failed write can no longer leave a stored event without the operation its
/// body carries, or an operation without its event. The resolution reads the row
/// back only for events the store holds.
fn store_device_op(
    txn: &WriteTransaction,
    id: &Hash,
    op: &crate::groups::DeviceOp,
    group: Option<&Hash>,
) -> Result<(), StoreError> {
    let encoded = encode_device_op(op, group)?;
    let mut device_ops = txn.open_table(DEVICE_OPS)?;
    device_ops.insert(id.as_bytes().as_slice(), encoded.as_slice())?;
    Ok(())
}

/// Removes an event's device-operation row, in the caller's transaction. Used when
/// an event that carried a row stops being held without being stored — a held
/// event that expires, or a released event that is refused or forks — so the row
/// never outlives the event it describes (issue #8160, item 2). Removing a row
/// that does not exist is not an error.
fn remove_device_op(txn: &WriteTransaction, id: &Hash) -> Result<(), StoreError> {
    let mut device_ops = txn.open_table(DEVICE_OPS)?;
    device_ops.remove(id.as_bytes().as_slice())?;
    Ok(())
}

/// The device operation carried by the body the store already holds for `event`,
/// for an event the waiting pool releases: the body was verified against the
/// header when it was stored, so a body that no longer decodes is corruption.
fn stored_device_op(
    txn: &WriteTransaction,
    event: &Event,
) -> Result<Option<(crate::groups::DeviceOp, Option<Hash>)>, StoreError> {
    let bodies = txn.open_table(BODIES)?;
    let Some(body) = bodies.get(event.id.as_bytes().as_slice())? else {
        return Ok(None);
    };
    let body = body.value().to_vec();
    body_device_op(event, Some(&body)).map_err(|_| {
        corrupt("a stored body no longer decodes as it did when it was stored".to_owned())
    })
}

/// Attaches the action of a group operation to a stored event that has none."""),
])

# ------------------------------------------------------------- waiting_rows.rs
patch("event_store/waiting_rows.rs", [
    ("use super::{StoreError, WAITING, WAITING_BY_DEP, be_u64, corrupt, hash_of, row_event, seq_num};",
     "use super::{\n    DEVICE_OPS, StoreError, WAITING, WAITING_BY_DEP, be_u64, corrupt, hash_of, row_event, seq_num,\n};"),
    # Item 2: an expiring held event takes its device-operation row with it.
    ("""        let expired = self.pool.expire(now);
        for (_author, event) in &expired {
            delete(self.txn, &event.id)?;
        }
        Ok(expired.len())""",
     """        let expired = self.pool.expire(now);
        if !expired.is_empty() {
            // A held event that expires takes its device-operation row with
            // it (issue #8160, item 2): the row exists so a released event
            // can fold its operation, and an expired event never folds.
            let mut device_ops = self.txn.open_table(DEVICE_OPS)?;
            for (_author, event) in &expired {
                device_ops.remove(event.id.as_bytes().as_slice())?;
            }
        }
        for (_author, event) in &expired {
            delete(self.txn, &event.id)?;
        }
        Ok(expired.len())"""),
])

# ------------------------------------------------------------------ mod.rs
patch("event_store/mod.rs", [
    # Item 4 report type, next to the table definition.
    ("""const DEVICE_OPS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("device_ops");""",
     """const DEVICE_OPS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("device_ops");

/// The outcome of [`EventStore::recover_device_ops`] (issue #8160, item 4): the
/// device-operation rows rebuilt from the bodies the store holds, and the stored
/// events that could not be rebuilt locally.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceOpsRecovery {
    /// Events whose device-operation row was rebuilt from their held body.
    pub recovered: Vec<Hash>,
    /// Stored events that commit to a body the store does not hold, or whose held
    /// body no longer decodes as a device operation: if such an event carried a
    /// device operation, only re-delivery of its body can restore the row.
    pub unrecoverable: Vec<Hash>,
}"""),
    # Recovery runs on open, with the additive tables.
    ("""        store.reload(now_ms)?;
        Ok(store)""",
     """        store.reload(now_ms)?;
        // A store written before device-operation rows existed gains them
        // here, rebuilt from the verified bodies it holds (issue #8160,
        // item 4). Idempotent: a store whose rows are complete scans clean.
        store.recover_device_ops()?;
        Ok(store)"""),
    # The recovery method itself, after stored_body.
    ("""    pub(crate) fn stored_body(&self, id: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        let txn = self.db.begin_read()?;
        let bodies = txn.open_table(BODIES)?;
        let stored = bodies.get(id.as_bytes().as_slice())?;
        Ok(stored.map(|body| body.value().to_vec()))
    }""",
     """    pub(crate) fn stored_body(&self, id: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        let txn = self.db.begin_read()?;
        let bodies = txn.open_table(BODIES)?;
        let stored = bodies.get(id.as_bytes().as_slice())?;
        Ok(stored.map(|body| body.value().to_vec()))
    }

    /// Rebuilds the device-operation rows of a store written before they
    /// existed (issue #8160, item 4). Every stored event whose verified body
    /// the store holds in `bodies` and that carries a device operation gains
    /// its row, in one transaction, so the resolution folds the operation
    /// again — an upgraded store's Admin-device grants and revocations stop
    /// being invisible to claims. The bodies are the ones the store itself
    /// verified against their headers when they were stored, so a rebuilt
    /// row is exactly the operation the signer published. A stored event
    /// whose header commits to a body the store does not hold cannot be
    /// rebuilt locally; it is named in the report's
    /// [`DeviceOpsRecovery::unrecoverable`] instead of being passed over
    /// silently. Runs on every open (see [`EventStore::open_with`]) and may
    /// be called directly; it is idempotent.
    ///
    /// # Errors
    ///
    /// Returns a redb or corruption error if the store cannot be read or
    /// written.
    pub fn recover_device_ops(&mut self) -> Result<DeviceOpsRecovery, StoreError> {
        let txn = self.db.begin_write()?;
        let mut recovered = Vec::new();
        let mut unrecoverable = Vec::new();
        {
            let events = txn.open_table(EVENTS)?;
            let bodies = txn.open_table(BODIES)?;
            let mut device_ops = txn.open_table(DEVICE_OPS)?;
            let mut stored = Vec::new();
            for entry in events.iter()? {
                let (_, value) = entry?;
                stored.push(decode_stored(value.value())?);
            }
            for event in stored {
                let id = event.id;
                if device_ops.get(id.as_bytes().as_slice())?.is_some() {
                    continue;
                }
                match bodies.get(id.as_bytes().as_slice())? {
                    Some(body) => {
                        match ingest::body_device_op(&event, Some(body.value())) {
                            Ok(Some((op, group))) => {
                                let encoded = resolve::encode_device_op(&op, group.as_ref())?;
                                device_ops
                                    .insert(id.as_bytes().as_slice(), encoded.as_slice())?;
                                recovered.push(id);
                            }
                            Ok(None) => {}
                            Err(_) => unrecoverable.push(id),
                        }
                    }
                    None => {
                        if event.header.payload_hash.is_some() {
                            unrecoverable.push(id);
                        }
                    }
                }
            }
        }
        txn.commit()?;
        Ok(DeviceOpsRecovery {
            recovered,
            unrecoverable,
        })
    }"""),
])
print("all patches applied")
