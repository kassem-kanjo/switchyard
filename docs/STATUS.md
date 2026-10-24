# Switchyard: status

**Updated:** 2026-10-24 · **Phase:** V1 released → public launch → V2

## Now
- **V1 released** (`v1.0.0`). The paper is merged (`docs/paper/`). README rewritten for the public repo, with a hero GIF, the 52 s video (`docs/media/switchyard-demo.mp4`), results and a build history. The release workflow attaches prebuilt CLI binaries.
- The launch kit (X thread, LinkedIn, HN, cards, video) is outside the repo in `../switchyard-launch/`.
- **Waiting on Kassem:** make the repo public, then post.

## Next (milestone V2: design, UX, sellability, launch)
1. Go public and launch (#23).
2. V2 CP1: #24 headline card and folded report, #22 jargon, visual pass. CP2: polish the demo (#10) and README (#9).

## How we build (CEO rule, 2026-10-22)
Flash-tier OpenCode workers code -> Sonnet leads drive and review them -> Opus reviews the leads -> the Overseer re-verifies. At most two heavy workers at once (the Go plan has a 5-hour cap). Worker worktrees go outside %TEMP% (the path is in machine.env) to avoid LNK1104. Briefs: `~/.claude/skills/studio` section 2.

## Blockers
- None.

## Key findings
- Most fault damage spreads as congestion; replay attribution captures it.
- A robot dying on an induction cell costs ~10× more (now cleared after 5 min, per review).
