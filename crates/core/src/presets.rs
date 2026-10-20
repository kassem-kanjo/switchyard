//! Built-in scenario presets.
//!
//! The TOML files live in `crates/core/scenarios/`, inside this crate, so
//! `include_str!` works from a packaged crate too (a path outside the crate
//! would break `cargo package` and builds from crates.io). They are embedded
//! so the WASM build and tests can load them without file I/O (the core does
//! no I/O, ADR 0002). The repository's top-level `scenarios/` directory holds
//! identical copies for command-line use; a CLI test keeps them in sync.

/// The `small` hub preset: 40 robots, 4 stations, 90 chutes.
pub const SMALL_TOML: &str = include_str!("../scenarios/small.toml");

/// The `medium` hub preset: 200 robots, 8 stations, 300 chutes.
pub const MEDIUM_TOML: &str = include_str!("../scenarios/medium.toml");

/// The `large` hub preset: 500 robots, 20 stations, 588 chutes.
pub const LARGE_TOML: &str = include_str!("../scenarios/large.toml");

/// Every preset as `(name, toml)`, smallest first.
pub const ALL: &[(&str, &str)] = &[
    ("small", SMALL_TOML),
    ("medium", MEDIUM_TOML),
    ("large", LARGE_TOML),
];

/// The TOML text of the preset called `name`, if there is one.
#[must_use]
pub fn by_name(name: &str) -> Option<&'static str> {
    ALL.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}
