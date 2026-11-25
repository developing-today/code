//! OPAQUE (RFC 9807) username/password login over a client-held ID key
//! (#13710 slice 2; design of record: #5684 synthesis §5 and the #5885
//! feasibility record).
//!
//! The server stores an OPAQUE registration record (`password_file`) and
//! the ID key *ciphertext* only. The ID key is random and generated on the
//! client; it is never derived from the password. After a successful
//! OPAQUE exchange the client gets the RFC 9807 `export_key`, which the
//! server never learns, and unwraps its ID key from the stored ciphertext
//! with a key derived from it, through the existing
//! [`key_envelope`](crate::key_envelope) format.
//!
//! A successful login yields an [`AuthenticatedAccount`] — evidence of
//! authentication only. It can start a first binding for an unbound
//! account and nothing else; key changes go through
//! [`BindingRegistry::update`](crate::login_binding::BindingRegistry::update).
//! A password reset is a fresh OPAQUE registration: it replaces the
//! password file and the wrapped ciphertext the server holds, and never
//! the bound ID key (#5885 invariant 5).
//!
//! The server's OPRF seed and key-exchange key live in [`OpaqueServer`].
//! They are high-entropy secret material: the type's `Debug` output is
//! redacted and the serialized form is returned in a zeroizing buffer, so
//! callers can keep it in a secrets store, never in code, logs or this
//! repository.

use std::fmt;

use ed25519_dalek::SigningKey;
use opaque_ke::{
    CipherSuite, ClientLogin, ClientLoginFinishParameters, ClientRegistration,
    ClientRegistrationFinishParameters, CredentialFinalization, CredentialRequest,
    CredentialResponse, Identifiers, RegistrationRequest, RegistrationResponse, RegistrationUpload,
    ServerLogin, ServerLoginParameters, ServerRegistration, ServerSetup,
};
use rand_core_06::OsRng;
use sha2::{Digest as _, Sha256};
use zeroize::Zeroizing;

use crate::key_envelope;
use crate::login_binding::{AuthAccount, AuthenticatedAccount, BindingError};

/// Domain that derives the key-envelope secret from the OPAQUE
/// `export_key`.
const EXPORT_WRAP_DOMAIN: &[u8] = b"id/opaque-export-wrap/v1\0";

/// Context both sides bind into the OPAQUE key exchange.
const LOGIN_CONTEXT: &[u8] = b"id/opaque-login/v1";

/// User-facing disclosure of what the server-stored backup exposes
/// (#5885 invariant 4; #5887 §6). Shown when a password login is set up.
pub const OFFLINE_GUESSING_DISCLOSURE: &str = "Your identity key is stored on our \
server only in encrypted form, locked by your password. We cannot read it. \
But if our server were ever breached, an attacker could try to guess your \
password offline, without us noticing. A long, unique password makes that \
guessing impractical; a short or reused one does not. Your password is not \
your identity: resetting it never recovers or replaces your identity key.";

/// The OPAQUE cipher suite: ristretto255 OPRF, 3DH with SHA-512, and
/// Argon2id (default parameters) as the key-stretching function.
#[derive(Debug, Clone, Copy)]
pub struct IdOpaqueSuite;

impl CipherSuite for IdOpaqueSuite {
    type OprfCs = opaque_ke::Ristretto255;
    type KeyExchange = opaque_ke::TripleDh<opaque_ke::Ristretto255, sha2_010::Sha512>;
    type Ksf = opaque_ke::argon2::Argon2<'static>;
}

/// Typed OPAQUE login errors. Protocol failures deliberately carry no
/// detail: a wrong password and a tampered message look the same.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpaqueError {
    /// The OPAQUE exchange failed (wrong password, unknown account,
    /// tampered or malformed message).
    Protocol,
    /// A message or stored record could not be decoded.
    Decode(&'static str),
    /// The wrapped ID key could not be created or opened.
    Envelope,
    /// The account identifier is invalid.
    Account(BindingError),
}

impl fmt::Display for OpaqueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol => f.write_str("password login failed"),
            Self::Decode(what) => write!(f, "could not decode {what}"),
            Self::Envelope => f.write_str("wrapped ID key could not be processed"),
            Self::Account(err) => write!(f, "invalid account: {err}"),
        }
    }
}

impl std::error::Error for OpaqueError {}

/// The server's long-term OPAQUE secrets (OPRF seed and key-exchange
/// key pair).
pub struct OpaqueServer {
    setup: ServerSetup<IdOpaqueSuite>,
}

impl fmt::Debug for OpaqueServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OpaqueServer { .. }")
    }
}

impl OpaqueServer {
    /// Generates fresh server secrets.
    pub fn generate() -> Self {
        Self {
            setup: ServerSetup::<IdOpaqueSuite>::new(&mut OsRng),
        }
    }

    /// The serialized secrets, for a secrets store only.
    pub fn to_secret_bytes(&self) -> Zeroizing<Vec<u8>> {
        Zeroizing::new(self.setup.serialize().to_vec())
    }

    /// Restores secrets serialized by [`OpaqueServer::to_secret_bytes`].
    ///
    /// # Errors
    ///
    /// [`OpaqueError::Decode`] for malformed bytes.
    pub fn from_secret_bytes(bytes: &[u8]) -> Result<Self, OpaqueError> {
        ServerSetup::<IdOpaqueSuite>::deserialize(bytes)
            .map(|setup| Self { setup })
            .map_err(|_err| OpaqueError::Decode("server secrets"))
    }

    /// Registration step 2: answers the client's registration request.
    ///
    /// # Errors
    ///
    /// [`OpaqueError`] for a bad account or message.
    pub fn registration_response(
        &self,
        account: &AuthAccount,
        request: &[u8],
    ) -> Result<Vec<u8>, OpaqueError> {
        let id = credential_id(account)?;
        let request = RegistrationRequest::<IdOpaqueSuite>::deserialize(request)
            .map_err(|_err| OpaqueError::Decode("registration request"))?;
        let result = ServerRegistration::<IdOpaqueSuite>::start(&self.setup, request, id)
            .map_err(|_err| OpaqueError::Protocol)?;
        Ok(result.message.serialize().to_vec())
    }

    /// Registration step 4: stores the client's upload. The record holds
    /// the OPAQUE password file and the wrapped ID key ciphertext only.
    ///
    /// # Errors
    ///
    /// [`OpaqueError::Decode`] for a malformed upload.
    pub fn finish_registration(
        &self,
        upload: &ClientUpload,
    ) -> Result<OpaqueAccountRecord, OpaqueError> {
        let message = RegistrationUpload::<IdOpaqueSuite>::deserialize(&upload.registration)
            .map_err(|_err| OpaqueError::Decode("registration upload"))?;
        let file = ServerRegistration::<IdOpaqueSuite>::finish(message);
        Ok(OpaqueAccountRecord {
            password_file: file.serialize().to_vec(),
            wrapped_id_key: upload.wrapped_id_key.clone(),
        })
    }

    /// Login step 2: answers a credential request. Pass `None` for an
    /// unknown account; OPAQUE then answers with a decoy so the response
    /// does not reveal whether the account exists.
    ///
    /// # Errors
    ///
    /// [`OpaqueError`] for a bad account, message or stored record.
    pub fn login_response(
        &self,
        account: &AuthAccount,
        record: Option<&OpaqueAccountRecord>,
        request: &[u8],
    ) -> Result<(Vec<u8>, PendingServerLogin), OpaqueError> {
        let id = credential_id(account)?;
        let file = record
            .map(|record| {
                ServerRegistration::<IdOpaqueSuite>::deserialize(&record.password_file)
                    .map_err(|_err| OpaqueError::Decode("password file"))
            })
            .transpose()?;
        let request = CredentialRequest::<IdOpaqueSuite>::deserialize(request)
            .map_err(|_err| OpaqueError::Decode("credential request"))?;
        let result =
            ServerLogin::start(&mut OsRng, &self.setup, file, request, id, server_params())
                .map_err(|_err| OpaqueError::Protocol)?;
        Ok((
            result.message.serialize().to_vec(),
            PendingServerLogin {
                state: result.state,
                account: account.clone(),
            },
        ))
    }
}

/// What the server stores per OPAQUE account: no password verifier the
/// server could replay, and the ID key only as ciphertext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpaqueAccountRecord {
    /// The OPAQUE registration record.
    pub password_file: Vec<u8>,
    /// The client's ID key, wrapped under its `export_key`-derived key.
    pub wrapped_id_key: Vec<u8>,
}

/// The client's registration upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientUpload {
    /// The OPAQUE `RegistrationUpload` message.
    pub registration: Vec<u8>,
    /// The ID key ciphertext for the server to store.
    pub wrapped_id_key: Vec<u8>,
}

/// Server state between login steps 2 and 4.
pub struct PendingServerLogin {
    state: ServerLogin<IdOpaqueSuite>,
    account: AuthAccount,
}

impl fmt::Debug for PendingServerLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingServerLogin")
            .field("account", &self.account)
            .finish_non_exhaustive()
    }
}

impl PendingServerLogin {
    /// Login step 4: checks the client's finalization. Success is
    /// authentication of the account only.
    ///
    /// # Errors
    ///
    /// [`OpaqueError::Protocol`] for a wrong password or tampered
    /// exchange.
    pub fn finish(self, finalization: &[u8]) -> Result<AuthenticatedAccount, OpaqueError> {
        let message = CredentialFinalization::<IdOpaqueSuite>::deserialize(finalization)
            .map_err(|_err| OpaqueError::Decode("credential finalization"))?;
        self.state
            .finish(message, server_params())
            .map_err(|_err| OpaqueError::Protocol)?;
        Ok(AuthenticatedAccount::from_verified_login(self.account))
    }
}

/// Client state between registration steps 1 and 3.
pub struct PendingClientRegistration {
    state: ClientRegistration<IdOpaqueSuite>,
}

impl fmt::Debug for PendingClientRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingClientRegistration { .. }")
    }
}

/// Registration step 1 (client): blinds the password.
///
/// # Errors
///
/// [`OpaqueError::Protocol`] if blinding fails.
pub fn client_registration_start(
    password: &[u8],
) -> Result<(Vec<u8>, PendingClientRegistration), OpaqueError> {
    let result = ClientRegistration::<IdOpaqueSuite>::start(&mut OsRng, password)
        .map_err(|_err| OpaqueError::Protocol)?;
    Ok((
        result.message.serialize().to_vec(),
        PendingClientRegistration {
            state: result.state,
        },
    ))
}

impl PendingClientRegistration {
    /// Registration step 3 (client): completes the OPAQUE registration
    /// and wraps the random, client-held ID key under the `export_key`.
    /// The ID key itself never leaves the client.
    ///
    /// # Errors
    ///
    /// [`OpaqueError`] for a bad server response or wrapping failure.
    pub fn finish(
        self,
        password: &[u8],
        response: &[u8],
        id_key: &SigningKey,
    ) -> Result<ClientUpload, OpaqueError> {
        let response = RegistrationResponse::<IdOpaqueSuite>::deserialize(response)
            .map_err(|_err| OpaqueError::Decode("registration response"))?;
        let result = self
            .state
            .finish(
                &mut OsRng,
                password,
                response,
                ClientRegistrationFinishParameters::default(),
            )
            .map_err(|_err| OpaqueError::Protocol)?;
        let secret = wrap_secret(&result.export_key);
        let key = Zeroizing::new(id_key.to_bytes());
        let wrapped_id_key =
            key_envelope::encrypt(&secret[..], &key).map_err(|_err| OpaqueError::Envelope)?;
        Ok(ClientUpload {
            registration: result.message.serialize().to_vec(),
            wrapped_id_key,
        })
    }
}

/// Client state between login steps 1 and 3.
pub struct PendingClientLogin {
    state: ClientLogin<IdOpaqueSuite>,
}

impl fmt::Debug for PendingClientLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PendingClientLogin { .. }")
    }
}

/// Login step 1 (client): starts the OPAQUE key exchange.
///
/// # Errors
///
/// [`OpaqueError::Protocol`] if blinding fails.
pub fn client_login_start(password: &[u8]) -> Result<(Vec<u8>, PendingClientLogin), OpaqueError> {
    let result = ClientLogin::<IdOpaqueSuite>::start(&mut OsRng, password)
        .map_err(|_err| OpaqueError::Protocol)?;
    Ok((
        result.message.serialize().to_vec(),
        PendingClientLogin {
            state: result.state,
        },
    ))
}

impl PendingClientLogin {
    /// Login step 3 (client): verifies the server and returns the
    /// finalization message plus a key that opens the wrapped ID key.
    ///
    /// # Errors
    ///
    /// [`OpaqueError::Protocol`] for a wrong password or a server that
    /// does not hold this account's record.
    pub fn finish(
        self,
        password: &[u8],
        response: &[u8],
    ) -> Result<(Vec<u8>, IdKeyUnwrapper), OpaqueError> {
        let response = CredentialResponse::<IdOpaqueSuite>::deserialize(response)
            .map_err(|_err| OpaqueError::Decode("credential response"))?;
        let params =
            ClientLoginFinishParameters::new(Some(LOGIN_CONTEXT), Identifiers::default(), None);
        let result = self
            .state
            .finish(&mut OsRng, password, response, params)
            .map_err(|_err| OpaqueError::Protocol)?;
        Ok((
            result.message.serialize().to_vec(),
            IdKeyUnwrapper {
                secret: wrap_secret(&result.export_key),
            },
        ))
    }
}

/// Client-side key derived from a login's `export_key`; opens the stored
/// ID key ciphertext.
pub struct IdKeyUnwrapper {
    secret: Zeroizing<[u8; 32]>,
}

impl fmt::Debug for IdKeyUnwrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IdKeyUnwrapper { .. }")
    }
}

impl IdKeyUnwrapper {
    /// Opens the wrapped ID key the server returned after login.
    ///
    /// # Errors
    ///
    /// [`OpaqueError::Envelope`] if the ciphertext does not open under
    /// this login's key.
    pub fn unwrap_id_key(&self, wrapped: &[u8]) -> Result<SigningKey, OpaqueError> {
        let key = Zeroizing::new(
            key_envelope::decrypt(&self.secret[..], wrapped)
                .map_err(|_err| OpaqueError::Envelope)?,
        );
        Ok(SigningKey::from_bytes(&key))
    }
}

fn server_params() -> ServerLoginParameters<'static, 'static> {
    ServerLoginParameters {
        context: Some(LOGIN_CONTEXT),
        identifiers: Identifiers::default(),
    }
}

fn credential_id(account: &AuthAccount) -> Result<&[u8], OpaqueError> {
    match account {
        AuthAccount::Opaque { account_id } => Ok(account_id.as_bytes()),
        AuthAccount::Oidc { .. } => Err(OpaqueError::Account(BindingError::Field {
            field: "account",
            problem: "is not an OPAQUE account",
        })),
    }
}

fn wrap_secret(export_key: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut hasher = Sha256::new();
    hasher.update(EXPORT_WRAP_DOMAIN);
    hasher.update(export_key);
    Zeroizing::new(hasher.finalize().into())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::login_binding::{BindingRegistry, BindingStatement, CustodyProfile, SignedBinding};

    const PASSWORD: &[u8] = b"correct horse battery staple";

    fn alice() -> AuthAccount {
        AuthAccount::opaque("alice").unwrap()
    }

    fn register(
        server: &OpaqueServer,
        account: &AuthAccount,
        password: &[u8],
        id_key: &SigningKey,
    ) -> OpaqueAccountRecord {
        let (request, pending) = client_registration_start(password).unwrap();
        let response = server.registration_response(account, &request).unwrap();
        let upload = pending.finish(password, &response, id_key).unwrap();
        server.finish_registration(&upload).unwrap()
    }

    fn login(
        server: &OpaqueServer,
        account: &AuthAccount,
        record: Option<&OpaqueAccountRecord>,
        password: &[u8],
    ) -> Result<(AuthenticatedAccount, IdKeyUnwrapper), OpaqueError> {
        let (request, client) = client_login_start(password)?;
        let (response, pending) = server.login_response(account, record, &request)?;
        let (finalization, unwrapper) = client.finish(password, &response)?;
        Ok((pending.finish(&finalization)?, unwrapper))
    }

    #[test]
    fn test_register_login_and_unwrap_id_key() {
        let server = OpaqueServer::generate();
        let id_key = SigningKey::from_bytes(&[11; 32]);
        let record = register(&server, &alice(), PASSWORD, &id_key);
        let (session, unwrapper) = login(&server, &alice(), Some(&record), PASSWORD).unwrap();
        assert_eq!(session.account(), &alice());
        let opened = unwrapper.unwrap_id_key(&record.wrapped_id_key).unwrap();
        assert_eq!(opened.to_bytes(), id_key.to_bytes());
    }

    #[test]
    fn test_wrong_password_and_unknown_account_fail_alike() {
        let server = OpaqueServer::generate();
        let record = register(
            &server,
            &alice(),
            PASSWORD,
            &SigningKey::from_bytes(&[11; 32]),
        );
        assert_eq!(
            login(&server, &alice(), Some(&record), b"wrong password").map(|_| ()),
            Err(OpaqueError::Protocol)
        );
        let bob = AuthAccount::opaque("bob").unwrap();
        assert_eq!(
            login(&server, &bob, None, PASSWORD).map(|_| ()),
            Err(OpaqueError::Protocol)
        );
    }

    #[test]
    fn test_server_secrets_round_trip_and_debug_is_redacted() {
        let server = OpaqueServer::generate();
        let id_key = SigningKey::from_bytes(&[11; 32]);
        let record = register(&server, &alice(), PASSWORD, &id_key);
        let restored = OpaqueServer::from_secret_bytes(&server.to_secret_bytes()).unwrap();
        assert!(login(&restored, &alice(), Some(&record), PASSWORD).is_ok());
        assert_eq!(format!("{server:?}"), "OpaqueServer { .. }");
    }

    #[test]
    fn invariant_1_server_never_receives_the_unwrapped_id_key() {
        let server = OpaqueServer::generate();
        let id_key = SigningKey::from_bytes(&[11; 32]);
        let (request, pending) = client_registration_start(PASSWORD).unwrap();
        let response = server.registration_response(&alice(), &request).unwrap();
        let upload = pending.finish(PASSWORD, &response, &id_key).unwrap();
        let secret = id_key.to_bytes();
        for sent in [&request, &upload.registration, &upload.wrapped_id_key] {
            assert!(!sent.windows(32).any(|w| w == secret));
        }
    }

    #[test]
    fn invariant_4_stored_backup_is_ciphertext_and_disclosed() {
        let server = OpaqueServer::generate();
        let id_key = SigningKey::from_bytes(&[11; 32]);
        let record = register(&server, &alice(), PASSWORD, &id_key);
        let secret = id_key.to_bytes();
        assert!(!record.wrapped_id_key.windows(32).any(|w| w == secret));
        assert!(!record.password_file.windows(32).any(|w| w == secret));
        assert!(
            !record
                .password_file
                .windows(PASSWORD.len())
                .any(|w| w == PASSWORD)
        );
        // The ciphertext does not open without the login's export key.
        assert!(key_envelope::decrypt(PASSWORD, &record.wrapped_id_key).is_err());
        assert!(OFFLINE_GUESSING_DISCLOSURE.contains("guess your password offline"));
        assert!(OFFLINE_GUESSING_DISCLOSURE.contains("never recovers or replaces"));
    }

    #[test]
    fn invariant_5_password_reset_never_rebinds_the_id_key() {
        let server = OpaqueServer::generate();
        let id_key = SigningKey::from_bytes(&[11; 32]);
        let record = register(&server, &alice(), PASSWORD, &id_key);
        let (session, unwrapper) = login(&server, &alice(), Some(&record), PASSWORD).unwrap();
        let held = unwrapper.unwrap_id_key(&record.wrapped_id_key).unwrap();

        let mut registry = BindingRegistry::new();
        let statement = BindingStatement {
            id_public_key: held.verifying_key().to_bytes(),
            account: alice(),
            custody: CustodyProfile::UserHeld,
            recovery_commitment: [3; 32],
            generation: 1,
            prev: None,
        };
        registry
            .enrol(
                &session,
                SignedBinding::enrol(statement.clone(), &held).unwrap(),
            )
            .unwrap();

        // Reset: a new password and a new key re-register the account.
        let attacker_key = SigningKey::from_bytes(&[12; 32]);
        let reset = register(&server, &alice(), b"new password 123", &attacker_key);
        let (reset_session, _) =
            login(&server, &alice(), Some(&reset), b"new password 123").unwrap();
        let rebind = BindingStatement {
            id_public_key: attacker_key.verifying_key().to_bytes(),
            ..statement
        };
        assert_eq!(
            registry
                .enrol(
                    &reset_session,
                    SignedBinding::enrol(rebind, &attacker_key).unwrap()
                )
                .map(|_| ()),
            Err(BindingError::AlreadyBound)
        );
        assert_eq!(
            registry.bound_key(&alice()),
            Some(id_key.verifying_key().to_bytes())
        );
    }

    #[test]
    fn test_oidc_account_is_not_an_opaque_credential() {
        let server = OpaqueServer::generate();
        let oidc = AuthAccount::oidc("https://accounts.google.com", "123").unwrap();
        let (request, _) = client_registration_start(PASSWORD).unwrap();
        assert!(matches!(
            server.registration_response(&oidc, &request),
            Err(OpaqueError::Account(_))
        ));
    }
}
