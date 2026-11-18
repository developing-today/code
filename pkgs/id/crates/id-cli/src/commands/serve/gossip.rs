//! Group-warning watching, presence heartbeats, and the gossip
//! loop for the serve command.
//! Extracted from `serve.rs` for the production-line guard (#10456;
//! split tracked in #1438): moves only, no behaviour change.

use super::*;

/// How often serve re-evaluates the group warnings against the group log.
pub(super) const GROUP_WARNING_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// The serve-side group warning watcher (P4.9, #799).
///
/// Serve holds exactly one of these for the life of the process, so the
/// [`Evaluator`]'s high-water marks survive across evaluations: a group that
/// crosses a size threshold warns once, not on every pass. The groups in the
/// node's own group log are the ones its operator made through the CLI, so
/// every group is observed as client-made. Member counts come from the fold.
///
/// Group events carry no timestamps, so churn cannot be reconstructed from
/// the log for time before serve started. Instead the watcher counts re-adds
/// from the log events folded since its previous evaluation (#1847): a
/// device authorization that takes effect for a device the watcher has seen
/// authorized before, and that is not authorized at that point in the event
/// stream, is a re-add, stamped with the time of the evaluation that folds
/// it. Counting from the event stream — not from the net device set — means
/// a revoke + re-add cycle that completes inside one evaluation interval
/// still counts. The first evaluation only establishes the baseline (the
/// devices the fold already shows), so history from before serve started
/// never counts.
pub(super) struct GroupWarningWatcher {
    evaluator: Evaluator,
    /// The devices authorized per group at the last event the watcher has
    /// folded, tracked through the event stream, not just at evaluations,
    /// with the accounts whose grants authorize each device: a revocation
    /// is account-scoped in the fold (it ends only the revoking account's
    /// grant), so a device stays authorized while any account's grant for
    /// it remains. An empty account set means the fold shows the device
    /// authorized but the watcher has not observed a grant for it (the
    /// device was adopted in the fold sync in `evaluate`).
    authorized: std::collections::BTreeMap<
        Hash,
        std::collections::BTreeMap<GroupKey, std::collections::BTreeSet<GroupKey>>,
    >,
    /// Every device ever seen authorized per group.
    seen_devices: std::collections::BTreeMap<Hash, std::collections::BTreeSet<GroupKey>>,
    /// The re-add times per group, as observed by this watcher.
    readds: std::collections::BTreeMap<Hash, Vec<u64>>,
    /// How many log events the watcher has folded, or `None` before the
    /// first evaluation establishes the baseline.
    events_seen: Option<usize>,
}

impl GroupWarningWatcher {
    pub(super) fn new() -> Self {
        Self {
            evaluator: Evaluator::default(),
            authorized: std::collections::BTreeMap::new(),
            seen_devices: std::collections::BTreeMap::new(),
            readds: std::collections::BTreeMap::new(),
            events_seen: None,
        }
    }

    /// Evaluates every active group in `projection` at `now` (Unix seconds).
    /// Returns the warnings this evaluation adds, and writes each of them to
    /// the server log through the warnings module.
    pub(super) fn evaluate(
        &mut self,
        projection: &Projection,
        events: &[id_events::groups::Event],
        now: u64,
    ) -> Vec<(Hash, Warning)> {
        let delta_readds = self.fold_new_events(projection, events);
        let mut fired = Vec::new();
        for (group, view) in projection.groups() {
            if !view.is_active() {
                continue;
            }
            // Keep the event-tracked authorization in step with the fold:
            // a device can leave or rejoin the active set without a device
            // event of its own (its account was removed or re-added). Such
            // a change is not a re-add — only a device event counts — but
            // the tracked set must match the fold, or a later genuine
            // re-add would be missed or counted twice.
            let current = projection.group_active_devices(group);
            {
                // Refresh the authorizing accounts from the fold as well
                // as the device keys: an account can stop authorizing a
                // device without a device event of its own (its membership
                // was removed), and a stale account left in the tracked
                // set would keep the device authorized through every
                // later revoke by the remaining accounts, hiding genuine
                // churn (Codex review on PR #4035).
                let mut fold_accounts: std::collections::BTreeMap<
                    GroupKey,
                    std::collections::BTreeSet<GroupKey>,
                > = std::collections::BTreeMap::new();
                for account in view.members.keys() {
                    for device in projection.active_devices(group, account) {
                        fold_accounts.entry(device).or_default().insert(*account);
                    }
                }
                let authorized = self.authorized.entry(*group).or_default();
                authorized.retain(|device, _accounts| current.contains(device));
                let seen = self.seen_devices.entry(*group).or_default();
                for device in &current {
                    let is_new = !authorized.contains_key(device);
                    if let Some(accounts) = fold_accounts.get(device) {
                        authorized.insert(*device, accounts.clone());
                    } else if is_new {
                        authorized.insert(*device, std::collections::BTreeSet::new());
                    }
                    if is_new {
                        seen.insert(*device);
                    }
                }
            }
            let fresh_readds = delta_readds.get(group).copied().unwrap_or(0);
            if fresh_readds > 0 {
                let times = self.readds.entry(*group).or_default();
                for _ in 0..fresh_readds {
                    times.push(now);
                }
                times.retain(|at| {
                    now.saturating_sub(*at) < id_groups::groups::warnings::CHURN_WINDOW_SECS
                });
            }
            let readds = self.readds.get(group).cloned().unwrap_or_default();
            let observation = Observation {
                group: *group,
                client_made: true,
                members: view.members.len(),
                readds: &readds,
            };
            let warnings = self.evaluator.observe(&observation, now);
            if !warnings.is_empty() {
                id_groups::groups::warnings::log(group, &warnings);
                fired.extend(warnings.into_iter().map(|warning| (*group, warning)));
            }
        }
        fired
    }

    /// Folds the log events recorded since the previous evaluation and
    /// counts the re-adds among them, per group. Only events the fold
    /// applied count: a refused authorization authorizes nothing, and a
    /// refused revocation revokes nothing.
    ///
    /// The first call only establishes the baseline — it adopts the fold's
    /// current devices and counts nothing — because events carry no
    /// timestamps and churn from before serve started must not be
    /// reconstructed. A log that shrank since the previous evaluation (it
    /// was replaced) likewise re-baselines instead of counting.
    pub(super) fn fold_new_events(
        &mut self,
        projection: &Projection,
        events: &[id_events::groups::Event],
    ) -> std::collections::BTreeMap<Hash, usize> {
        let mut counts = std::collections::BTreeMap::new();
        let Some(cursor) = self.events_seen else {
            self.baseline(projection);
            self.events_seen = Some(events.len());
            return counts;
        };
        if cursor > events.len() {
            self.baseline(projection);
            self.events_seen = Some(events.len());
            return counts;
        }
        for event in &events[cursor..] {
            let Some(group) = event.group else {
                continue;
            };
            let id_events::groups::Body::Device(op) = &event.body else {
                continue;
            };
            if projection.outcome(&event.id) != Some(&id_events::groups::Outcome::Applied) {
                continue;
            }
            match op {
                id_events::groups::DeviceOp::Add { device, .. } => {
                    let seen = self.seen_devices.entry(group).or_default();
                    let was_seen = seen.contains(device);
                    seen.insert(*device);
                    let authorized = self.authorized.entry(group).or_default();
                    if !authorized.contains_key(device) && was_seen {
                        *counts.entry(group).or_default() += 1;
                    }
                    authorized.entry(*device).or_default().insert(event.author);
                }
                id_events::groups::DeviceOp::Revoke { device } => {
                    // A revocation is account-scoped in the fold: it ends
                    // only the revoking account's grant, so the device
                    // stays authorized while another account's grant for
                    // it remains (Codex review on PR #4035). Removing the
                    // device outright would count the other account's
                    // later re-add as churn that never happened.
                    if let Some(authorized) = self.authorized.get_mut(&group) {
                        let mut ended = false;
                        if let Some(accounts) = authorized.get_mut(device) {
                            ended = accounts.remove(&event.author) && accounts.is_empty();
                        }
                        if ended {
                            authorized.remove(device);
                        }
                    }
                }
            }
        }
        self.events_seen = Some(events.len());
        counts
    }

    /// Adopts the fold's current devices as the watcher's tracked state,
    /// counting nothing: the state before the baseline is history the
    /// watcher did not observe.
    pub(super) fn baseline(&mut self, projection: &Projection) {
        self.authorized.clear();
        self.seen_devices.clear();
        for (group, view) in projection.groups() {
            let current = projection.group_active_devices(group);
            self.seen_devices.insert(*group, current.clone());
            // Record which accounts' grants authorize each device, so a
            // later account-scoped revocation replays correctly against
            // the baseline too.
            let mut devices: std::collections::BTreeMap<
                GroupKey,
                std::collections::BTreeSet<GroupKey>,
            > = current
                .iter()
                .map(|device| (*device, std::collections::BTreeSet::new()))
                .collect();
            for account in view.members.keys() {
                for device in projection.active_devices(group, account) {
                    devices.entry(device).or_default().insert(*account);
                }
            }
            self.authorized.insert(*group, devices);
        }
    }
}

/// One warning evaluation pass over the group log at `path`.
///
/// A missing log means no groups, not an error. A log that cannot be read or
/// folded is noted in the server log and skipped: warnings never block serve.
pub(super) fn evaluate_group_warnings(
    watcher: &mut GroupWarningWatcher,
    path: &std::path::Path,
    now: u64,
) -> Vec<(Hash, Warning)> {
    if !path.exists() {
        return Vec::new();
    }
    let log = match GroupLog::open(path) {
        Ok(log) => log,
        Err(error) => {
            debug!("group warnings: cannot open {}: {error}", path.display());
            return Vec::new();
        }
    };
    let projection = match log.projection() {
        Ok(projection) => projection,
        Err(error) => {
            debug!("group warnings: cannot fold {}: {error}", path.display());
            return Vec::new();
        }
    };
    watcher.evaluate(&projection, log.events(), now)
}

/// The current time as Unix seconds, for the warning evaluator's clock.
pub(super) fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(super) const MAX_STALE_PROBES_PER_PASS: usize = 4;

/// Verifies an incoming presence heartbeat and stores only the channels whose
/// folded group membership contains the sender (#4363). Membership comes from
/// the receiver's own group fold for the channel (the `GroupView` member
/// set); a channel the receiver holds no fold for scopes to empty, because
/// membership the receiver cannot see cannot be confirmed. If the local group
/// log cannot be read or folded, the heartbeat is refused rather than
/// accepted without its scope check. Signature verification is unchanged: a
/// heartbeat whose signature by its claimed node does not verify is dropped.
pub(super) fn receive_presence_heartbeat(
    table: &PresenceTable,
    heartbeat: PresenceHeartbeat,
    groups_log: &Path,
) -> bool {
    if !heartbeat.verify() {
        debug!("presence heartbeat failed signature verification, dropped");
        return false;
    }

    let log = match GroupLog::open(groups_log) {
        Ok(log) => log,
        Err(error) => {
            debug!("presence heartbeat dropped: cannot open groups log: {error}");
            return false;
        }
    };
    let projection = match log.projection() {
        Ok(projection) => projection,
        Err(error) => {
            debug!("presence heartbeat dropped: cannot fold groups log: {error}");
            return false;
        }
    };

    let node = heartbeat.node;
    let channels = heartbeat.channels.len();
    let accepted = table.update_scoped(heartbeat, &|sender: &AuthorKey, group: &Hash| {
        projection
            .group(group)
            .is_some_and(|view| view.members.contains_key(sender))
    });
    if accepted {
        debug!(
            "presence heartbeat from {}: {channels} claimed channel(s)",
            node.iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
    }
    accepted
}

/// The presence-heartbeat period for the gossip loop: [`HEARTBEAT_INTERVAL`],
/// the cadence `chat_presence` documents, as the single source (#4777).
/// A function (not an inline constant at the call site) so a test can pin
/// the wiring and the constant cannot silently go dead again.
pub(super) fn presence_heartbeat_period() -> std::time::Duration {
    HEARTBEAT_INTERVAL
}

/// Background loop for gossip-based peer discovery.
///
/// Runs three concurrent tasks:
/// 1. **Announce**: Broadcasts a [`PeerAnnouncement`] every [`ANNOUNCE_INTERVAL`],
///    and a presence heartbeat every [`HEARTBEAT_INTERVAL`] on its own timer
/// 2. **Receive**: Listens for incoming gossip events and updates the peer table
/// 3. **Cleanup**: Periodically probes stale peers via `ListPeers` RPC — refreshes
///    their `last_seen` if reachable, removes them if not
///
/// This function runs until the sender/receiver are dropped (on shutdown).
pub(super) async fn run_gossip_loop(
    node_id: EndpointId,
    sender: distributed_topic_tracker::GossipSender,
    mut receiver: distributed_topic_tracker::GossipReceiver,
    peer_discovery: PeerDiscovery,
    chat_presence: PresenceTable,
    groups_log: PathBuf,
    signing_key: ed25519_dalek::SigningKey,
    store: iroh_blobs::api::Store,
    endpoint: Endpoint,
) {
    // Timeout for each individual peer probe.
    const PROBE_TIMEOUT: tokio::time::Duration = tokio::time::Duration::from_secs(10);

    let announce_store = store;
    let announce_node_id = node_id;

    // Spawn announce task: peer announcements and presence heartbeats run
    // on independent timers in this one task. They used to share the
    // announcement timer, so heartbeats actually went out every 30 s and
    // the documented 25 s HEARTBEAT_INTERVAL cadence (and the TTL
    // drop-resilience it promises) never happened (#4777).
    let announce_handle = tokio::spawn(async move {
        let mut announce_interval = tokio::time::interval(ANNOUNCE_INTERVAL);
        let mut heartbeat_interval = tokio::time::interval(presence_heartbeat_period());
        loop {
            tokio::select! {
                _ = announce_interval.tick() => {
                    // Get blob count from tags
                    let blob_count = match announce_store.tags().list().await {
                        Ok(mut stream) => {
                            let mut count = 0u64;
                            while stream.next().await.is_some() {
                                count += 1;
                            }
                            count
                        }
                        Err(_) => 0,
                    };

                    let announcement = PeerAnnouncement {
                        node_id: announce_node_id,
                        name: None,
                        blob_count,
                        timestamp_secs: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs()),
                    };

                    match SignedPeerAnnouncement::sign(announcement, &signing_key) {
                        Some(signed) => match signed.encode_frame() {
                            Ok(bytes) => {
                                if let Err(e) = sender.broadcast(bytes).await {
                                    debug!("gossip broadcast error: {}", e);
                                }
                            }
                            Err(e) => {
                                debug!("gossip announcement serialization error: {}", e);
                            }
                        },
                        None => {
                            warn!(
                                "could not sign peer announcement for endpoint {}",
                                announce_node_id
                            );
                        }
                    }

                }
                _ = heartbeat_interval.tick() => {
                    // Presence rides the same gossip plane (#2809): an ephemeral
                    // heartbeat naming this node. Channel membership lives in the
                    // chat layer above serve, so this heartbeat names no channels;
                    // receivers scope and read it through PresenceTable.
                    let mut presence = PresenceHeartbeat::new(
                        *announce_node_id.as_bytes(),
                        Vec::new(),
                        Vec::new(),
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs()),
                    );
                    presence.sign(&signing_key);
                    if let Err(e) = sender.broadcast(encode_frame(&presence)).await {
                        debug!("gossip presence broadcast error: {}", e);
                    }
                }
            }
        }
    });

    // Spawn receive task
    let recv_discovery = peer_discovery.clone();
    let recv_presence = chat_presence.clone();
    let recv_groups_log = groups_log;
    let recv_handle = tokio::spawn(async move {
        loop {
            match receiver.next().await {
                Ok(event) => match event {
                    iroh_gossip::api::Event::Received(msg) => {
                        if let Some(heartbeat) = decode_frame(&msg.content) {
                            // Presence is a claim about `node`, and the gossip
                            // topic is public and does not authenticate
                            // authors: accept the frame only when the
                            // heartbeat's signature by its claimed node
                            // verifies, so nobody can forge or suppress
                            // another node's presence (#2809 Codex review).
                            // The accepted heartbeat is then scoped to the
                            // sender's membership in the receiver's group
                            // fold (#4363).
                            receive_presence_heartbeat(&recv_presence, heartbeat, &recv_groups_log);
                        } else if let Some(announcement) =
                            SignedPeerAnnouncement::decode_frame(&msg.content)
                        {
                            debug!(
                                "peer announcement from {}: blob_count={}",
                                announcement.node_id, announcement.blob_count
                            );
                            recv_discovery.update(announcement);
                        } else {
                            debug!("unauthenticated or invalid peer announcement frame dropped");
                        }
                    }
                    iroh_gossip::api::Event::NeighborUp(peer) => {
                        info!("gossip neighbor up: {}", peer);
                    }
                    iroh_gossip::api::Event::NeighborDown(peer) => {
                        info!("gossip neighbor down: {}", peer);
                    }
                    iroh_gossip::api::Event::Lagged => {
                        warn!("gossip receiver lagged, some messages were missed");
                    }
                },
                Err(e) => {
                    debug!("gossip receiver stream ended: {}", e);
                    break;
                }
            }
        }
    });

    // Spawn stale cleanup task — probes stale peers before removal
    let cleanup_discovery = peer_discovery;
    let cleanup_presence = chat_presence;

    let cleanup_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(STALE_CHECK_INTERVAL);
        let mut probe_cursor: Option<EndpointId> = None;

        loop {
            interval.tick().await;

            // Presence entries expire on their own stamps; prune them here so a
            // long-running server does not retain every node key it has ever
            // seen (#2809 Codex review). This runs even when no discovery peer
            // is stale.
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let pruned = cleanup_presence.prune_at(now_secs);
            if pruned > 0 {
                debug!("pruned {} expired presence entrie(s)", pruned);
            }

            let stale = cleanup_discovery.stale_peers();
            if stale.is_empty() {
                continue;
            }

            let batch = select_stale_probe_batch(&stale, probe_cursor, MAX_STALE_PROBES_PER_PASS);
            if let Some(last) = batch.last() {
                probe_cursor = Some(last.announcement.node_id);
            }
            debug!("probing {} of {} stale peer(s)", batch.len(), stale.len());

            for info in &batch {
                let peer_id = info.announcement.node_id;

                // Try to connect and send ListPeers RPC
                let probe_result = tokio::time::timeout(PROBE_TIMEOUT, async {
                    let conn =
                        id_core::connectivity::connect(&endpoint, peer_id, META_ALPN).await?;
                    let (mut send, mut recv) = conn.open_bi().await?;
                    let req = postcard::to_allocvec(&MetaRequest::ListPeers)?;
                    send.write_all(&req).await?;
                    send.finish()?;
                    let resp_buf = crate::meta_client::read_capped_response(&mut recv).await?;
                    let _resp: MetaResponse = crate::protocol::decode_meta_exact(&resp_buf)?;
                    conn.close(0u32.into(), b"probe");
                    anyhow::Ok(())
                })
                .await;

                match probe_result {
                    Ok(Ok(())) => {
                        debug!("stale peer {} is still alive, refreshing", peer_id);
                        cleanup_discovery.refresh(&peer_id);
                    }
                    Ok(Err(e)) => {
                        debug!("stale peer {} probe failed: {}, removing", peer_id, e);
                        cleanup_discovery.remove_if_stale(&peer_id, STALE_THRESHOLD);
                    }
                    Err(_) => {
                        debug!("stale peer {} probe timed out, removing", peer_id);
                        cleanup_discovery.remove_if_stale(&peer_id, STALE_THRESHOLD);
                    }
                }
            }
        }
    });

    // Wait for any task to complete (normally they run until shutdown)
    tokio::select! {
        _ = announce_handle => {}
        _ = recv_handle => {}
        _ = cleanup_handle => {}
    }
}

/// Select a deterministic, bounded round-robin batch from stale peers.
pub(super) fn select_stale_probe_batch(
    stale: &[id_core::discovery::PeerInfo],
    after: Option<EndpointId>,
    limit: usize,
) -> Vec<id_core::discovery::PeerInfo> {
    if stale.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut ordered = stale.to_vec();
    ordered.sort_by(|left, right| {
        left.announcement
            .node_id
            .as_bytes()
            .cmp(right.announcement.node_id.as_bytes())
    });
    let start = after.map_or(0, |cursor| {
        let next = ordered
            .partition_point(|info| info.announcement.node_id.as_bytes() <= cursor.as_bytes());
        if next == ordered.len() { 0 } else { next }
    });
    (0..limit.min(ordered.len()))
        .map(|offset| ordered[(start + offset) % ordered.len()].clone())
        .collect()
}
