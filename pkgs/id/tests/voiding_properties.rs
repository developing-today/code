//! Authority-based voiding over random scenarios (P2.3).
//!
//! A scenario is a seeded set of signed events: a group create, then group operations (adds and
//! removes of random keys) and content, each naming a random set of earlier events in `after`.
//! Each scenario is delivered to a fresh store in its causal order and in several random arrival
//! orders. Every order must reach the same applied set and roster, and no order may leave an event
//! waiting: a voided event stays stored, so its author's later events still link to it.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use id::chain::{ChainExt, sign};
use id::event_store::{EventStore, Ingest};
use id::membership::AuthId;
use id::membership::body::encode_body;
use id::store_traits::SharedEventStore;
use id::waiting::WaitingLimits;
use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::{Hash, SeqNum, SigningKey};
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;

/// Arrival orders checked per scenario, besides the causal order.
const ORDERS: usize = 4;

/// The authors of a scenario. The first one creates the group.
const WHO: [char; 4] = ['A', 'B', 'C', 'D'];

/// `SplitMix64`. A scenario is a pure function of its seed.
struct Rng(u64);

impl Rng {
    const fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n`.
    fn below(&mut self, n: usize) -> usize {
        let n = u64::try_from(n).expect("a small bound");
        usize::try_from(self.next_u64() % n).expect("an index fits in usize")
    }
}

/// Cases per property. `PROPTEST_CASES` raises it.
fn config() -> ProptestConfig {
    let cases = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(16);
    ProptestConfig {
        cases,
        failure_persistence: None,
        ..ProptestConfig::default()
    }
}

/// One signed event of a scenario, with its membership action when it is a group operation, and the
/// body that carries the action.
#[derive(Clone)]
struct Step {
    id: Hash,
    bytes: Vec<u8>,
    action: Option<GroupAction<AuthId, ()>>,
    body: Option<Vec<u8>>,
}

fn key(who: char) -> SigningKey {
    SigningKey::from_bytes(&[who as u8; 32])
}

fn auth(who: char) -> AuthId {
    AuthId::Key(*key(who).verifying_key().as_bytes())
}

/// A random scenario of 4 to 10 events. Event 0 creates the group with A, B and C as managers.
/// Each later event is signed by a random author, names up to three random earlier events, and is
/// an add, a remove, or content. Each author's events form a chain, so the sequence numbers and
/// backlinks follow from the order of signing.
fn scenario(seed: u64) -> Vec<Step> {
    let mut rng = Rng(seed);
    let size = 4 + rng.below(7);
    let mut steps: Vec<Step> = Vec::new();
    let mut group: Option<Hash> = None;
    let mut last: BTreeMap<char, (SeqNum, Hash)> = BTreeMap::new();
    for index in 0..size {
        let who = if index == 0 {
            'A'
        } else {
            WHO[rng.below(WHO.len())]
        };
        let action = if index == 0 {
            Some(GroupAction::Create {
                initial_members: ['A', 'B', 'C']
                    .iter()
                    .map(|m| (GroupMember::Individual(auth(*m)), Access::manage()))
                    .collect(),
            })
        } else {
            match rng.below(100) {
                0..35 => {
                    let access = if rng.below(2) == 0 {
                        Access::read()
                    } else {
                        Access::manage()
                    };
                    Some(GroupAction::Add {
                        member: GroupMember::Individual(auth(WHO[rng.below(WHO.len())])),
                        access,
                    })
                }
                35..70 => Some(GroupAction::Remove {
                    member: GroupMember::Individual(auth(WHO[rng.below(WHO.len())])),
                }),
                _ => None,
            }
        };

        let mut after = Vec::new();
        let named = if steps.is_empty() { 0 } else { rng.below(4) };
        for _ in 0..named {
            let pick = steps[rng.below(steps.len())].id;
            if !after.contains(&pick) {
                after.push(pick);
            }
        }

        let (seq, backlink) = match last.get(&who) {
            Some((seq, hash)) => (seq + 1, Some(*hash)),
            None => (0, None),
        };
        let ext = ChainExt {
            kind: if action.is_some() {
                "group.op"
            } else {
                "message"
            }
            .to_owned(),
            group: if index == 0 { None } else { group },
            after,
            tag: None,
            document: None,
            removes: None,
            chat: None,
        };
        let body = action.as_ref().map(|action| encode_body(action).unwrap());
        let event = sign(&key(who), seq, backlink, body.as_deref(), ext).unwrap();
        if index == 0 {
            group = Some(event.id);
        }
        last.insert(who, (seq, event.id));
        steps.push(Step {
            id: event.id,
            bytes: event.bytes,
            action,
            body,
        });
    }
    steps
}

/// The result of one arrival order: which events applied, and the group's roster.
#[derive(Debug, PartialEq, Eq)]
struct Final {
    applied: Vec<bool>,
    roster: Vec<AuthId>,
}

/// Delivers the scenario in `order` to a fresh store, handing each action in directly: the scenario
/// path. Every event must be stored and none may be refused or left waiting.
fn deliver(steps: &[Step], order: &[usize]) -> Result<Final, String> {
    let dir = tempfile::tempdir().map_err(|err| err.to_string())?;
    let mut store =
        EventStore::open(dir.path(), WaitingLimits::default(), 0).map_err(|err| err.to_string())?;
    for &index in order {
        let step = &steps[index];
        let outcome = store
            .ingest_with_action(&step.bytes, step.action.as_ref(), 0)
            .map_err(|err| err.to_string())?;
        if matches!(outcome, Ingest::Refused(_)) {
            return Err(format!("event {index} was refused: {outcome:?}"));
        }
    }
    read_final(steps, &store)
}

/// Delivers the scenario in `order` through the production path: each event with its body, to
/// `SharedEventStore::ingest`, which reads the action from the body. The same checks as [`deliver`].
fn deliver_production(steps: &[Step], order: &[usize]) -> Result<Final, String> {
    let dir = tempfile::tempdir().map_err(|err| err.to_string())?;
    let store =
        EventStore::open(dir.path(), WaitingLimits::default(), 0).map_err(|err| err.to_string())?;
    let shared = SharedEventStore::new(store);
    for &index in order {
        let step = &steps[index];
        let outcome = shared
            .ingest(&step.bytes, step.body.as_deref(), 0)
            .map_err(|err| err.to_string())?;
        if matches!(outcome, Ingest::Refused(_)) {
            return Err(format!("event {index} was refused: {outcome:?}"));
        }
    }
    shared
        .read(|store| Ok(read_final(steps, store)))
        .map_err(|err| err.to_string())?
}

/// The applied flag of each step, and the roster, once every step is in. No event may be left
/// waiting, and every event must be stored.
fn read_final(steps: &[Step], store: &EventStore) -> Result<Final, String> {
    let waiting = store.waiting_ids().map_err(|err| err.to_string())?;
    if !waiting.is_empty() {
        return Err(format!("{} events are left waiting", waiting.len()));
    }
    for (index, step) in steps.iter().enumerate() {
        let stored = store
            .event_by_id(&step.id)
            .map_err(|err| err.to_string())?
            .is_some();
        if !stored {
            return Err(format!("event {index} was never stored"));
        }
    }
    let mut applied = Vec::new();
    for step in steps {
        applied.push(
            store
                .event_applied(&step.id)
                .map_err(|err| err.to_string())?,
        );
    }
    let mut roster: Vec<AuthId> = store
        .roster(&steps[0].id)
        .map_err(|err| err.to_string())?
        .into_iter()
        .map(AuthId::Key)
        .collect();
    roster.sort();
    Ok(Final { applied, roster })
}

/// A random permutation of `0..n`, by Fisher-Yates.
fn shuffled(n: usize, rng: &mut Rng) -> Vec<usize> {
    let mut order: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        order.swap(i, rng.below(i + 1));
    }
    order
}

proptest! {
    #![proptest_config(config())]

    /// Random removals, adds and dependent chains give the same applied set and roster in every
    /// arrival order, and no order leaves an event waiting.
    #[test]
    fn test_authority_voiding_is_the_same_for_every_arrival_order(seed in any::<u64>()) {
        let steps = scenario(seed);
        let causal: Vec<usize> = (0..steps.len()).collect();
        let reference = deliver(&steps, &causal).map_err(TestCaseError::fail)?;
        let mut rng = Rng(seed ^ 0xA5A5_5A5A_F00D_BEEF);
        for _ in 0..ORDERS {
            let order = shuffled(steps.len(), &mut rng);
            let outcome = deliver(&steps, &order).map_err(TestCaseError::fail)?;
            prop_assert_eq!(&outcome, &reference, "arrival order {:?}", order);
        }
    }

    /// Through the production path, which reads each action from the event's body, every arrival
    /// order gives the same applied set and roster as the scenario path in causal order.
    #[test]
    fn test_production_ingest_is_the_same_for_every_arrival_order(seed in any::<u64>()) {
        let steps = scenario(seed);
        let causal: Vec<usize> = (0..steps.len()).collect();
        let reference = deliver(&steps, &causal).map_err(TestCaseError::fail)?;
        let mut rng = Rng(seed ^ 0x5EED_1CE0_F00D_CAFE_u64);
        let production = deliver_production(&steps, &causal).map_err(TestCaseError::fail)?;
        prop_assert_eq!(&production, &reference, "production path in causal order");
        for _ in 0..ORDERS {
            let order = shuffled(steps.len(), &mut rng);
            let outcome = deliver_production(&steps, &order).map_err(TestCaseError::fail)?;
            prop_assert_eq!(&outcome, &reference, "production path, arrival order {:?}", order);
        }
    }
}
