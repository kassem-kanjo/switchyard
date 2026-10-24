# Switchyard: status

**Updated:** 2026-10-24 · **Phase:** V1 released → public launch → V2

## Now
- **V1 released** (`v1.0.0`). The paper is merged (`docs/paper/`). README rewritten for the public repo, with a hero GIF, the 52 s video (`docs/media/switchyard-demo.mp4`), results and a build history. The release workflow attaches prebuilt CLI binaries.
- The launch kit (X thread, LinkedIn, HN, cards, video) is outside the repo in `../switchyard-launch/`.
- **Repo is public** (2026-10-24). Posting waits on Kassem's go.

## Next: V2 (planned 2026-10-24; plan `docs/plans/v2.md`, design `docs/design/v2.md`, ADR 0007)
1. **CP1, a report you act on in 10 s:** built on branch `feat/v2-cp1` (#24 headline card and folded report, #22 plain wording); awaiting the Opus review, then the PR. Screenshot: `docs/checkpoints/img/v2-cp1-headline.png`.
2. **CP2, credible and one command:** #32 design tokens, #33 incident on the floor, #34 first-run path, #35 `switchyard view`.
3. **CP3, launch:** #9 README refresh, #10 video re-render, #23 launch, #36 outreach kit, #26 piece (optional).
Each issue names its OpenCode worker pieces (W1-W10); Sonnet leads integrate; one Opus review per CP.

## How we build (CEO rule, 2026-10-22)
Flash-tier OpenCode workers code -> Sonnet leads drive and review them -> Opus reviews the leads -> the Overseer re-verifies. At most two heavy workers at once (the Go plan has a 5-hour cap). Worker worktrees go outside %TEMP% (the path is in machine.env) to avoid LNK1104. Briefs: `~/.claude/skills/studio` section 2.

## Blockers
- None.

## Key findings
- Most fault damage spreads as congestion; replay attribution captures it.
- A robot dying on an induction cell costs ~10× more (now cleared after 5 min, per review).
