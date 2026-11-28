//! The client side of the append-list user-attribute protocol (#10983;
//! design #9127 §3, `doc/2026-11-17_design_id9127_append_list_user_attributes.md`).
//!
//! A client builds a [`SubmissionRequest`] once (hash the object, sign
//! the hash) and sends that same request until the server answers. The
//! server's submission is idempotent per (attribute, hash, submitter), so
//! a retry after a lost reply, including one where the server applied
//! the submission and only the answer was lost, returns the existing
//! sequence or pending id and never makes a second entry.
//!
//! [`AppendListService`] is the seam between this path and a transport.
//! [`AppendListServer`] implements it in process; a networked transport
//! implements it with its own error type, and the client retries only
//! those transport errors, never a [`Refusal`].

use std::convert::Infallible;
use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

use crate::append_list::{
    AppendListServer, ContentHash, Refusal, SubmissionId, SubmissionState, SubmitOutcome,
};

/// One `append-submit` request: the object, its hash, and the
/// submitter's signature over the hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionRequest {
    /// The attribute path, such as `geordi.png/comments`.
    pub attribute: String,
    /// The entry object's bytes.
    pub object: Vec<u8>,
    /// The BLAKE3 hash of `object`.
    pub hash: ContentHash,
    /// The submitter's key.
    pub submitter: VerifyingKey,
    /// The submitter's signature over `hash`.
    pub signature: Signature,
}

impl SubmissionRequest {
    /// Hashes `object` and signs the hash with `key`.
    #[must_use]
    pub fn sign(attribute: &str, object: Vec<u8>, key: &SigningKey) -> Self {
        let hash = ContentHash::of(&object);
        Self {
            attribute: attribute.to_owned(),
            signature: key.sign(&hash.0),
            hash,
            object,
            submitter: key.verifying_key(),
        }
    }
}

/// The server side of the protocol as a client reaches it. The outer
/// `Result` is the transport; the inner one is the server's answer.
pub trait AppendListService {
    /// A transport failure, after which the request may or may not have
    /// reached the server.
    type Error;

    /// Sends `append-submit`.
    ///
    /// # Errors
    ///
    /// A transport error when no answer came back.
    fn append_submit(
        &mut self,
        request: &SubmissionRequest,
    ) -> Result<Result<SubmitOutcome, Refusal>, Self::Error>;

    /// Reads the submitter's own state for one object.
    ///
    /// # Errors
    ///
    /// A transport error when no answer came back.
    fn submission_state(
        &mut self,
        attribute: &str,
        submitter: &VerifyingKey,
        hash: &ContentHash,
    ) -> Result<Result<Option<SubmissionState>, Refusal>, Self::Error>;
}

impl AppendListService for AppendListServer {
    type Error = Infallible;

    fn append_submit(
        &mut self,
        request: &SubmissionRequest,
    ) -> Result<Result<SubmitOutcome, Refusal>, Infallible> {
        Ok(Self::submit(
            self,
            &request.attribute,
            &request.object,
            &request.hash,
            &request.submitter,
            &request.signature,
        ))
    }

    fn submission_state(
        &mut self,
        attribute: &str,
        submitter: &VerifyingKey,
        hash: &ContentHash,
    ) -> Result<Result<Option<SubmissionState>, Refusal>, Infallible> {
        Ok(Self::own_state(self, attribute, submitter, hash))
    }
}

/// Where a submission landed, whether on the first attempt or a retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landed {
    /// In the list at this sequence number.
    Appended(u64),
    /// In the pending queue, waiting for an approver.
    Pending(SubmissionId),
}

impl From<SubmitOutcome> for Landed {
    fn from(outcome: SubmitOutcome) -> Self {
        match outcome {
            SubmitOutcome::Appended(seq) | SubmitOutcome::AlreadyAppended(seq) => {
                Self::Appended(seq)
            }
            SubmitOutcome::Pending(id) | SubmitOutcome::AlreadyPending(id) => Self::Pending(id),
        }
    }
}

/// Why a client op did not complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientError<E> {
    /// The server refused it. Not retried: the same request would be
    /// refused again.
    Refused(Refusal),
    /// Every attempt failed in transport; this is the last error. The
    /// submission may have landed, and resending the same request is safe.
    Transport(E),
}

impl<E: fmt::Display> fmt::Display for ClientError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => write!(f, "refused: {refusal}"),
            Self::Transport(err) => write!(f, "transport failed: {err}"),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for ClientError<E> {}

/// A submitter's client for append-list attributes.
#[derive(Debug)]
pub struct AppendListClient<'k> {
    key: &'k SigningKey,
    attempts: usize,
}

impl<'k> AppendListClient<'k> {
    /// A client that signs with `key` and tries each op up to three times.
    #[must_use]
    pub const fn new(key: &'k SigningKey) -> Self {
        Self { key, attempts: 3 }
    }

    /// Sets how many times an op is tried before the last transport
    /// error is returned (at least once).
    #[must_use]
    pub const fn with_attempts(mut self, attempts: usize) -> Self {
        self.attempts = if attempts == 0 { 1 } else { attempts };
        self
    }

    /// Builds the request for `object` on `attribute`.
    #[must_use]
    pub fn request(&self, attribute: &str, object: Vec<u8>) -> SubmissionRequest {
        SubmissionRequest::sign(attribute, object, self.key)
    }

    /// Submits `object` and returns where it landed. The request is built
    /// once and resent unchanged on a transport error, so a retry never
    /// makes a second entry.
    ///
    /// # Errors
    ///
    /// [`ClientError::Refused`] at once on a refusal;
    /// [`ClientError::Transport`] when every attempt failed in transport.
    pub fn submit<S: AppendListService>(
        &self,
        service: &mut S,
        attribute: &str,
        object: Vec<u8>,
    ) -> Result<Landed, ClientError<S::Error>> {
        self.resubmit(service, &self.request(attribute, object))
    }

    /// Sends an existing request again, as after a failure the caller
    /// saw. Same retry rules as [`Self::submit`].
    ///
    /// # Errors
    ///
    /// As [`Self::submit`].
    pub fn resubmit<S: AppendListService>(
        &self,
        service: &mut S,
        request: &SubmissionRequest,
    ) -> Result<Landed, ClientError<S::Error>> {
        self.retry(|| service.append_submit(request))
            .map(Landed::from)
    }

    /// This client's own state for `object` on `attribute`: pending,
    /// appended at a sequence, refused by an approver, or `None` when it
    /// never reached the server.
    ///
    /// # Errors
    ///
    /// As [`Self::submit`].
    pub fn state<S: AppendListService>(
        &self,
        service: &mut S,
        attribute: &str,
        object: &[u8],
    ) -> Result<Option<SubmissionState>, ClientError<S::Error>> {
        let hash = ContentHash::of(object);
        let submitter = self.key.verifying_key();
        self.retry(|| service.submission_state(attribute, &submitter, &hash))
    }

    fn retry<T, E>(
        &self,
        mut op: impl FnMut() -> Result<Result<T, Refusal>, E>,
    ) -> Result<T, ClientError<E>> {
        let mut attempt = 1;
        loop {
            match op() {
                Ok(answer) => return answer.map_err(ClientError::Refused),
                Err(err) if attempt >= self.attempts => return Err(ClientError::Transport(err)),
                Err(_err) => attempt += 1,
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::append_list::AttributeConfig;

    const ATTR: &str = "geordi.png/comments";

    fn key(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    /// Owner 1, member 2, attribute opened.
    fn server() -> AppendListServer {
        let mut server = AppendListServer::new();
        server
            .create_append_list(
                ATTR,
                key(1).verifying_key(),
                &AttributeConfig {
                    members: vec![key(2).verifying_key()],
                    ..AttributeConfig::default()
                },
            )
            .unwrap();
        server.open(ATTR, &key(1).verifying_key()).unwrap();
        server
    }

    /// A transport that loses replies. `before` drops the next N requests
    /// before they reach the server; `after` delivers the next N and then
    /// loses the answer, as a connection that dies mid-reply.
    struct Flaky {
        server: AppendListServer,
        before: usize,
        after: usize,
        delivered: usize,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Lost {
        Request,
        Reply,
    }

    impl fmt::Display for Lost {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(match self {
                Self::Request => "request lost",
                Self::Reply => "reply lost",
            })
        }
    }

    impl Flaky {
        fn new(server: AppendListServer, before: usize, after: usize) -> Self {
            Self {
                server,
                before,
                after,
                delivered: 0,
            }
        }

        fn deliver<T>(&mut self, op: impl FnOnce(&mut AppendListServer) -> T) -> Result<T, Lost> {
            if self.before > 0 {
                self.before -= 1;
                return Err(Lost::Request);
            }
            let answer = op(&mut self.server);
            self.delivered += 1;
            if self.after > 0 {
                self.after -= 1;
                return Err(Lost::Reply);
            }
            Ok(answer)
        }
    }

    impl AppendListService for Flaky {
        type Error = Lost;

        fn append_submit(
            &mut self,
            request: &SubmissionRequest,
        ) -> Result<Result<SubmitOutcome, Refusal>, Lost> {
            self.deliver(|server| AppendListService::append_submit(server, request).unwrap())
        }

        fn submission_state(
            &mut self,
            attribute: &str,
            submitter: &VerifyingKey,
            hash: &ContentHash,
        ) -> Result<Result<Option<SubmissionState>, Refusal>, Lost> {
            self.deliver(|server| {
                AppendListService::submission_state(server, attribute, submitter, hash).unwrap()
            })
        }
    }

    #[test]
    fn a_member_submits_and_sees_its_sequence() {
        let mut server = server();
        let member = key(2);
        let client = AppendListClient::new(&member);
        assert_eq!(
            client.submit(&mut server, ATTR, b"first".to_vec()),
            Ok(Landed::Appended(0))
        );
        assert_eq!(
            client.submit(&mut server, ATTR, b"second".to_vec()),
            Ok(Landed::Appended(1))
        );
        assert_eq!(
            client.state(&mut server, ATTR, b"second"),
            Ok(Some(SubmissionState::Appended(1)))
        );
        let entry = &server.read(ATTR, Some(0), None).unwrap()[0];
        assert_eq!(entry.hash, ContentHash::of(b"second"));
        assert_eq!(entry.submitter, member.verifying_key());
    }

    #[test]
    fn an_outsider_sees_pending_until_approval_or_rejection() {
        let mut server = server();
        let owner = key(1).verifying_key();
        let outsider = key(5);
        let client = AppendListClient::new(&outsider);

        let Ok(Landed::Pending(approved)) =
            client.submit(&mut server, ATTR, b"kind words".to_vec())
        else {
            panic!("an outsider's submission is queued");
        };
        assert_eq!(
            client.state(&mut server, ATTR, b"kind words"),
            Ok(Some(SubmissionState::Pending(approved)))
        );
        assert_eq!(server.approve(ATTR, &owner, approved), Ok(0));
        assert_eq!(
            client.state(&mut server, ATTR, b"kind words"),
            Ok(Some(SubmissionState::Appended(0)))
        );

        let Ok(Landed::Pending(rejected)) = client.submit(&mut server, ATTR, b"spam".to_vec())
        else {
            panic!("queued");
        };
        server.reject(ATTR, &owner, rejected).unwrap();
        assert_eq!(
            client.state(&mut server, ATTR, b"spam"),
            Ok(Some(SubmissionState::Refused(rejected)))
        );
        assert_eq!(client.state(&mut server, ATTR, b"never sent"), Ok(None));
    }

    #[test]
    fn a_lost_reply_is_retried_without_a_duplicate_entry() {
        // The server applies the first attempt, and its reply is lost.
        let mut flaky = Flaky::new(server(), 0, 1);
        let member = key(2);
        let client = AppendListClient::new(&member);
        assert_eq!(
            client.submit(&mut flaky, ATTR, b"once".to_vec()),
            Ok(Landed::Appended(0))
        );
        assert_eq!(flaky.delivered, 2);
        assert_eq!(flaky.server.read(ATTR, None, None).unwrap().len(), 1);

        // The same for a pending submission: one queue entry, one id.
        let mut flaky = Flaky::new(server(), 1, 2);
        let outsider = key(5);
        let client = AppendListClient::new(&outsider).with_attempts(4);
        let Ok(Landed::Pending(id)) = client.submit(&mut flaky, ATTR, b"once".to_vec()) else {
            panic!("queued");
        };
        assert_eq!(flaky.delivered, 3);
        let pending = flaky.server.pending(ATTR, &key(1).verifying_key()).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
    }

    #[test]
    fn exhausting_attempts_reports_transport_and_a_later_resubmit_is_still_one_entry() {
        let member = key(2);
        let client = AppendListClient::new(&member).with_attempts(2);
        let request = client.request(ATTR, b"slow".to_vec());

        // Both attempts reach the server and both replies are lost.
        let mut flaky = Flaky::new(server(), 0, 2);
        assert_eq!(
            client.resubmit(&mut flaky, &request),
            Err(ClientError::Transport(Lost::Reply))
        );
        // The caller sends the same request later, after the owner closed
        // the list: the retry still reports the entry that landed.
        flaky.server.close(ATTR, &key(1).verifying_key()).unwrap();
        assert_eq!(
            client.resubmit(&mut flaky, &request),
            Ok(Landed::Appended(0))
        );
        assert_eq!(flaky.server.read(ATTR, None, None).unwrap().len(), 1);
    }

    #[test]
    fn refusals_are_returned_at_once_and_not_retried() {
        let mut fresh = AppendListServer::new();
        fresh
            .create_append_list(ATTR, key(1).verifying_key(), &AttributeConfig::default())
            .unwrap();
        let mut flaky = Flaky::new(fresh, 0, 0);
        let member = key(2);
        let client = AppendListClient::new(&member).with_attempts(5);
        assert_eq!(
            client.submit(&mut flaky, ATTR, b"too early".to_vec()),
            Err(ClientError::Refused(Refusal::NotOpened))
        );
        assert_eq!(flaky.delivered, 1);
        assert_eq!(
            client.submit(&mut flaky, "nowhere", b"x".to_vec()),
            Err(ClientError::Refused(Refusal::NoSuchAttribute))
        );
    }

    #[test]
    fn requests_are_hashed_and_signed_by_the_client_key() {
        let member = key(2);
        let request = SubmissionRequest::sign(ATTR, b"body".to_vec(), &member);
        assert_eq!(request.hash, ContentHash::of(b"body"));
        assert_eq!(request.submitter, member.verifying_key());
        assert!(
            member
                .verifying_key()
                .verify_strict(&request.hash.0, &request.signature)
                .is_ok()
        );
        // Signing is deterministic, so a rebuilt request is the same request.
        assert_eq!(
            request,
            SubmissionRequest::sign(ATTR, b"body".to_vec(), &member)
        );
    }
}
