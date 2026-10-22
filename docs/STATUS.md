# Switchyard: status

**Updated:** 2026-10-22 (08:30) · **Phase:** CP3 built → CP3 gates → V1 release

## Now
- `v0.1.0`; CP1 merged (#3). **CP2** PR #4 green except one pending Windows test job; merge when green.
- **CP3** PR #5 (stacked on #4) at `8181244`: **sub-checkpoint CP3.1 done**, all Auditor findings #13-#17 fixed. Built by flash-tier OpenCode workers plus Sonnet leads, reviewed by Opus (4 extra fixes), gates re-run by the Overseer: fmt 0, clippy 0, `cargo test --workspace` 0, planted suite 24/24.
- Issues #13-#17 still show open on GitHub: closing them needs Kassem's OK (the auto-mode classifier blocks issue writes). Also to file: a follow-up for per-lattice-value noise bands (not blocking V1).

## Next (milestone V1)
1. CP3.2: the rest of #6 (`/code-review high`, visual check of reports, CI), then merge #4, then #5. **Then a CEO check of CP3.**
2. #7 viewer post-mortem panel · #8 harder planted suite.
3. #9 README numbers and GIF · #10 demo video · #11 `/security-review` · #12 tag `v1.0.0` and CEO review.

## How we build (CEO rule, 2026-10-22)
Flash-tier OpenCode workers code -> Sonnet leads drive and review them -> Opus reviews the leads -> the Overseer re-verifies. At most two heavy workers at once (the Go plan has a 5-hour cap). Worktrees outside %TEMP% (`C:/Users/kassem/sy-work/`) to avoid LNK1104. Briefs: `~/.claude/skills/studio` section 2.

## Blockers
- None.

## Key findings
- Most fault damage spreads as congestion; replay attribution captures it.
- A robot dying on an induction cell costs ~10× more (now cleared after 5 min, per review).
