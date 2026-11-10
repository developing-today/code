//! Plaza guest SDK for Rust (F12.8, issue #1065).
//!
//! The Roc platform (`examples/roc-world`) turns a pure Roc program into
//! an `id` world module through a Zig host adapter. This crate is the
//! Rust equal: implement the [`World`] trait and invoke
//! [`plaza_guest!`] once, and the crate produces an import-free
//! Wasm32 module exporting the full Plaza ABI v1 surface documented in
//! `examples/roc-world/ABI.md` — memory, alloc/free, init/update, view,
//! layout, records, wants, snapshot/restore, the shared output slot,
//! and the diagnostic pair.
//!
//! Helper modules mirror the Roc platform libraries so a Rust guest
//! reads like its Roc counterpart:
//!
//! - [`screen`] = `Screen.roc` (portable screen JSON + plain text)
//! - [`json`] = `RocJson.roc` (bounded decode, lexeme-preserving numbers)
//! - [`grid`] = `RocGrid.roc` (total, bounds-checked row-major grids)
//! - [`cap`] = `RocCap.roc` (capability request sessions)
//! - [`text`] = `RocText.roc` (total byte-level text helpers)
//!
//! `examples/rust-counter` is the playable example built on this SDK.

pub mod cap;
pub mod grid;
pub mod json;
pub mod screen;
pub mod text;

use std::alloc::{alloc, dealloc, Layout};
use std::cell::UnsafeCell;

/// A Plaza world program. `Model` is opaque to the host: the ABI hands
/// the host a pointer it passes back to later calls.
///
/// Only `init`, `update`, and `view` are required; the optional
/// projections default to the Roc platform's absent-export behaviour,
/// except `wants`, which defaults to the empty document `"{}"` that
/// Roc programs return when they want nothing.
pub trait World {
    type Model;

    fn init(seed: u64) -> Self::Model;
    fn update(model: Self::Model, participant: u32, event: &[u8]) -> Self::Model;
    fn view(model: &Self::Model) -> Vec<u8>;

    fn layout(model: &Self::Model, _width: u64, _height: u64) -> Option<Vec<u8>> {
        let _ = model;
        None
    }

    fn records(model: &Self::Model) -> Option<Vec<u8>> {
        let _ = model;
        None
    }

    fn wants(model: &Self::Model) -> Option<Vec<u8>> {
        let _ = model;
        Some(b"{}".to_vec())
    }

    fn snapshot(model: &Self::Model) -> Option<Vec<u8>> {
        let _ = model;
        None
    }

    /// Reconstruct a model from `snapshot` bytes. Must be total: for
    /// malformed or unsupported text return a sensible model rather
    /// than trapping (the ABI's restore rule).
    fn restore(snapshot: &[u8]) -> Self::Model {
        let _ = snapshot;
        Self::init(0)
    }
}

// ---- ABI plumbing -------------------------------------------------------

struct OutputSlot(UnsafeCell<Vec<u8>>);
unsafe impl Sync for OutputSlot {}
static OUTPUT: OutputSlot = OutputSlot(UnsafeCell::new(Vec::new()));

fn set_output(bytes: Vec<u8>) -> i32 {
    unsafe {
        let slot = &mut *OUTPUT.0.get();
        *slot = bytes;
        slot.as_ptr() as i32
    }
}

fn output_len() -> i32 {
    unsafe { (&*OUTPUT.0.get()).len() as i32 }
}

fn output_ptr() -> i32 {
    unsafe { (&*OUTPUT.0.get()).as_ptr() as i32 }
}

fn guest_alloc(len: i32) -> i32 {
    let len = usize::try_from(len).unwrap_or(0).max(1);
    let layout = Layout::from_size_align(len, 1).unwrap();
    unsafe { alloc(layout) as i32 }
}

fn guest_free(ptr: i32, len: i32) {
    if ptr == 0 {
        return;
    }
    let len = usize::try_from(len).unwrap_or(0).max(1);
    let layout = Layout::from_size_align(len, 1).unwrap();
    unsafe { dealloc(ptr as *mut u8, layout) }
}

fn leak_model<M>(model: M) -> i32 {
    Box::into_raw(Box::new(model)) as i32
}

unsafe fn take_model<M>(ptr: i32) -> M {
    *Box::from_raw(ptr as *mut M)
}

unsafe fn ref_model<'a, M>(ptr: i32) -> &'a M {
    &*(ptr as *const M)
}

unsafe fn event_bytes<'a>(ptr: i32, len: i32) -> &'a [u8] {
    let len = usize::try_from(len).unwrap_or(0);
    if len == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr as *const u8, len)
    }
}

/// Generate the 14 documented Plaza ABI v1 exports for a [`World`]
/// implementation. Invoke once at the crate root of a `cdylib` crate:
///
/// ```ignore
/// struct MyWorld;
/// impl plaza_sdk::World for MyWorld { /* ... */ }
/// plaza_sdk::plaza_guest!(MyWorld);
/// ```
///
/// ABI.md makes `plaza_snapshot`/`plaza_restore` an all-or-nothing
/// pair and the conformance suite requires every documented export,
/// so the macro always exports both: a `World` whose `snapshot`
/// returns `None` gets a snapshot of the empty byte string and a
/// restore through `World::restore`, keeping the pair total.
#[macro_export]
macro_rules! plaza_guest {
    ($world:ty) => {
        #[no_mangle]
        pub extern "C" fn plaza_abi_version() -> i32 {
            1
        }

        #[no_mangle]
        pub extern "C" fn plaza_alloc(len: i32) -> i32 {
            $crate::plaza_alloc_impl(len)
        }

        #[no_mangle]
        pub extern "C" fn plaza_free(ptr: i32, len: i32) {
            $crate::plaza_free_impl(ptr, len)
        }

        #[no_mangle]
        pub extern "C" fn plaza_init(seed: i64) -> i32 {
            $crate::plaza_init_impl::<$world>(seed)
        }

        #[no_mangle]
        pub extern "C" fn plaza_update(
            model: i32,
            participant: i32,
            event_ptr: i32,
            event_len: i32,
        ) -> i32 {
            $crate::plaza_update_impl::<$world>(model, participant, event_ptr, event_len)
        }

        #[no_mangle]
        pub extern "C" fn plaza_view(model: i32) -> i32 {
            $crate::plaza_view_impl::<$world>(model)
        }

        #[no_mangle]
        pub extern "C" fn plaza_layout(model: i32, width: i64, height: i64) -> i32 {
            $crate::plaza_layout_impl::<$world>(model, width, height)
        }

        #[no_mangle]
        pub extern "C" fn plaza_records(model: i32) -> i32 {
            $crate::plaza_records_impl::<$world>(model)
        }

        #[no_mangle]
        pub extern "C" fn plaza_wants(model: i32) -> i32 {
            $crate::plaza_wants_impl::<$world>(model)
        }

        #[no_mangle]
        pub extern "C" fn plaza_snapshot(model: i32) -> i32 {
            $crate::plaza_snapshot_impl::<$world>(model)
        }

        #[no_mangle]
        pub extern "C" fn plaza_restore(snapshot_ptr: i32, snapshot_len: i32) -> i32 {
            $crate::plaza_restore_impl::<$world>(snapshot_ptr, snapshot_len)
        }

        #[no_mangle]
        pub extern "C" fn plaza_out_len() -> i32 {
            $crate::plaza_out_len_impl()
        }

        #[no_mangle]
        pub extern "C" fn plaza_error_ptr() -> i32 {
            $crate::plaza_error_ptr_impl()
        }

        #[no_mangle]
        pub extern "C" fn plaza_error_len() -> i32 {
            0
        }
    };
}

#[doc(hidden)]
pub fn plaza_alloc_impl(len: i32) -> i32 {
    guest_alloc(len)
}

#[doc(hidden)]
pub fn plaza_free_impl(ptr: i32, len: i32) {
    guest_free(ptr, len)
}

#[doc(hidden)]
pub fn plaza_init_impl<W: World>(seed: i64) -> i32 {
    leak_model(W::init(seed as u64))
}

#[doc(hidden)]
pub fn plaza_update_impl<W: World>(
    model: i32,
    participant: i32,
    event_ptr: i32,
    event_len: i32,
) -> i32 {
    unsafe {
        let model = take_model::<W::Model>(model);
        let event = event_bytes(event_ptr, event_len).to_vec();
        leak_model(W::update(model, participant as u32, &event))
    }
}

#[doc(hidden)]
pub fn plaza_view_impl<W: World>(model: i32) -> i32 {
    unsafe { set_output(W::view(ref_model::<W::Model>(model))) }
}

#[doc(hidden)]
pub fn plaza_layout_impl<W: World>(model: i32, width: i64, height: i64) -> i32 {
    unsafe {
        let model_ref = ref_model::<W::Model>(model);
        let bytes = W::layout(model_ref, width as u64, height as u64)
            .unwrap_or_else(|| W::view(model_ref));
        set_output(bytes)
    }
}

#[doc(hidden)]
pub fn plaza_records_impl<W: World>(model: i32) -> i32 {
    unsafe {
        let bytes = W::records(ref_model::<W::Model>(model)).unwrap_or_default();
        set_output(bytes)
    }
}

#[doc(hidden)]
pub fn plaza_wants_impl<W: World>(model: i32) -> i32 {
    unsafe {
        let bytes = W::wants(ref_model::<W::Model>(model)).unwrap_or_default();
        set_output(bytes)
    }
}

#[doc(hidden)]
pub fn plaza_snapshot_impl<W: World>(model: i32) -> i32 {
    unsafe {
        let bytes = W::snapshot(ref_model::<W::Model>(model)).unwrap_or_default();
        set_output(bytes)
    }
}

#[doc(hidden)]
pub fn plaza_restore_impl<W: World>(snapshot_ptr: i32, snapshot_len: i32) -> i32 {
    unsafe {
        let bytes = event_bytes(snapshot_ptr, snapshot_len).to_vec();
        leak_model(W::restore(&bytes))
    }
}

#[doc(hidden)]
pub fn plaza_out_len_impl() -> i32 {
    output_len()
}

#[doc(hidden)]
pub fn plaza_error_ptr_impl() -> i32 {
    // No crash has been captured: the diagnostic pair reads as an
    // empty message (length 0), which ABI.md permits at rest.
    output_ptr()
}
