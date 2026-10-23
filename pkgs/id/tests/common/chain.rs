//! The event decoder's contract, shared by the property tests and the regression
//! replay. The checks are the same for every byte string, so a case found by a
//! property run can be replayed unchanged.

use id::chain::decode;
use p2panda_core::Hash;

/// Checks the decoder's contract for one byte string.
///
/// The decoder must not panic: a panic fails the caller. It may refuse the
/// bytes. If it accepts them, the accepted event must hold up: its operation ID
/// is BLAKE3 over the bytes, its extension passes the validation rules, and
/// decoding the same bytes again gives the same event.
///
/// # Errors
///
/// Describes the first part of the contract that does not hold.
pub fn check_event_bytes(bytes: &[u8]) -> Result<(), String> {
    let Ok(event) = decode(bytes) else {
        return Ok(());
    };
    if event.id != Hash::digest(bytes) {
        return Err(format!("the id of {bytes:?} is not BLAKE3 over the bytes"));
    }
    event
        .header
        .extensions
        .validate()
        .map_err(|err| format!("accepted {bytes:?}, but its extension fails: {err}"))?;
    let again =
        decode(&event.bytes).map_err(|err| format!("decoding {bytes:?} again failed: {err}"))?;
    if again != event {
        return Err(format!("decoding {bytes:?} again gave a different event"));
    }
    Ok(())
}
