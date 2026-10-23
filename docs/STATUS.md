# Switchyard: status

**Updated:** 2026-10-22 (08:30) · **Phase:** CP3 merged → CEO check → V1 release

## Now
- **CP1-CP3 merged into main** (PR #4 CP2, PR #5 CP3). CP3.2 done: viewer post-mortem panel (#7), honest harder planted suite (#8: ranking 44/48, interaction 37/48), Auditor fixes #19-#21. Overseer re-ran: fmt/clippy/cargo test 0, viewer typecheck/test 58/58/build 0; PR CI 11/11 green.
- **Next: CEO check of CP3.**
- Node 16 is first on PATH and breaks vite; use Node 22 (`$NODE_DIR` in machine.env).

## Next (milestone V1)
1. CEO check of CP3 (screenshot: `docs/checkpoints/img/viewer-postmortem.png`; reports via `switchyard postmortem`).
2. #22 cosmetic report fixes · #18 per-mask bands (optional).
3. #9 README numbers and GIF · #10 demo video · #23 launch kit and first thought piece (`docs/insights.md`) · #11 `/security-review` · #12 tag `v1.0.0` and CEO review. Repo goes public at launch (licence: Kassem to choose MIT or source-available).

## How we build (CEO rule, 2026-10-22)
Flash-tier OpenCode workers code -> Sonnet leads drive and review them -> Opus reviews the leads -> the Overseer re-verifies. At most two heavy workers at once (the Go plan has a 5-hour cap). Worktrees outside %TEMP% (`C:/Users/kassem/sy-work/`) to avoid LNK1104. Briefs: `~/.claude/skills/studio` section 2.

## Blockers
- None.

## Key findings
- Most fault damage spreads as congestion; replay attribution captures it.
- A robot dying on an induction cell costs ~10× more (now cleared after 5 min, per review).
