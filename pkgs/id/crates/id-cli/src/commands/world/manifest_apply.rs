//! `id world` manifest plan/apply helpers (#16255 slice 6): parse a
//! `world.toml` into a validated manifest, diff it against live host
//! state, and apply the reconciliation steps in reachability order.

use anyhow::{Context, Result};

use id_worlds::world_net::WorldClient;

/// Parse a `world.toml` file into a validated manifest.
pub(crate) fn parse_manifest_file(
    file: &std::path::Path,
) -> Result<id_caps::world_manifest::WorldManifest> {
    let text = std::fs::read_to_string(file)
        .with_context(|| format!("read world manifest {}", file.display()))?;
    id_caps::world_manifest::parse(&text)
}

/// Fetch the current world state the manifest is compared against: the active
/// module hash, the grants, the ceiling, the execution profile, the pending
/// schedules and the install seed. The host returns the seed only to a caller
/// presenting the admin token (#240), so a capability-only read (diff) plans
/// with seed 0 — a seed alone never plans a step, and apply always reads with
/// the admin token.
pub(crate) async fn current_world_state(
    client: &WorldClient,
    capability: &str,
    admin_token: Option<&str>,
) -> Result<id_caps::world_manifest::WorldState> {
    // One fresh session per read: admin and query frames are served one
    // operation per stream, so sequencing these reads on one stream would
    // talk to a session the host already finished.
    let info = client
        .fresh_session()
        .await?
        .info_details(capability)
        .await?;
    let caps: id_caps::world_caps::CapsReport =
        serde_json::from_value(client.fresh_session().await?.caps(capability).await?)
            .context("host returned a malformed capability report")?;
    let profile: id_caps::world_limits::WorldExecutionProfile = serde_json::from_value(
        client
            .fresh_session()
            .await?
            .execution_profile(None, Some(capability))
            .await?,
    )
    .context("host returned a malformed execution profile")?;
    // The presentation policy is public pre-join metadata, so it is read
    // with no token at all.
    let presentation: id_core::presentation_policy::PresentationPolicy =
        serde_json::from_value(client.fresh_session().await?.presentation_policy().await?)
            .context("host returned a malformed presentation policy")?;
    // The report carries the world's own ceiling as a list; the shared
    // fold turns a report equal to the default ceiling into None, which
    // means the default ceiling — the one meaning of None for a live
    // ceiling (#6817, #838).
    let ceiling = id_caps::world_manifest::fold_reported_ceiling(caps.ceiling.clone());
    let (seed, schedules) = client
        .fresh_session()
        .await?
        .manifest_state(admin_token, Some(capability))
        .await?;
    Ok(id_caps::world_manifest::WorldState {
        module_hash: info.active_module_hash,
        seed: seed.unwrap_or(0),
        grants: caps.granted,
        ceiling,
        profile,
        presentation,
        schedules,
    })
}

/// Whether [`apply_steps`] actually applies this step over the wire. Every
/// step kind is wired since #807 (module install and schedules included),
/// so every step counts toward the "applied N step(s)" summary (#818). The
/// predicate stays explicit per kind so a future reported-only step cannot
/// silently claim credit: list it here only when its apply path lands.
pub(crate) fn step_is_applied(step: &id_caps::world_manifest::ApplyStep) -> bool {
    use id_caps::world_manifest::ApplyStep;
    matches!(
        step,
        ApplyStep::InstallModule { .. }
            | ApplyStep::SetGrants { .. }
            | ApplyStep::SetCeiling { .. }
            | ApplyStep::SetProfile { .. }
            | ApplyStep::SetPresentation { .. }
            | ApplyStep::ScheduleAt { .. }
            | ApplyStep::CancelSchedule { .. }
    )
}

/// Apply every reconciliation step over the wire through the admin
/// operations: module install (compile a source, or download a known hash
/// and re-install its bytes), grants, ceiling, profile, and schedule
/// set/cancel — each journaled by the host. Returns how many of the steps
/// were actually applied, so the caller's summary cannot claim credit for
/// any step whose apply path is missing (#818).
///
/// The steps run in an order that makes the plan's result reachable, not
/// strictly in plan order (#807): schedule cancels first, so a manifest
/// that replaces ids at the pending-schedule capacity frees the slots its
/// additions need; the execution profile next, so a module installed in
/// the same apply is instantiated under the manifest's profile, not the
/// previous one (a profile applies on the next install); grants and
/// ceiling next, so a module compiled in the same apply may import the
/// capabilities the manifest grants; then the module install; schedule
/// additions last.
///
/// Each operation runs on a fresh session (see
/// [`WorldClient::fresh_session`]): the host serves admin frames one
/// operation per stream, so a second operation on the same stream would
/// write to a session that no longer has a receiver.
/// Order steps for reachability (see [`apply_steps`]): cancellations
/// first, then profile, grants and ceiling, then the presentation
/// policy (it depends on nothing else and nothing depends on it), then
/// the module install, and schedule additions last. The sort is stable, so steps of the
/// same kind keep their plan order.
pub(crate) fn ordered_steps<'a>(
    steps: &'a [id_caps::world_manifest::ApplyStep],
) -> Vec<&'a id_caps::world_manifest::ApplyStep> {
    use id_caps::world_manifest::ApplyStep;
    fn rank(step: &ApplyStep) -> u8 {
        match step {
            ApplyStep::CancelSchedule { .. } => 0,
            ApplyStep::SetProfile { .. } => 1,
            ApplyStep::SetGrants { .. } => 2,
            ApplyStep::SetCeiling { .. } => 3,
            ApplyStep::SetPresentation { .. } => 4,
            ApplyStep::InstallModule { .. } => 5,
            ApplyStep::ScheduleAt { .. } => 6,
        }
    }
    let mut ordered: Vec<&ApplyStep> = steps.iter().collect();
    ordered.sort_by_key(|step| rank(step));
    ordered
}

pub(crate) async fn apply_steps(
    client: &WorldClient,
    admin_token: &str,
    capability: &str,
    steps: &[id_caps::world_manifest::ApplyStep],
) -> Result<usize> {
    use id_caps::world_manifest::ApplyStep;
    let applied = steps.iter().filter(|step| step_is_applied(step)).count();
    let ordered = ordered_steps(steps);
    for step in ordered {
        match step {
            ApplyStep::SetGrants { grants } => {
                let current: id_caps::world_caps::CapsReport =
                    serde_json::from_value(client.fresh_session().await?.caps(capability).await?)
                        .context("host returned a malformed capability report")?;
                let to_revoke: Vec<String> = current
                    .granted
                    .iter()
                    .filter(|name| !grants.contains(name))
                    .cloned()
                    .collect();
                let to_grant: Vec<String> = grants
                    .iter()
                    .filter(|name| !current.granted.contains(name))
                    .cloned()
                    .collect();
                if !to_grant.is_empty() || !to_revoke.is_empty() {
                    client
                        .fresh_session()
                        .await?
                        .update_caps(admin_token, &to_grant, &to_revoke)
                        .await?;
                }
            }
            ApplyStep::SetCeiling { ceiling } => {
                // `None` means the default ceiling — the one meaning of
                // None for a live ceiling (#6817): restore the default
                // names on the wire; providers with an `Excluded` manifest
                // default stay out until a ceiling names them (#838).
                let names = ceiling.clone().unwrap_or_else(|| {
                    id_caps::world_caps::default_ceiling_names()
                        .into_iter()
                        .collect::<Vec<_>>()
                });
                client
                    .fresh_session()
                    .await?
                    .set_caps_ceiling(admin_token, &names)
                    .await?;
            }
            ApplyStep::SetProfile { profile } => {
                client
                    .fresh_session()
                    .await?
                    .set_execution_profile(admin_token, *profile)
                    .await?;
            }
            ApplyStep::SetPresentation { policy } => {
                client
                    .fresh_session()
                    .await?
                    .set_presentation_policy(admin_token, policy)
                    .await?;
            }
            ApplyStep::InstallModule { hash, source, seed } => {
                if let Some(source) = source {
                    let sources = read_compile_sources(source)?;
                    client
                        .fresh_session()
                        .await?
                        .compile(admin_token, sources, *seed, false)
                        .await?;
                } else if let Some(hash) = hash {
                    // The module is already on the host: fetch its bytes by
                    // hash and install them under the manifest's seed.
                    let wasm = client
                        .fresh_session()
                        .await?
                        .download_module_admin(admin_token, hash)
                        .await?;
                    client
                        .fresh_session()
                        .await?
                        .install_wasm(admin_token, &wasm, *seed)
                        .await?;
                } else {
                    anyhow::bail!("install_module step names neither a hash nor a source");
                }
            }
            ApplyStep::ScheduleAt { id, at_ms } => {
                client
                    .fresh_session()
                    .await?
                    .set_schedule(admin_token, id, *at_ms)
                    .await?;
            }
            ApplyStep::CancelSchedule { id } => {
                client
                    .fresh_session()
                    .await?
                    .cancel_schedule(admin_token, id)
                    .await?;
            }
        }
    }
    Ok(applied)
}

/// Read the Roc sources a manifest `module_source` names, in the shape the
/// host compile operation expects: the main file first (always sent as
/// `main.roc`), then any sibling `.roc` files when the source is a
/// directory. Size bounds mirror `id world compile`.
pub(crate) fn read_compile_sources(source: &std::path::Path) -> Result<Vec<(String, String)>> {
    let mut paths: Vec<(String, std::path::PathBuf)> = Vec::new();
    if source.is_dir() {
        paths.push(("main.roc".to_owned(), source.join("main.roc")));
        let mut siblings: Vec<std::path::PathBuf> = std::fs::read_dir(source)
            .with_context(|| format!("read source directory {}", source.display()))?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|ext| ext == "roc")
                    && path.file_name().is_some_and(|name| name != "main.roc")
            })
            .collect();
        siblings.sort();
        for path in siblings {
            let name = path
                .file_name()
                .context("a source file has no name")?
                .to_string_lossy()
                .into_owned();
            paths.push((name, path));
        }
    } else {
        paths.push(("main.roc".to_owned(), source.to_path_buf()));
    }
    let mut sources = Vec::new();
    let mut total = 0usize;
    for (name, path) in paths {
        let content =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        anyhow::ensure!(
            content.len() <= id_worlds::world_compile::MAX_COMPILE_FILE_BYTES,
            "{} exceeds {} bytes",
            path.display(),
            id_worlds::world_compile::MAX_COMPILE_FILE_BYTES
        );
        total += content.len();
        sources.push((name, content));
    }
    anyhow::ensure!(
        total <= id_worlds::world_compile::MAX_COMPILE_TOTAL_BYTES,
        "the sources exceed {} bytes",
        id_worlds::world_compile::MAX_COMPILE_TOTAL_BYTES
    );
    Ok(sources)
}
