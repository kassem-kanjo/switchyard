# Switchyard: status

**Updated:** 2026-10-22 (08:30) · **Phase:** V1 released → V2 (design and launch)

## Now
- **V1 tagged `v1.0.0`** (CEO review package: `docs/reviews/v1.md`). CP1-CP3 merged, the V1 depth work (#25, #18) and the security fixes (#11, #28) merged.
- **Launch prep done (2026-10-23):** 3-page paper `docs/paper/switchyard-v1.pdf` (PR #30, awaiting Kassem's read). Launch kit outside the repo in `../switchyard-launch/` (LAUNCH.md with the X thread, LinkedIn post and HN copy; 4 cards; demo.mp4 47 s; demo.gif; 3 stills).
- Waiting on the CEO: approve the paper, the licence (MIT or source-available), then flip the repo public and post.

## Next (milestone V2: design, UX, sellability, launch)
1. #26 first thought piece (with Kassem).
2. V2 CP1: #24 headline card + folded report, #22 jargon, visual pass. CP2: #9 GIF, #10 video. CP3: #23 launch kit; go public.

## How we build (CEO rule, 2026-10-22)
Flash-tier OpenCode workers code -> Sonnet leads drive and review them -> Opus reviews the leads -> the Overseer re-verifies. At most two heavy workers at once (the Go plan has a 5-hour cap). Worker worktrees go outside %TEMP% (the path is in machine.env) to avoid LNK1104. Briefs: `~/.claude/skills/studio` section 2.

## Blockers
- None.

## Key findings
- Most fault damage spreads as congestion; replay attribution captures it.
- A robot dying on an induction cell costs ~10× more (now cleared after 5 min, per review).
