//! Started-server config loading, provisioning, and blob-gate
//! setup for the serve command.
//! Extracted from `serve.rs` for the production-line guard (#10456;
//! split tracked in #1438): moves only, no behaviour change.

use super::*;

use anyhow::Context as _;

#[allow(unused_variables)] // web/port only used with web feature
/// The loaded operator server config and the provisioning it ran
/// (#5119 slice 1). Kept at startup scope so the later slices — the
/// pull gate from `config.pull_policy()` and the freshness window in
/// seitan admission — wire into live serving from this one load.
#[cfg(feature = "world")]
#[derive(Debug)]
pub struct StartedServerConfig {
    /// The parsed config.
    pub config: crate::server_config::ServerConfig,
    /// The accounts and group events provisioning applied.
    pub provisioned: crate::server_config::Provisioned,
    /// What the provisioning run created.
    pub report: crate::server_config::Report,
}

/// Load the operator server config at `path` and run startup
/// provisioning from it (#5119 slice 1).
///
/// # Errors
///
/// Fails when the file cannot be read or parses badly — the
/// [`crate::server_config::ServerConfig::parse`] error is named, per
/// that parser's startup-failing contract — or when provisioning is
/// refused (for example a group creator the create rule does not
/// allow).
#[cfg(feature = "world")]
pub fn start_server_config(path: &Path) -> Result<StartedServerConfig> {
    let config = crate::server_config::ServerConfig::load(path)
        .map_err(|error| anyhow::anyhow!("server config {}: {error}", path.display()))?;
    let mut provisioned = crate::server_config::Provisioned::new();
    let report = provisioned
        .apply(&config)
        .map_err(|error| anyhow::anyhow!("server config {}: {error}", path.display()))?;
    Ok(StartedServerConfig {
        config,
        provisioned,
        report,
    })
}

/// The directory journal the server config provisioning persists
/// its accounts in (#5119 slice 3): `directory.jsonl` beside the
/// groups log, the same layout a world directory uses.
#[cfg(feature = "world")]
pub(super) fn server_config_directory_path(groups_log: &Path) -> PathBuf {
    groups_log.with_file_name("directory.jsonl")
}

/// Load the operator server config at `path` and run startup
/// provisioning into the live serving state (#5119 slice 3): the
/// provisioned accounts go into the journaled directory beside the
/// groups log and the preformed group events into the groups log
/// itself — the same state live serving folds and replays — so a
/// restart loads them back, provisioning skips them, and the report
/// names no new creations.
///
/// # Errors
///
/// Fails on the same startup contract as [`start_server_config`],
/// and when the persisted directory or groups log cannot be opened
/// or written.
#[cfg(feature = "world")]
pub fn start_server_config_persisted(
    path: &Path,
    groups_log: &Path,
) -> Result<StartedServerConfig> {
    let config = crate::server_config::ServerConfig::load(path)
        .map_err(|error| anyhow::anyhow!("server config {}: {error}", path.display()))?;
    let directory =
        id_directory::directory::Directory::open(&server_config_directory_path(groups_log))?;
    let mut log = id_groups::groups::log::GroupLog::open(groups_log)?;
    let already = log.events().len();
    let mut provisioned =
        crate::server_config::Provisioned::from_parts(directory, log.events().to_vec());
    let report = provisioned
        .apply(&config)
        .map_err(|error| anyhow::anyhow!("server config {}: {error}", path.display()))?;
    // Seed only the events this start added: the rest are the
    // from_parts state itself, and re-checking them on every
    // restart would rescan the whole log history.
    for event in &provisioned.events()[already..] {
        log.record_preformed(event)?;
    }
    Ok(StartedServerConfig {
        config,
        provisioned,
        report,
    })
}

/// Seed the serving world's own directory with the accounts the
/// server config provisions (#5119, PR #8356 review): slice 3
/// persists provisioning into the directory beside the groups log,
/// but a durable world loads `.id-worlds/<name>/directory.jsonl`
/// ([`id_worlds::world_store::open_world`]), so without this seed the
/// live world's directory view treats configured keys as anonymous
/// while startup reports them provisioned. The caller names the
/// default world, the one serve opens at startup. Seeding is the
/// same idempotent account application provisioning itself runs,
/// so a restart creates nothing. Group events are not seeded here:
/// they live in the groups log, which live serving already folds.
///
/// # Errors
///
/// Fails when the world directory cannot be created or opened, or
/// when the directory refuses a configured user -- the same
/// startup-failing contract as the provisioning it mirrors.
#[cfg(feature = "world")]
pub fn seed_world_directory(
    config: &crate::server_config::ServerConfig,
    dir: &Path,
) -> Result<crate::server_config::Report> {
    std::fs::create_dir_all(dir)?;
    let directory = id_directory::directory::Directory::open(&dir.join("directory.jsonl"))?;
    let mut provisioned = crate::server_config::Provisioned::from_parts(directory, Vec::new());
    provisioned.apply_users(config).map_err(|error| {
        anyhow::anyhow!("server config world directory {}: {error}", dir.display())
    })
}

/// The pull policy the live event-sync gate serves under (#5119): the server
/// config's `[policy] pull_policy` when a config was loaded, else the code
/// default ([`id_events::pull_policy::PullPolicy::Open`]), so a `serve` without
/// `--server-config` behaves exactly as before.
#[cfg(feature = "world")]
pub(super) fn serve_pull_policy(
    started: Option<&StartedServerConfig>,
) -> id_events::pull_policy::PullPolicy {
    started.map_or(id_events::pull_policy::PullPolicy::Open, |started| {
        started.config.pull_policy()
    })
}

/// The freshness window seitan admission serves under (#8487): the loaded
/// server config's `[policy] freshness_window_seconds`, resolved through
/// [`crate::server_config::ServerConfig::freshness`], when a config was
/// loaded, else [`id_directory::seitan::DEFAULT_WINDOW`] — so a `serve` without
/// `--server-config` behaves exactly as before.
#[cfg(feature = "world")]
pub(super) fn serve_freshness_window(started: Option<&StartedServerConfig>) -> std::time::Duration {
    started.map_or(id_directory::seitan::DEFAULT_WINDOW, |started| {
        started.config.freshness().window()
    })
}

/// The world protocol serve registers for a hub: the loaded operator
/// config's #2247 budget policy when a server config was given, the
/// coded defaults when none was. Before this wire, the budgets
/// table was parsed and held at startup scope but never reached the
/// protocol, so configured per-counterparty caps were inert in
/// production.
#[cfg(feature = "world")]
pub(super) fn world_protocol_for(
    hub: id_worlds::world_hub::WorldHub,
    local_node: [u8; 32],
    started: Option<&StartedServerConfig>,
) -> id_worlds::world_net::WorldProtocol {
    match started {
        Some(started) => id_worlds::world_net::WorldProtocol::with_budgets(
            hub,
            local_node,
            started.config.budgets().clone(),
        ),
        None => id_worlds::world_net::WorldProtocol::with_local_node(hub, local_node),
    }
}

/// The largest blob the gate's HashSeq expansion will read into memory: a
/// `HashSeq` of this size names 32768 children, far beyond any collection
/// this node serves; a larger blob is treated as plain content, never a
/// collection, so building the reference index stays size-bounded.
#[cfg(feature = "web")]
pub(super) const MAX_HASHSEQ_BYTES: u64 = 1 << 20;

/// Builds the blob gate's folded worlds from the two sources serve has: the group
/// log at `groups_log` for membership, and the event store's document references
/// for which blobs each world names (#173).
///
/// A referenced blob that parses as a `HashSeq` contributes its children to the
/// same world, but only when every child is present in the store — arbitrary
/// content whose length is a multiple of 32 bytes would otherwise spray
/// references onto hashes nobody stored.
///
/// # Errors
///
/// Fails when any source fails to read — the group log cannot be opened or
/// does not fold, or the document references cannot be read. A failed build
/// produces no fold at all, so a partial fold built from fallback values is
/// never mistaken for a complete one: the caller decides what a failure
/// means (startup falls back to an empty gate; the refresh loop keeps the
/// previous fold, #13821).
#[cfg(feature = "web")]
pub(super) async fn build_blob_gate(
    events: &Arc<tokio::sync::Mutex<crate::local_events::LocalEvents>>,
    store: &iroh_blobs::api::Store,
    groups_log: &std::path::Path,
) -> anyhow::Result<crate::blob_gate::FoldedWorlds> {
    let log = GroupLog::open(groups_log).context("blob gate: cannot open group log")?;
    let group_projection = crate::blob_gate::fold_group_events(log.events())
        .context("blob gate: group log does not fold")?;
    let references = {
        let events = events.lock().await;
        events
            .document_references()
            .context("blob gate: cannot read document references")?
    };
    // Expand HashSeq collections whose children are all in the store. The
    // expansion is bounded (#7181 review): only a group-referenced blob can
    // contribute children to the index — an ungrouped reference is discarded
    // below, so reading it here would be pure waste — and a blob is read into
    // memory only when its stored size, asked from the store status rather
    // than the content, is a positive multiple of 32 no larger than
    // MAX_HASHSEQ_BYTES. A HashSeq is a list of 32-byte hashes; anything else
    // cannot be one, and an oversized one is not worth a startup-time read
    // proportional to its full contents.
    let mut children: std::collections::HashMap<iroh_blobs::Hash, Vec<iroh_blobs::Hash>> =
        std::collections::HashMap::new();
    let mut seen: std::collections::BTreeSet<iroh_blobs::Hash> = std::collections::BTreeSet::new();
    let mut frontier: Vec<iroh_blobs::Hash> = references
        .iter()
        .filter(|(_, group)| group.is_some())
        .map(|(hash, _)| *hash)
        .collect();
    while let Some(hash) = frontier.pop() {
        if !seen.insert(hash) {
            continue;
        }
        let Ok(iroh_blobs::api::blobs::BlobStatus::Complete { size }) = store.status(hash).await
        else {
            continue;
        };
        if size == 0 || size > MAX_HASHSEQ_BYTES || !size.is_multiple_of(32) {
            continue;
        }
        let Ok(bytes) = store.get_bytes(hash).await else {
            continue;
        };
        let kids = crate::blob_gate::hashseq_children(&bytes);
        if kids.is_empty() {
            continue;
        }
        let mut all_present = true;
        for kid in &kids {
            match store.has(*kid).await {
                Ok(true) => {}
                _ => {
                    all_present = false;
                    break;
                }
            }
        }
        if all_present {
            frontier.extend(kids.iter().copied());
            children.insert(hash, kids);
        }
    }
    let named = references
        .into_iter()
        .filter_map(|(hash, group)| group.map(|group| (hash, crate::blob_gate::world_key(&group))));
    let index = crate::blob_gate::build_reference_index(named, |hash| {
        children.get(hash).cloned().unwrap_or_default()
    });
    Ok(crate::blob_gate::FoldedWorlds::from_group_projection(
        group_projection,
        index,
    ))
}
