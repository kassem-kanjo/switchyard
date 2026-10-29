# Switchyard: status

**Updated:** 2026-10-29 · **Phase:** V2 CP2 done → CEO check → CP3 launch

## Now
- **V2 CP1 + CP2 merged (2026-10-29):** headline card and plain report (#39), console palette and floor marks (#40), `switchyard view` (#38), floor legibility with 1x = real time (#45), the operations console with rail, inspector and timeline (#46), the first run with a 22 s guided demo (#48), and the Intel-Mac release fix (#47; ships with v2.0.0).
- **Waiting on Kassem:** the CP2 check (screenshots in `docs/checkpoints/img/v2-console-1440.png` and `v2-firstrun-*.png`).

## Next: V2 CP3 (launch)
1. Positioning pass (plan CP3.0): remove slogans and "digital twin" from the README, paper, cards, video and repo description.
2. #9 README refresh with the console UI, #10 re-render the Remotion video with the console, #36 outreach kit, #23 launch.
3. Tag `v2.0.0` (adds the Intel-Mac binary) and the V2 review package.

## How we build (CEO rule, 2026-10-22)
Flash-tier OpenCode workers code -> Sonnet leads drive and review them -> Opus reviews the leads -> the Overseer re-verifies. At most two heavy workers at once (the Go plan has a 5-hour cap). Worker worktrees go outside %TEMP% (the path is in machine.env) to avoid LNK1104. Briefs: `~/.claude/skills/studio` section 2.

## Blockers
- None.

## Key findings
- Most fault damage spreads as congestion; replay attribution captures it.
- A robot dying on an induction cell costs ~10× more (now cleared after 5 min, per review).
