//! Built-in scenario presets.
//!
//! The TOML files in `scenarios/` are the source of truth; they are embedded
//! here so the WASM build and tests can load them without file I/O (the core
//! does no I/O, ADR 0002).

/// The `small` hub preset (`scenarios/small.toml`).
pub const SMALL_TOML: &str = include_str!("../../../scenarios/small.toml");
