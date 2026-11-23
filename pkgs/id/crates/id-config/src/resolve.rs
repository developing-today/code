//! Layered mode and scoped behavior resolution (slice 9111-02, #10193).
//!
//! The resolver keeps configuration planes distinct while applying their
//! documented precedence: deployment defaults, the selected mode bundle,
//! tenant, peer, link, storage, and finally operation-scoped behavior. Lower
//! scopes may replace behavior in either direction, but authority and
//! resource ceilings remain hard boundaries.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::{Behavior, CeilingOverride, Ceilings, Mode, ResolvedBehavior, SyncScope};

/// A behavior and optional resource narrowing supplied by one named scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerOverride {
    /// Stable scope name used in provenance and diagnostics.
    pub name: String,
    /// Behavioral values supplied by this scope.
    pub behavior: Behavior,
    /// Optional narrowing of the current resource ceilings.
    pub ceilings: CeilingOverride,
}

/// The ordered inputs to [`resolve`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionInput {
    /// Deployment-wide behavioral defaults.
    pub deployment: Behavior,
    /// Named mode bundle applied after deployment defaults.
    pub mode: Mode,
    /// Tenant-scoped behavior, if selected.
    pub tenant: Option<LayerOverride>,
    /// Peer-scoped behavior, if selected.
    pub peer: Option<LayerOverride>,
    /// Link-scoped behavior, if selected.
    pub link: Option<LayerOverride>,
    /// Storage-scoped behavior, if selected.
    pub storage: Option<LayerOverride>,
    /// Operation-scoped behavior, if permitted.
    pub operation: Option<LayerOverride>,
    /// Hard resource ceiling from the validated configuration document.
    pub resource_ceiling: Ceilings,
    /// Sync scopes authorized by the owning tenant/authority plane.
    pub authority_sync_scopes: Vec<SyncScope>,
}

/// The deterministic result of layered resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLayers {
    /// Fully resolved behavior.
    pub behavior: ResolvedBehavior,
    /// Effective resource ceilings after scope narrowings.
    pub ceilings: Ceilings,
    /// Source of each resolved behavior field.
    pub provenance: BTreeMap<String, String>,
}

/// Why layered resolution failed.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResolveError {
    /// A scope name was empty.
    #[error("scope name must not be empty")]
    EmptyScope,
    /// A scope attempted to widen a currently effective ceiling.
    #[error("scope {scope} widens ceiling {field} to {attempted}, above current ceiling {ceiling}")]
    CeilingWidened {
        /// Scope that attempted the widening.
        scope: String,
        /// Ceiling field.
        field: &'static str,
        /// Requested value.
        attempted: u64,
        /// Current effective ceiling.
        ceiling: u64,
    },
    /// A behavior value exceeds the effective resource ceiling.
    #[error("scope {scope} sets {field} to {value}, above ceiling {ceiling}")]
    ResourceExceeded {
        /// Scope supplying the value.
        scope: String,
        /// Behavior field.
        field: &'static str,
        /// Requested value.
        value: u64,
        /// Effective ceiling.
        ceiling: u64,
    },
    /// The final behavior is outside the owning authority plane.
    #[error("scope {scope} sets {field}={value:?} outside the authority ceiling")]
    AuthorityExceeded {
        /// Last scope that supplied the value.
        scope: String,
        /// Behavior field.
        field: &'static str,
        /// Disallowed scope.
        value: SyncScope,
    },
    /// No authority scopes were supplied.
    #[error("authority must allow at least one sync scope")]
    EmptyAuthority,
}

/// Resolve layered behavior in the fixed precedence order.
///
/// The mode bundle is applied after deployment defaults and before the named
/// scopes. Every scope is applied in the fixed plane order, never by caller
/// insertion order. Authority is checked on the final result because a mode
/// default may be replaced by a later authorized scope; resource ceilings are
/// checked at every layer so an over-ceiling value cannot be hidden by a later
/// narrowing override.
pub fn resolve(input: &ResolutionInput) -> Result<ResolvedLayers, ResolveError> {
    if input.authority_sync_scopes.is_empty() {
        return Err(ResolveError::EmptyAuthority);
    }

    let mut state = State {
        behavior: Mode::SingleNode.defaults(),
        ceilings: input.resource_ceiling,
        provenance: BTreeMap::new(),
    };
    apply_behavior(&mut state, &input.deployment, "deployment")?;
    apply_mode(&mut state, input.mode)?;
    for layer in [
        input.tenant.as_ref(),
        input.peer.as_ref(),
        input.link.as_ref(),
        input.storage.as_ref(),
        input.operation.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        apply_layer(&mut state, layer)?;
    }

    if !input
        .authority_sync_scopes
        .contains(&state.behavior.sync_scope)
    {
        let scope = state
            .provenance
            .get("sync_scope")
            .cloned()
            .unwrap_or_else(|| "unknown".to_owned());
        return Err(ResolveError::AuthorityExceeded {
            scope,
            field: "sync_scope",
            value: state.behavior.sync_scope,
        });
    }

    Ok(ResolvedLayers {
        behavior: state.behavior,
        ceilings: state.ceilings,
        provenance: state.provenance,
    })
}

struct State {
    behavior: ResolvedBehavior,
    ceilings: Ceilings,
    provenance: BTreeMap<String, String>,
}

fn apply_mode(state: &mut State, mode: Mode) -> Result<(), ResolveError> {
    let source = format!("mode:{}", mode.as_str());
    let defaults = mode.defaults();
    state.behavior = defaults;
    for field in ["max_hops", "sync_scope", "tier"] {
        state.provenance.insert(field.to_owned(), source.clone());
    }
    ensure_resource(&state.behavior, &state.ceilings, source)
}

fn apply_layer(state: &mut State, layer: &LayerOverride) -> Result<(), ResolveError> {
    if layer.name.trim().is_empty() {
        return Err(ResolveError::EmptyScope);
    }
    narrow_ceilings(&mut state.ceilings, &layer.ceilings, &layer.name)?;
    apply_behavior(state, &layer.behavior, &format!("override:{}", layer.name))
}

fn apply_behavior(
    state: &mut State,
    behavior: &Behavior,
    source: &str,
) -> Result<(), ResolveError> {
    if let Some(max_hops) = behavior.max_hops {
        state.behavior.max_hops = max_hops;
        state
            .provenance
            .insert("max_hops".to_owned(), source.to_owned());
    }
    if let Some(sync_scope) = behavior.sync_scope {
        state.behavior.sync_scope = sync_scope;
        state
            .provenance
            .insert("sync_scope".to_owned(), source.to_owned());
    }
    if let Some(tier) = behavior.tier {
        state.behavior.tier = tier;
        state
            .provenance
            .insert("tier".to_owned(), source.to_owned());
    }
    ensure_resource(&state.behavior, &state.ceilings, source.to_owned())
}

fn ensure_resource(
    behavior: &ResolvedBehavior,
    ceilings: &Ceilings,
    scope: String,
) -> Result<(), ResolveError> {
    if behavior.max_hops > ceilings.max_hops {
        return Err(ResolveError::ResourceExceeded {
            scope,
            field: "max_hops",
            value: u64::from(behavior.max_hops),
            ceiling: u64::from(ceilings.max_hops),
        });
    }
    Ok(())
}

fn narrow_ceilings(
    current: &mut Ceilings,
    requested: &CeilingOverride,
    scope: &str,
) -> Result<(), ResolveError> {
    check_narrowing(
        scope,
        "max_hops",
        requested.max_hops.map(u64::from),
        u64::from(current.max_hops),
    )?;
    check_narrowing(scope, "max_bytes", requested.max_bytes, current.max_bytes)?;
    check_narrowing(
        scope,
        "max_time_ms",
        requested.max_time_ms,
        current.max_time_ms,
    )?;
    check_narrowing(
        scope,
        "max_fan_out",
        requested.max_fan_out.map(u64::from),
        u64::from(current.max_fan_out),
    )?;
    *current = current.narrowed_by(requested);
    Ok(())
}

fn check_narrowing(
    scope: &str,
    field: &'static str,
    requested: Option<u64>,
    ceiling: u64,
) -> Result<(), ResolveError> {
    if let Some(attempted) = requested {
        if attempted > ceiling {
            return Err(ResolveError::CeilingWidened {
                scope: scope.to_owned(),
                field,
                attempted,
                ceiling,
            });
        }
    }
    Ok(())
}
