//! The presets embedded in `switchyard-core` (`crates/core/scenarios/`) are
//! the source of truth; the repository's top-level `scenarios/` directory
//! holds copies for command-line use. This test keeps them identical. It
//! reads files at test time (not with `include_str!`), so it never affects
//! how the crates package or build.

use std::path::Path;

use switchyard_core::presets;

#[test]
fn top_level_scenarios_match_the_embedded_presets() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenarios");
    let hubs = presets::ALL.iter().map(|(n, t)| (format!("{n}.toml"), t));
    let chaos = presets::CHAOS
        .iter()
        .map(|(n, t)| (format!("chaos/{n}.toml"), t));
    for (name, embedded) in hubs.chain(chaos) {
        let path = root.join(&name);
        let copy = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        // Normalise line endings: Git may check files out with CRLF.
        assert_eq!(
            copy.replace("\r\n", "\n"),
            embedded.replace("\r\n", "\n"),
            "{} differs from crates/core/scenarios/{name}",
            path.display()
        );
    }
}
