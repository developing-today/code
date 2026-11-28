//! S3 spike: joiner history from the current epoch, and key-agreement ordering for the `id`
//! design. Method and results are in RESULTS.md. Run with `-- base` or `-- patched`.
#![allow(dead_code, unused_imports)]

mod base {
    //! The unmodified p2panda-encryption at the workspace pin, with `MemberDgm` as the membership.
    use p2panda_encryption as enc;

    include!("harness.rs");

    type Dgm = MemberDgm<Id, Mid>;

    /// `from_join` without a fork. This is the local sequence of `EncryptionGroup::add`: add, then
    /// process the add locally. The welcome gets an empty bundle instead of the adder's whole
    /// bundle. `Dcgka::add`, `Dcgka::process` and `Ordering::next_control_message` are public, and
    /// the fields of `GroupState` are public, so no crate change is needed.
    pub fn add_from_join(mut y: Y, added: Id, rng: &enc::Rng) -> (Y, Msg) {
        let (dcgka, output) =
            Dcgka::add(y.dcgka, added, &SecretBundle::init(), rng).expect("add from_join");
        let (orderer, msg) = <Orderer as Ordering<Id, Mid, Dgm>>::next_control_message(
            y.orderer,
            &output.control_message,
            &output.direct_messages,
        )
        .expect("order the add");
        let (dcgka, _) = Dcgka::process(
            dcgka,
            ProcessInput {
                seq: msg.id(),
                sender: msg.sender(),
                control_message: output.control_message,
                direct_message: None,
            },
        )
        .expect("process the add locally");
        y.dcgka = dcgka;
        y.orderer = orderer;
        (y, msg)
    }

    pub fn run() {
        println!("# base: unmodified p2panda-encryption");
        run_base();
    }
}

mod patched {
    //! The path copy at `patched/p2panda-encryption`: `add_from_join`, plus the joiner-membership
    //! fix in `process_add`. The stock `TestDgm` is the membership, so the fix is what is tested.
    use p2panda_encryption_patched as enc;

    include!("harness.rs");

    type Dgm = enc::data_scheme::test_utils::dgm::TestDgm<Id, Mid>;

    pub fn add_from_join(y: Y, added: Id, rng: &enc::Rng) -> (Y, Msg) {
        ok(
            EncryptionGroup::add_from_join(y, added, rng),
            "add_from_join",
        )
    }

    pub fn run() {
        println!("# patched: path copy with add_from_join");
        run_patched();
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("base") => base::run(),
        Some("patched") => patched::run(),
        None => {
            base::run();
            patched::run();
        }
        Some(other) => panic!("unknown mode {other}; use base or patched"),
    }
}
