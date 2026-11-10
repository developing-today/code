//! Spike S5: canonical CBOR for signed bytes.
//!
//! `canonical` is the candidate wrapper. The other modules support the measurements:
//! `sample` builds seeded values and mutations, `matrix` runs the rejection matrix, `fuzz` runs
//! the decode-then-re-encode check, and `golden` builds the golden vectors and the forgeries.

pub mod canonical;
pub mod fuzz;
pub mod golden;
pub mod matrix;
pub mod sample;
