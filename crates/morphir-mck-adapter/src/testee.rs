//! The operations that answer each request in the kit's adapter protocol.
//!
//! Stage one (this crate's first slice) only wires up `capabilities` and
//! `exit`, handled directly in `main.rs`. `decode`, `readTree` and
//! `writeTree` land here as later stages give the adapter a codec to call.
