//! The seam between the wire protocols and the world host (seam 9 of the
//! crate-split design, #4406): the host publishes whether it can take a
//! request; the transfer protocols in this crate consume that and nothing
//! else. The facade implements [`SessionHost`] for its world hub, running
//! each request as a directory action under the hub's lease; the errors it
//! returns keep the directory's `Refusal` in their chain, so the wire
//! layer's refused-vs-retry mapping is unchanged.

use std::future::Future;

use anyhow::Error;

use crate::artifact::{SignedArtifact, Stored};
use crate::envelope::Envelope;

/// Whether the host could take a request at all.
#[derive(Debug)]
pub enum Availability<T> {
    /// The host took the request and produced this outcome.
    Available(T),
    /// The host is busy (its world lease is contended); trying again later
    /// may succeed.
    Busy,
}

/// What receiving an envelope did, as the wire answer needs it. The
/// directory's own `Received` also carries the journal entry it appended,
/// which the wire never sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeReceived {
    /// The envelope changed the directory.
    Applied,
    /// The envelope was already applied, so nothing changed.
    Duplicate,
}

/// The directory operations the envelope and artifact protocols need from
/// the world host. `Ok(Available(None))` means the action ran but had
/// nothing to report (the envelope or artifact was not received/stored).
pub trait SessionHost: Clone + std::fmt::Debug + Send + Sync + 'static {
    /// Receive one friend envelope into the hosted world.
    fn receive_envelope(
        &self,
        envelope: Envelope,
    ) -> impl Future<Output = Result<Availability<Option<EnvelopeReceived>>, Error>> + Send;

    /// Store one pushed artifact in the hosted world.
    fn receive_artifact(
        &self,
        artifact: SignedArtifact,
    ) -> impl Future<Output = Result<Availability<Option<Stored>>, Error>> + Send;

    /// Read one page of `account`'s artifacts after sequence `after`.
    fn read_artifacts(
        &self,
        account: String,
        after: u64,
    ) -> impl Future<Output = Result<Availability<Vec<SignedArtifact>>, Error>> + Send;
}
