//! A group member with its own replica. Shared by the concurrency, secrecy and join experiments.

use crate::common::{block, Member};
use beekem::{
    cgka::Cgka,
    error::CgkaError,
    id::{MemberId, TreeId},
    operation::CgkaOperation,
    pcs_key::PcsKey,
};
use future_form::Local;
use keyhive_crypto::{
    share_key::{ShareKey, ShareSecretKey},
    signed::Signed,
    signer::memory::MemorySigner,
    verifiable::Verifiable,
};
use rand::rngs::StdRng;
use std::sync::Arc;

pub type Op = Arc<Signed<CgkaOperation>>;

pub struct Party {
    pub m: Member,
    pub c: Cgka,
    /// Ops received before their predecessors. Only used when the harness buffers.
    pub buffer: Vec<Op>,
}

impl Party {
    pub fn new(doc_id: TreeId, m: Member) -> Self {
        let c = Cgka::new(doc_id, m.id, m.keys());
        Self { m, c, buffer: Vec::new() }
    }

    pub fn id(&self) -> MemberId {
        self.m.id
    }

    /// Rotate this member's leaf key and encrypt its path.
    pub fn update(&mut self, rng: &mut StdRng) -> Result<Op, CgkaError> {
        let sk = ShareSecretKey::generate(rng);
        let (_, op, _) = block(self.c.update::<Local, _, StdRng>(sk.share_key(), sk, &self.m.signer, rng))?;
        Ok(Arc::new(op))
    }

    pub fn add(&mut self, id: MemberId, pk: ShareKey) -> Result<Option<Op>, CgkaError> {
        Ok(block(self.c.add::<Local, _>(id, pk, &self.m.signer))?.map(Arc::new))
    }

    pub fn remove(&mut self, id: MemberId) -> Result<Option<Op>, CgkaError> {
        Ok(block(self.c.remove::<Local, _>(id, &self.m.signer))?.map(Arc::new))
    }

    /// Apply a received op. The crate does not verify the signature; callers must.
    pub fn merge(&mut self, op: &Op) -> Result<bool, CgkaError> {
        self.c.merge_concurrent_operation(op.clone())
    }

    /// The root key this replica can derive, or the error it gets.
    pub fn root(&mut self) -> Result<PcsKey, CgkaError> {
        self.c.pcs_key_from_tree_root()
    }
}

/// `n` members in one group. Member 0 adds everyone in order, then settles with one update. Every
/// replica applies each op right away. Returns the parties, the doc id, the settling op and its key.
pub fn build(n: usize, rng: &mut StdRng) -> (Vec<Party>, TreeId, Op, PcsKey) {
    let doc_id = TreeId(MemorySigner::generate(rng).verifying_key());
    let mut ps: Vec<Party> = (0..n).map(|_| Party::new(doc_id, Member::new(rng))).collect();
    for i in 0..n {
        let (id, pk) = (ps[i].id(), ps[i].m.pk);
        let op = ps[0].add(id, pk).expect("add").expect("new member");
        for p in ps.iter_mut().skip(1) {
            p.merge(&op).expect("in-order add");
        }
    }
    let op = ps[0].update(rng).expect("settle");
    for p in ps.iter_mut().skip(1) {
        p.merge(&op).expect("in-order update");
    }
    let key = ps[0].root().expect("settled group has a root");
    (ps, doc_id, op, key)
}
