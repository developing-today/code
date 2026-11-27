//! `convert` through the gateway service (#14393): the posted
//! path, refusals that write nothing, atomic failure with the cap
//! released, quotes, restart, and recovery of a lost answer.

use std::fs;
use std::path::PathBuf;

use id_currency_registry::{AccountKind, account_spec};
use p2panda_core::SigningKey as PandaKey;

use super::super::{
    FileIntentStore, GatewayService, InMemoryIntentStore, IntentFinding, IntentState, IntentStore,
    ServiceError,
};
use super::{
    ConversionLegRole, ConversionOutcome, ConversionPresentation, ConversionResponse, leg_ids,
};
use crate::chain::Event;
use crate::currency::conversion::{FailedPreSubmit, PresentedObject};
use crate::currency::gateway::{
    Convert as GwConvert, CurrencyOp, CurrencyRequest, Issue as GwIssue,
    RegisterCurrency as GwRegister, sign_request,
};
use crate::currency::gateway_poc::verify_receipt;
use crate::currency::graph::{
    CurrencyBinding, CurrencyDefinition, CurrencyId, SupplyCap, TokenIssue, sign_definition,
    sign_issue,
};
use crate::currency::{AuthorisedCallers, RateAuthorisation, issuer_id, sign_authorisation};
use crate::currency_poc::ledger::{
    Account, AccountBalance, AccountError, InMemoryLedger, LedgerBackend, Transfer, TransferError,
};
use crate::hosted_binding::hex_encode;

const NOW: u64 = 10_000;
const SUPPLY: u128 = 1_000_000;

struct Actor {
    device_key: ed25519_dalek::SigningKey,
    account_key: PandaKey,
    account: String,
    device: String,
}

fn actor(device_seed: u8, account_seed: u8) -> Actor {
    let device_key = ed25519_dalek::SigningKey::from_bytes(&[device_seed; 32]);
    let device = hex_encode(&device_key.verifying_key().to_bytes());
    let account_key = PandaKey::from_bytes(&[account_seed; 32]);
    let account = hex_encode(&issuer_id(&account_key));
    Actor {
        device_key,
        account_key,
        account,
        device,
    }
}

/// A currency the fixture registered and defined.
struct Currency {
    id: CurrencyId,
    key: [u8; 32],
    definition: Event,
    ledger: u32,
}

/// A ledger whose next chain submission loses its answer: the
/// chain is applied (or not) and the backend reports nothing,
/// the way a crash between submission and reply looks.
#[derive(Default)]
struct LossyLedger {
    inner: InMemoryLedger,
    /// `Some(true)`: apply the next chain, then lose the answer.
    /// `Some(false)`: lose the next chain before it lands.
    lose_next: Option<bool>,
}

impl LedgerBackend for LossyLedger {
    fn create_accounts(&mut self, accounts: &[Account]) -> Vec<Result<(), AccountError>> {
        self.inner.create_accounts(accounts)
    }

    fn create_transfers(&mut self, batch: &[Transfer]) -> Vec<Result<(), TransferError>> {
        match self.lose_next.take() {
            Some(true) => {
                let _ = self.inner.create_transfers(batch);
                Vec::new()
            }
            Some(false) => Vec::new(),
            None => self.inner.create_transfers(batch),
        }
    }

    fn try_lookup_transfer(&self, id: u128) -> anyhow::Result<Option<Transfer>> {
        self.inner.try_lookup_transfer(id)
    }

    fn try_lookup_account(&self, id: u128) -> anyhow::Result<Option<Account>> {
        self.inner.try_lookup_account(id)
    }

    fn try_account_balance(&self, id: u128) -> anyhow::Result<Option<AccountBalance>> {
        self.inner.try_account_balance(id)
    }

    fn try_transfers_with_user_data_128(&self, value: u128) -> anyhow::Result<Vec<Transfer>> {
        self.inner.try_transfers_with_user_data_128(value)
    }

    fn try_accounts_on_ledger(&self, ledger: u32) -> anyhow::Result<Vec<Account>> {
        self.inner.try_accounts_on_ledger(ledger)
    }

    fn try_transfers_on_ledger(&self, ledger: u32, limit: usize) -> anyhow::Result<Vec<Transfer>> {
        self.inner.try_transfers_on_ledger(ledger, limit)
    }
}

struct Fixture<B: LedgerBackend, I: IntentStore> {
    svc: GatewayService<B, I>,
    gateway_vk: ed25519_dalek::VerifyingKey,
    /// Issues DOLR, the source currency.
    dollar_issuer: Actor,
    /// Issues EURO and is the LP: it holds EURO inventory and
    /// signs the DOLR → EURO rate authorisation.
    euro_issuer: Actor,
    alice: Actor,
    dollar: Currency,
    euro: Currency,
    nonce: u64,
}

fn open<B: LedgerBackend, I: IntentStore>(backend: B, store: I) -> GatewayService<B, I> {
    let gateway_key = ed25519_dalek::SigningKey::from_bytes(&[250; 32]);
    let mut svc = GatewayService::open(backend, gateway_key, store).expect("service opens");
    for a in [actor(1, 11), actor(5, 55), actor(2, 22), actor(3, 33)] {
        svc.attach_device(&a.device, &a.account);
    }
    svc
}

fn sign<B: LedgerBackend, I: IntentStore>(
    fx: &mut Fixture<B, I>,
    device_key: &ed25519_dalek::SigningKey,
    request: &CurrencyRequest,
) -> (crate::directory_auth::Signed, Vec<u8>) {
    fx.nonce += 1;
    let nonce = format!("nonce-{}", fx.nonce);
    sign_request(device_key, request, NOW, &nonce).expect("request signs")
}

fn register<B: LedgerBackend, I: IntentStore>(
    svc: &mut GatewayService<B, I>,
    issuer: &Actor,
    symbol: &str,
    nonce: &str,
) -> Currency {
    let request = CurrencyRequest {
        client_request_id: 1,
        op: CurrencyOp::RegisterCurrency(GwRegister {
            name: format!("{symbol} money"),
            symbol: symbol.to_owned(),
            scale: 2,
            cap: SUPPLY,
        }),
    };
    let (signed, body) = sign_request(&issuer.device_key, &request, NOW, nonce).expect("signs");
    let receipt = svc.handle(&signed, &body, NOW, None).expect("register");
    let ledger = receipt.receipt.ledger_id.expect("a ledger is allocated");
    let definition = CurrencyDefinition {
        name: format!("{symbol} money"),
        symbol: symbol.to_owned(),
        scale: 2,
        supply: SupplyCap { capped: SUPPLY },
        binding: CurrencyBinding::Ledger { ledger },
        memo: None,
    };
    let (id, event) =
        sign_definition(&issuer.account_key, 0, None, &definition, vec![]).expect("defines");
    let body = definition.body().expect("definition body");
    svc.record_definition(&event, &body)
        .expect("definition records");
    Currency {
        key: *id.compact_hash().as_bytes(),
        id,
        definition: event,
        ledger,
    }
}

fn issue<B: LedgerBackend, I: IntentStore>(
    svc: &mut GatewayService<B, I>,
    issuer: &Actor,
    currency: &Currency,
    to: &Actor,
    amount: u128,
    client_request_id: u128,
    nonce: &str,
) {
    let token = TokenIssue {
        currency: currency.id.compact_hash(),
        amount,
        to: issuer_id(&to.account_key),
        memo: None,
    };
    let event = sign_issue(
        &issuer.account_key,
        1,
        Some(currency.definition.id),
        &currency.id,
        &token,
        vec![],
    )
    .expect("issue signs");
    let body = token.body().expect("issue body");
    let request = CurrencyRequest {
        client_request_id,
        op: CurrencyOp::Issue(GwIssue {
            currency: currency.key,
            amount,
            to: to.account.clone(),
        }),
    };
    let (signed, request_body) =
        sign_request(&issuer.device_key, &request, NOW, nonce).expect("signs");
    svc.handle(&signed, &request_body, NOW, Some((&event, &body)))
        .expect("issue posts");
}

/// Two currencies, 10 000 DOLR issued to Alice, and 5 000 EURO
/// issued to the EURO issuer's own holder account (its inventory).
fn fixture<B: LedgerBackend, I: IntentStore>(backend: B, store: I) -> Fixture<B, I> {
    let mut svc = open(backend, store);
    let gateway_vk = ed25519_dalek::SigningKey::from_bytes(&[250; 32]).verifying_key();
    let dollar_issuer = actor(1, 11);
    let euro_issuer = actor(5, 55);
    let alice = actor(2, 22);
    let dollar = register(&mut svc, &dollar_issuer, "DOLR", "setup-1");
    let euro = register(&mut svc, &euro_issuer, "EURO", "setup-2");
    issue(
        &mut svc,
        &dollar_issuer,
        &dollar,
        &alice,
        10_000,
        2,
        "setup-3",
    );
    issue(
        &mut svc,
        &euro_issuer,
        &euro,
        &euro_issuer,
        5_000,
        2,
        "setup-4",
    );
    Fixture {
        svc,
        gateway_vk,
        dollar_issuer,
        euro_issuer,
        alice,
        dollar,
        euro,
        nonce: 0,
    }
}

/// The EURO issuer's DOLR → EURO offer: 0.9 EURO per DOLR, a 1%
/// fee, 10..=5 000 per conversion, 3 000 in total.
fn authorisation<B: LedgerBackend, I: IntentStore>(fx: &Fixture<B, I>) -> RateAuthorisation {
    RateAuthorisation {
        authorisation_id: 7,
        version: 1,
        issuer_id: issuer_id(&fx.euro_issuer.account_key),
        source_ledger: hex_encode(&fx.dollar.key),
        dest_ledger: hex_encode(&fx.euro.key),
        rate_num: 9,
        rate_den: 10,
        min_amount: 10,
        max_amount: 5_000,
        total_cap: 3_000,
        authorised_callers: AuthorisedCallers::Any,
        not_before: 0,
        expires_at: 4_102_444_800,
        reversal_window_secs: None,
        fee_bps: Some(100),
    }
}

fn present(
    signer: &Actor,
    auth: &RateAuthorisation,
    quote_id: Option<u128>,
) -> ConversionPresentation {
    let event = sign_authorisation(&signer.account_key, 0, None, auth, vec![]).expect("signs");
    ConversionPresentation {
        issuer: auth.issuer_id,
        authorisation_id: auth.authorisation_id,
        authorisations: vec![PresentedObject {
            event,
            body: auth.body().expect("authorisation body"),
        }],
        revokes: Vec::new(),
        quote_id,
        reversal_of: None,
    }
}

fn convert_request<B: LedgerBackend, I: IntentStore>(
    fx: &Fixture<B, I>,
    client_request_id: u128,
    amount: u128,
) -> CurrencyRequest {
    CurrencyRequest {
        client_request_id,
        op: CurrencyOp::Convert(GwConvert {
            source: fx.dollar.key,
            dest: fx.euro.key,
            from: fx.alice.account.clone(),
            amount,
        }),
    }
}

fn convert<B: LedgerBackend, I: IntentStore>(
    fx: &mut Fixture<B, I>,
    client_request_id: u128,
    amount: u128,
    presented: &ConversionPresentation,
) -> Result<ConversionResponse, ServiceError> {
    let request = convert_request(fx, client_request_id, amount);
    let device_key = fx.alice.device_key.clone();
    let (signed, body) = sign(fx, &device_key, &request);
    fx.svc.handle_convert(&signed, &body, NOW, presented)
}

fn balance<B: LedgerBackend, I: IntentStore>(
    fx: &Fixture<B, I>,
    currency: &Currency,
    owner: &Actor,
) -> u128 {
    fx.svc
        .holder_balance(&currency.key, &owner.account)
        .unwrap_or(0)
}

fn fee_balance<B: LedgerBackend, I: IntentStore>(fx: &Fixture<B, I>) -> u128 {
    let spec = account_spec(
        fx.dollar.ledger,
        AccountKind::Fee,
        &fx.dollar_issuer.account,
        "",
    );
    fx.svc
        .backend()
        .account_balance(spec.id)
        .map_or(0, |balance| balance.credits_posted)
}

fn lp(fx: &Fixture<impl LedgerBackend, impl IntentStore>) -> [u8; 32] {
    issuer_id(&fx.euro_issuer.account_key)
}

fn temp_store_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "id-gateway-convert-{tag}-{}-{n}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

/// The failing-first test of #14393: a signed `convert` request
/// with a presented authorisation posts one linked chain —
/// source, fee and destination legs — moves exactly the quoted
/// amounts, answers a retry from its intent without moving value
/// twice, consumes the cap, and reconciles clean.
#[test]
fn convert_posts_one_linked_chain_and_replays_from_its_intent() {
    let mut fx = fixture(InMemoryLedger::default(), InMemoryIntentStore::new());
    let auth = authorisation(&fx);
    let presented = present(&fx.euro_issuer, &auth, None);

    let response = convert(&mut fx, 9, 1_000, &presented).expect("conversion posts");
    assert!(verify_receipt(&fx.gateway_vk, &response.signed_receipt));
    assert_eq!(response.signed_receipt.receipt.op, "convert");
    assert_eq!(response.signed_receipt.receipt.account, fx.alice.account);
    assert_eq!(
        response.conversion_receipt.conversion_id,
        response.signed_receipt.receipt.idempotency
    );
    assert_eq!(
        response.conversion_receipt.outcome,
        ConversionOutcome::Posted
    );
    assert_eq!(response.conversion_receipt.legs.len(), 3);
    assert_eq!(
        response.conversion_receipt.authorisation_id,
        auth.authorisation_id
    );
    assert_eq!(
        response.conversion_receipt.authorisation_version,
        auth.version
    );
    assert_eq!(response.conversion_receipt.quote_id, None);
    assert_eq!(
        (
            response.conversion_receipt.amount_in,
            response.conversion_receipt.fee,
            response.conversion_receipt.amount_out
        ),
        (1_000, 10, 900)
    );

    // 1 000 DOLR at 0.9 is 900 EURO; the 1% fee is 10 DOLR on
    // its own leg to the DOLR issuer's fee account.
    assert_eq!(balance(&fx, &fx.dollar, &fx.alice), 8_990);
    assert_eq!(balance(&fx, &fx.euro, &fx.alice), 900);
    assert_eq!(balance(&fx, &fx.dollar, &fx.euro_issuer), 1_000);
    assert_eq!(balance(&fx, &fx.euro, &fx.euro_issuer), 4_100);
    assert_eq!(fee_balance(&fx), 10);
    assert_eq!(fx.svc.conversion_usage(&lp(&fx), 7), 1_000);

    // The intent holds the whole chain: three legs, linked but
    // for the last, each carrying the conversion id.
    let intents = fx.svc.intents().all().expect("intents read");
    let intent = intents
        .iter()
        .find(|intent| intent.op == "convert")
        .expect("a convert intent");
    assert_eq!(intent.state, IntentState::Posted);
    assert_eq!(intent.legs.len(), 3);
    assert_eq!(
        intent.legs.iter().map(|leg| leg.linked).collect::<Vec<_>>(),
        vec![true, true, false]
    );
    assert!(
        intent
            .legs
            .iter()
            .all(|leg| leg.user_data_128 == intent.idempotency)
    );
    assert_eq!(
        intent.legs.iter().map(|leg| leg.id).collect::<Vec<_>>(),
        leg_ids(intent.idempotency, 3)
    );
    let record = intent.conversion.as_ref().expect("conversion terms");
    assert_eq!(
        (record.amount_in, record.fee, record.amount_out),
        (1_000, 10, 900)
    );

    // A retry is answered from the intent; nothing moves twice.
    let replay = convert(&mut fx, 9, 1_000, &presented).expect("retry answers");
    assert_eq!(
        replay.signed_receipt.receipt,
        response.signed_receipt.receipt
    );
    assert_eq!(replay.conversion_receipt, response.conversion_receipt);
    assert_eq!(balance(&fx, &fx.euro, &fx.alice), 900);
    assert_eq!(fx.svc.conversion_usage(&lp(&fx), 7), 1_000);

    let report = fx.svc.reconcile_intents(NOW).expect("reconcile runs");
    assert!(report.is_clean(), "findings: {:?}", report.findings);
}

/// The F3 op carries no authorisation, so the generic entry point
/// refuses it by name rather than executing it unverified.
#[test]
fn handle_refuses_convert_without_a_presented_authorisation() {
    let mut fx = fixture(InMemoryLedger::default(), InMemoryIntentStore::new());
    let request = convert_request(&fx, 9, 1_000);
    let device_key = fx.alice.device_key.clone();
    let (signed, body) = sign(&mut fx, &device_key, &request);
    let error = fx
        .svc
        .handle(&signed, &body, NOW, None)
        .expect_err("convert needs handle_convert");
    assert!(matches!(error, ServiceError::Unsupported(_)), "{error:?}");
}

/// Every pre-flight refusal writes nothing: no intent, no ledger
/// movement, no usage.
#[test]
fn refusals_write_no_intent_and_move_nothing() {
    let mut fx = fixture(InMemoryLedger::default(), InMemoryIntentStore::new());
    let intents_before = fx.svc.intents().all().expect("read").len();
    let auth = authorisation(&fx);

    // Over the remaining cap (3 000 in total).
    let presented = present(&fx.euro_issuer, &auth, None);
    let error = convert(&mut fx, 9, 3_500, &presented).expect_err("over the cap");
    assert!(
        matches!(
            error,
            ServiceError::Conversion(FailedPreSubmit::CapExhausted {
                remaining_volume: 3_000
            })
        ),
        "{error:?}"
    );

    // An authorisation for another pair: the reverse direction.
    let mut reverse = auth.clone();
    reverse.source_ledger = auth.dest_ledger.clone();
    reverse.dest_ledger = auth.source_ledger.clone();
    let reversed = present(&fx.euro_issuer, &reverse, None);
    let error = convert(&mut fx, 10, 1_000, &reversed).expect_err("wrong pair");
    assert!(
        matches!(
            error,
            ServiceError::Conversion(_) | ServiceError::ConversionMismatch(_)
        ),
        "{error:?}"
    );

    // An authorisation another key signed, presented under the LP.
    let mut forged = auth.clone();
    forged.issuer_id = issuer_id(&fx.dollar_issuer.account_key);
    let mut foreign = present(&fx.dollar_issuer, &forged, None);
    foreign.issuer = lp(&fx);
    let error = convert(&mut fx, 11, 1_000, &foreign).expect_err("foreign signer");
    assert!(matches!(error, ServiceError::Conversion(_)), "{error:?}");

    // A quote this gateway never issued.
    let unquoted = present(&fx.euro_issuer, &auth, Some(99));
    let error = convert(&mut fx, 12, 1_000, &unquoted).expect_err("unknown quote");
    assert!(
        matches!(
            error,
            ServiceError::Conversion(FailedPreSubmit::UnknownQuote)
        ),
        "{error:?}"
    );

    // An LP that neither issues either currency nor holds both.
    let bob = actor(3, 33);
    let mut stranger = auth;
    stranger.issuer_id = issuer_id(&bob.account_key);
    let error =
        convert(&mut fx, 13, 1_000, &present(&bob, &stranger, None)).expect_err("ineligible LP");
    assert!(
        matches!(
            error,
            ServiceError::Conversion(FailedPreSubmit::LpNotEligible)
        ),
        "{error:?}"
    );

    assert_eq!(fx.svc.intents().all().expect("read").len(), intents_before);
    assert_eq!(balance(&fx, &fx.dollar, &fx.alice), 10_000);
    assert_eq!(balance(&fx, &fx.euro, &fx.euro_issuer), 5_000);
    assert_eq!(fx.svc.conversion_usage(&lp(&fx), 7), 0);
}

/// The LP's destination inventory is a constrained account: a
/// conversion it cannot cover fails as one atomic chain, the
/// failure is the recorded outcome, and the cap reservation is
/// released so later conversions still fit.
#[test]
fn short_inventory_fails_atomically_and_releases_the_cap() {
    let mut fx = fixture(InMemoryLedger::default(), InMemoryIntentStore::new());
    let mut auth = authorisation(&fx);
    auth.max_amount = 50_000;
    auth.total_cap = 100_000;
    let presented = present(&fx.euro_issuer, &auth, None);

    // 6 000 DOLR would pay 5 400 EURO; the LP holds 5 000.
    let error = convert(&mut fx, 9, 6_000, &presented).expect_err("inventory is short");
    assert!(
        matches!(
            error,
            ServiceError::Cluster(TransferError::DebitsMustNotExceedCredits)
        ),
        "{error:?}"
    );
    assert_eq!(balance(&fx, &fx.dollar, &fx.alice), 10_000);
    assert_eq!(balance(&fx, &fx.euro, &fx.euro_issuer), 5_000);
    assert_eq!(fee_balance(&fx), 0);
    assert_eq!(fx.svc.conversion_usage(&lp(&fx), 7), 0);

    // The verdict is durable: a retry replays it.
    let error = convert(&mut fx, 9, 6_000, &presented).expect_err("verdict replays");
    assert!(matches!(error, ServiceError::StoredVerdict(_)), "{error:?}");

    // Reconciliation finds no leg of the failed chain on the ledger.
    let report = fx.svc.reconcile_intents(NOW).expect("reconcile runs");
    assert!(report.is_clean(), "findings: {:?}", report.findings);

    convert(&mut fx, 10, 1_000, &presented).expect("a covered conversion posts");
    assert_eq!(fx.svc.conversion_usage(&lp(&fx), 7), 1_000);
}

/// A quote pins the amounts; a conversion under it pays exactly
/// them, and a conversion for another amount cannot use it.
#[test]
fn quoted_conversion_executes_the_quoted_amounts() {
    let mut fx = fixture(InMemoryLedger::default(), InMemoryIntentStore::new());
    let auth = authorisation(&fx);
    let presented = present(&fx.euro_issuer, &auth, None);
    let request = convert_request(&fx, 20, 1_000);
    let device_key = fx.alice.device_key.clone();
    let (signed, body) = sign(&mut fx, &device_key, &request);
    let intents_before = fx.svc.intents().all().expect("read").len();
    let quote = fx
        .svc
        .quote_convert(&signed, &body, NOW, &presented, 42)
        .expect("quote issues");
    assert_eq!((quote.x, quote.computed_y, quote.fee), (1_000, 900, 10));
    assert_eq!(
        fx.svc.intents().all().expect("read").len(),
        intents_before,
        "a quote writes no intent"
    );

    let quoted = present(&fx.euro_issuer, &auth, Some(42));
    let error = convert(&mut fx, 21, 500, &quoted).expect_err("quote is for 1 000");
    assert!(
        matches!(
            error,
            ServiceError::Conversion(FailedPreSubmit::QuoteMismatch)
        ),
        "{error:?}"
    );

    convert(&mut fx, 22, 1_000, &quoted).expect("quoted conversion posts");
    assert_eq!(balance(&fx, &fx.euro, &fx.alice), quote.computed_y);
    assert_eq!(fee_balance(&fx), quote.fee);
    let intents = fx.svc.intents().all().expect("read");
    let record = intents
        .iter()
        .find_map(|intent| intent.conversion.clone())
        .expect("conversion terms");
    assert_eq!(record.quote_id, Some(42));
}

/// A restarted service rebuilds the authorisation's usage from
/// the intent log, so the cap holds across restarts, and a retry
/// of a posted conversion is still answered from its record.
#[test]
fn restart_rebuilds_usage_from_the_intent_log() {
    let dir = temp_store_dir("restart");
    let mut fx = fixture(
        InMemoryLedger::default(),
        FileIntentStore::open(dir.clone()).expect("store opens"),
    );
    let auth = authorisation(&fx);
    let presented = present(&fx.euro_issuer, &auth, None);
    let receipt = convert(&mut fx, 9, 1_000, &presented).expect("conversion posts");

    let Fixture {
        svc,
        gateway_vk,
        dollar_issuer,
        euro_issuer,
        alice,
        dollar,
        euro,
        nonce,
    } = fx;
    let backend = svc.into_backend();
    let mut fx = Fixture {
        svc: open(
            backend,
            FileIntentStore::open(dir.clone()).expect("reopens"),
        ),
        gateway_vk,
        dollar_issuer,
        euro_issuer,
        alice,
        dollar,
        euro,
        nonce,
    };
    assert_eq!(fx.svc.conversion_usage(&lp(&fx), 7), 1_000);
    let error = convert(&mut fx, 10, 2_500, &presented).expect_err("cap counts the past");
    assert!(
        matches!(
            error,
            ServiceError::Conversion(FailedPreSubmit::CapExhausted {
                remaining_volume: 2_000
            })
        ),
        "{error:?}"
    );
    let replay = convert(&mut fx, 9, 1_000, &presented).expect("retry answers");
    assert_eq!(
        replay.signed_receipt.receipt,
        receipt.signed_receipt.receipt
    );
    assert_eq!(replay.conversion_receipt, receipt.conversion_receipt);
    assert_eq!(balance(&fx, &fx.euro, &fx.alice), 900);
    let _ = fs::remove_dir_all(&dir);
}

/// A lost answer leaves the intent recorded with its reservation
/// held; a retry resolves it against the ledger by leg id —
/// finalizing a chain that landed, resubmitting one that did not
/// — and value moves exactly once either way.
#[test]
fn lost_answer_is_recovered_by_leg_id() {
    let mut fx = fixture(LossyLedger::default(), InMemoryIntentStore::new());
    let auth = authorisation(&fx);
    let presented = present(&fx.euro_issuer, &auth, None);

    // The chain lands, the answer is lost.
    fx.svc.backend_mut_for_tests().lose_next = Some(true);
    let error = convert(&mut fx, 9, 1_000, &presented).expect_err("answer lost");
    assert!(matches!(error, ServiceError::Backend(_)), "{error:?}");
    assert_eq!(fx.svc.conversion_usage(&lp(&fx), 7), 1_000);
    let report = fx.svc.reconcile_intents(NOW).expect("reconcile runs");
    assert!(
        report
            .findings
            .iter()
            .any(|finding| matches!(finding, IntentFinding::UnresolvedIntent { op, .. } if op == "convert")),
        "findings: {:?}",
        report.findings
    );
    let receipt = convert(&mut fx, 9, 1_000, &presented).expect("recovery finalizes");
    assert!(verify_receipt(&fx.gateway_vk, &receipt.signed_receipt));
    assert_eq!(
        receipt.conversion_receipt.outcome,
        ConversionOutcome::Posted
    );
    assert_eq!(balance(&fx, &fx.euro, &fx.alice), 900);
    assert_eq!(balance(&fx, &fx.dollar, &fx.alice), 8_990);

    // The chain never lands: recovery resubmits the stored chain.
    fx.svc.backend_mut_for_tests().lose_next = Some(false);
    let error = convert(&mut fx, 10, 500, &presented).expect_err("chain lost");
    assert!(matches!(error, ServiceError::Backend(_)), "{error:?}");
    assert_eq!(balance(&fx, &fx.euro, &fx.alice), 900);
    convert(&mut fx, 10, 500, &presented).expect("recovery resubmits");
    assert_eq!(balance(&fx, &fx.euro, &fx.alice), 1_350);
    assert_eq!(balance(&fx, &fx.dollar, &fx.alice), 8_485);
    assert_eq!(fx.svc.conversion_usage(&lp(&fx), 7), 1_500);
    let report = fx.svc.reconcile_intents(NOW).expect("reconcile runs");
    assert!(report.is_clean(), "findings: {:?}", report.findings);
}

/// Phase G oracle (#15830, design #2063 invariant 11): a posted
/// conversion's `ConversionReceipt` carries everything the
/// receipt holder needs to audit it to every ledger leg — the
/// exact offer version, the quote it executed under, all three
/// leg transfer ids with roles, ledgers and amounts, both
/// currencies, the three amounts, and the outcome — while the
/// shared signed `Receipt` it accompanies is unchanged.
#[test]
fn conversion_receipt_carries_the_offer_legs_and_outcome() {
    let mut fx = fixture(InMemoryLedger::default(), InMemoryIntentStore::new());
    let auth = authorisation(&fx);
    let presented = present(&fx.euro_issuer, &auth, None);
    let request = convert_request(&fx, 20, 1_000);
    let device_key = fx.alice.device_key.clone();
    let (signed, body) = sign(&mut fx, &device_key, &request);
    fx.svc
        .quote_convert(&signed, &body, NOW, &presented, 42)
        .expect("quote issues");
    let quoted = present(&fx.euro_issuer, &auth, Some(42));
    let signed_receipt = convert(&mut fx, 22, 1_000, &quoted).expect("quoted conversion posts");
    assert!(verify_receipt(&fx.gateway_vk, &signed_receipt));

    let intents = fx.svc.intents().all().expect("intents read");
    let intent = intents
        .iter()
        .find(|intent| intent.op == "convert")
        .expect("a convert intent");
    let receipt = fx
        .svc
        .conversion_receipt(intent.idempotency)
        .expect("receipt reads")
        .expect("a conversion receipt");

    assert_eq!(receipt.conversion_id, intent.idempotency);
    assert_eq!(receipt.lp, hex_encode(&lp(&fx)));
    assert_eq!(receipt.authorisation_id, 7);
    assert_eq!(receipt.authorisation_version, 1);
    assert_eq!(receipt.quote_id, Some(42));
    assert_eq!(
        receipt.legs.iter().map(|leg| leg.role).collect::<Vec<_>>(),
        vec![
            ConversionLegRole::Source,
            ConversionLegRole::Fee,
            ConversionLegRole::Destination
        ]
    );
    assert_eq!(
        receipt
            .legs
            .iter()
            .map(|leg| leg.transfer_id)
            .collect::<Vec<_>>(),
        leg_ids(intent.idempotency, 3)
    );
    assert_eq!(
        receipt
            .legs
            .iter()
            .map(|leg| leg.ledger_id)
            .collect::<Vec<_>>(),
        vec![fx.dollar.ledger, fx.dollar.ledger, fx.euro.ledger]
    );
    assert_eq!(
        receipt
            .legs
            .iter()
            .map(|leg| leg.amount)
            .collect::<Vec<_>>(),
        vec![1_000, 10, 900]
    );
    assert_eq!(receipt.source_currency_id, intent.currency_id);
    assert_eq!(
        receipt.source_currency_key,
        Some(hex_encode(&fx.dollar.key))
    );
    assert_eq!(receipt.source_ledger_id, Some(fx.dollar.ledger));
    assert_eq!(receipt.dest_currency_key, hex_encode(&fx.euro.key));
    assert_eq!(receipt.dest_ledger_id, fx.euro.ledger);
    assert_eq!(
        (receipt.amount_in, receipt.fee, receipt.amount_out),
        (1_000, 10, 900)
    );
    assert_eq!(receipt.outcome, ConversionOutcome::Posted);
    assert_eq!(receipt.reversal_of, None);

    // No intent under an id is no conversion receipt.
    assert_eq!(fx.svc.conversion_receipt(424_242).expect("reads"), None);
}

/// Phase G oracle (#15830, design #2063 invariant 10): a
/// conversion executed as a reversal is a separate conversion
/// whose receipt names the original conversion id; the
/// original's receipt names nothing.
#[test]
fn reversal_receipt_names_the_original_conversion() {
    let mut fx = fixture(InMemoryLedger::default(), InMemoryIntentStore::new());
    let auth = authorisation(&fx);
    let presented = present(&fx.euro_issuer, &auth, None);
    convert(&mut fx, 9, 1_000, &presented).expect("original posts");
    let original_id = fx
        .svc
        .intents()
        .all()
        .expect("read")
        .into_iter()
        .find(|intent| intent.op == "convert")
        .expect("original intent")
        .idempotency;

    // The reversal: 500 of Alice's 900 EURO back to DOLR at par,
    // no fee, under the LP's reverse-direction authorisation.
    let mut reverse = auth.clone();
    reverse.authorisation_id = 8;
    reverse.source_ledger = auth.dest_ledger.clone();
    reverse.dest_ledger = auth.source_ledger.clone();
    reverse.rate_num = 1;
    reverse.rate_den = 1;
    reverse.fee_bps = None;
    let mut reversal = present(&fx.euro_issuer, &reverse, None);
    reversal.reversal_of = Some(original_id);
    let request = CurrencyRequest {
        client_request_id: 10,
        op: CurrencyOp::Convert(GwConvert {
            source: fx.euro.key,
            dest: fx.dollar.key,
            from: fx.alice.account.clone(),
            amount: 500,
        }),
    };
    let device_key = fx.alice.device_key.clone();
    let (signed, body) = sign(&mut fx, &device_key, &request);
    let response = fx
        .svc
        .handle_convert(&signed, &body, NOW, &reversal)
        .expect("reversal posts");
    assert_eq!(response.conversion_receipt.reversal_of, Some(original_id));
    assert_eq!(
        response.conversion_receipt.outcome,
        ConversionOutcome::Posted
    );
    assert_eq!(balance(&fx, &fx.dollar, &fx.alice), 8_990 + 500);
    assert_eq!(balance(&fx, &fx.euro, &fx.alice), 400);

    let intents = fx.svc.intents().all().expect("read");
    let reversal_intent = intents
        .iter()
        .find(|intent| {
            intent
                .conversion
                .as_ref()
                .is_some_and(|record| record.reversal_of == Some(original_id))
        })
        .expect("the reversal's intent carries the reference");
    let receipt = fx
        .svc
        .conversion_receipt(reversal_intent.idempotency)
        .expect("reads")
        .expect("a conversion receipt");
    assert_eq!(receipt.reversal_of, Some(original_id));
    assert_eq!(receipt.authorisation_id, 8);
    assert_eq!(
        receipt.legs.iter().map(|leg| leg.role).collect::<Vec<_>>(),
        vec![ConversionLegRole::Source, ConversionLegRole::Destination]
    );
    assert_eq!(
        (receipt.amount_in, receipt.fee, receipt.amount_out),
        (500, 0, 500)
    );
    assert_eq!(receipt.outcome, ConversionOutcome::Posted);

    let original = fx
        .svc
        .conversion_receipt(original_id)
        .expect("reads")
        .expect("the original's receipt");
    assert_eq!(original.reversal_of, None);
}

/// Phase G oracle (#15830): the receipt of a conversion the
/// cluster refused reports the failed outcome with the recorded
/// terms — the audit read does not hide refusals.
#[test]
fn failed_conversion_receipt_reports_the_failed_outcome() {
    let mut fx = fixture(InMemoryLedger::default(), InMemoryIntentStore::new());
    let mut auth = authorisation(&fx);
    auth.max_amount = 50_000;
    auth.total_cap = 100_000;
    let presented = present(&fx.euro_issuer, &auth, None);
    convert(&mut fx, 9, 6_000, &presented).expect_err("inventory is short");

    let intents = fx.svc.intents().all().expect("read");
    let intent = intents
        .iter()
        .find(|intent| intent.op == "convert")
        .expect("a convert intent");
    assert_eq!(intent.state, IntentState::Failed);
    let receipt = fx
        .svc
        .conversion_receipt(intent.idempotency)
        .expect("reads")
        .expect("a conversion receipt");
    assert_eq!(receipt.outcome, ConversionOutcome::Failed);
    assert_eq!(
        (receipt.amount_in, receipt.fee, receipt.amount_out),
        (6_000, 60, 5_400)
    );
    assert_eq!(receipt.legs.len(), 3);
}

/// A recorded conversion whose leg id holds other contents on the
/// ledger is a conflict: surfaced, never overwritten, and the
/// intent stays recorded for the operator.
#[test]
fn foreign_leg_contents_quarantine_the_conversion() {
    let mut fx = fixture(LossyLedger::default(), InMemoryIntentStore::new());
    let auth = authorisation(&fx);
    let presented = present(&fx.euro_issuer, &auth, None);
    fx.svc.backend_mut_for_tests().lose_next = Some(false);
    convert(&mut fx, 9, 1_000, &presented).expect_err("chain lost");
    let intent = fx
        .svc
        .intents()
        .all()
        .expect("read")
        .into_iter()
        .find(|intent| intent.op == "convert")
        .expect("recorded conversion");
    // Another writer posts a transfer under the source leg's id.
    let mut foreign = intent.legs[0].clone();
    foreign.linked = false;
    foreign.amount = 1;
    assert_eq!(
        fx.svc.backend_mut_for_tests().create_transfers(&[foreign]),
        vec![Ok(())]
    );
    let error = convert(&mut fx, 9, 1_000, &presented).expect_err("conflict");
    assert!(
        matches!(error, ServiceError::IntentConflict(_)),
        "{error:?}"
    );
    let still = fx
        .svc
        .intents()
        .get(intent.idempotency)
        .expect("read")
        .expect("present");
    assert_eq!(still.state, IntentState::Recorded);
}
