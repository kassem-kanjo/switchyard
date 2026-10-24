# 0007. `switchyard view`: the viewer served from the CLI binary

**Status:** proposed · **Date:** 2026-10-24

## Context
The visual viewer is a Vite app that needs Node 22 and `npm ci` to run. That is fine for contributors and a barrier for everyone else: an ops leader or a robotics engineer will download a binary, not set up a JavaScript toolchain. The CEO does not want a hosted web version: the tool should feel like a local engineering tool, and a public site invites comparisons with polished SaaS products.

## Decision
- Add a `switchyard view [--scenario <preset>] [--port <n>]` subcommand.
- The release build embeds the production viewer bundle (`viewer/dist`, including the wasm) in the binary with `include_dir`.
- A minimal HTTP server (`tiny_http`) binds **127.0.0.1 only** and serves those embedded files with correct MIME types (including `application/wasm`).
- The command opens the default browser to `http://127.0.0.1:<port>/?preset=<scenario>`, and also prints the URL.
- The browser is only the display. Nothing is hosted, and nothing leaves the machine.

## Consequences
- One download and one command give the full visual experience, with no Node.
- The binary grows by the bundle size (about 2-4 MB).
- CI must build the viewer before the CLI in the release workflow, and must check that `switchyard view` serves `index.html` and the `.wasm`.
- New dependencies `include_dir` and `tiny_http` are pure Rust and MIT/Apache, so they compile on the GNU toolchain with no C compiler.
- The dev workflow (`npm run dev`) stays unchanged.

## Alternatives considered
- **Hosted demo (GitHub Pages):** rejected by the CEO. It reads as a toy, and it suggests the tool is a web app.
- **Tauri or Electron desktop app:** a heavier build, a platform webview matrix, and code-signing questions. That's more than V2 needs.
- **Native renderer (Bevy, egui):** a full UI rewrite; it may come later behind the existing `Renderer` interface.
