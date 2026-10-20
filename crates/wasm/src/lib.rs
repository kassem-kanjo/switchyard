//! # switchyard-wasm
//!
//! The wasm-bindgen facade the browser viewer loads in a Web Worker
//! (ADR 0001, ARCHITECTURE §8).
//!
//! V0 stub: it exposes only [`version`], which proves the core compiles for
//! `wasm32-unknown-unknown`. Sim creation, stepping and flat snapshot
//! buffers follow once the viewer contract is settled.

// wasm-bindgen's generated glue contains `unsafe` blocks; the workspace
// denies unsafe code everywhere else.
#![allow(unsafe_code)]

use wasm_bindgen::prelude::wasm_bindgen;

/// The version of the simulation core compiled into this module.
#[wasm_bindgen]
#[must_use]
pub fn version() -> String {
    switchyard_core::VERSION.to_owned()
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_matches_core() {
        assert_eq!(super::version(), switchyard_core::VERSION);
    }
}
