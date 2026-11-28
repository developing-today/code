//! The hub's side of the `id-sync` host seam (seam 9 of the crate
//! split, #4406): the world hub runs each wire request as a directory
//! action under its lease. The protocols live in `id_sync`; this is
//! their composition with the hub. (The other half of the seam, the
//! `ArtifactSink` impl for the directory, lives in `id-directory`
//! with the `Directory` type since step 6b, #4409.)

use anyhow::Result;
use id_sync::artifact::{SignedArtifact, Stored};
use id_sync::envelope::Envelope;
use id_sync::host::{Availability, EnvelopeReceived, SessionHost};

use crate::directory::Received;
use crate::directory_auth::Caller;
use crate::directory_view::DirectoryAction;
use crate::world_hub::WorldHub;

impl SessionHost for WorldHub {
    async fn receive_envelope(
        &self,
        envelope: Envelope,
    ) -> Result<Availability<Option<EnvelopeReceived>>> {
        let Ok(lease) = self.lease(None, None).await else {
            return Ok(Availability::Busy);
        };
        let outcome = lease
            .service()
            .directory(Caller::default(), DirectoryAction::Receive { envelope })
            .await?;
        Ok(Availability::Available(match outcome.received {
            Some(Received::Applied(_)) => Some(EnvelopeReceived::Applied),
            Some(Received::Duplicate) => Some(EnvelopeReceived::Duplicate),
            None => None,
        }))
    }

    async fn receive_artifact(
        &self,
        artifact: SignedArtifact,
    ) -> Result<Availability<Option<Stored>>> {
        let Ok(lease) = self.lease(None, None).await else {
            return Ok(Availability::Busy);
        };
        let outcome = lease
            .service()
            .directory(
                Caller::default(),
                DirectoryAction::ReceiveArtifact { artifact },
            )
            .await?;
        Ok(Availability::Available(outcome.stored))
    }

    async fn read_artifacts(
        &self,
        account: String,
        after: u64,
    ) -> Result<Availability<Vec<SignedArtifact>>> {
        let Ok(lease) = self.lease(None, None).await else {
            return Ok(Availability::Busy);
        };
        let outcome = lease
            .service()
            .directory(
                Caller::default(),
                DirectoryAction::ReadArtifacts { account, after },
            )
            .await?;
        Ok(Availability::Available(
            outcome.artifact_page.unwrap_or_default(),
        ))
    }
}
