# Switchyard: status

**Updated:** 2026-10-22 (02:35) · **Phase:** CP3 built → CP3 gates → V1 release

## Now
- `v0.1.0`; CP1 merged (#3). **CP2** PR #4: all review findings fixed; CI green except one Windows test job pending, so merge when green.
- **CP3** PR #5 (stacked on #4): post-mortem engine (CUSUM, dithered checkpoint replays, exact Shapley, fixes, SRE report, `switchyard postmortem`). 24/24 planted incidents; 96 replays in 2.3 s. Auditor review in progress (issue #6).

## Next (GitHub milestone V1: issues #6–#12)
1. #6 CP3 gates (Auditor + `/code-review high`, visual, CI) → merge #4 then #5.
2. #7 wire the viewer's post-mortem panel to real reports · #8 harder planted suite.
3. #9 README numbers and GIF · #10 demo video · #11 `/security-review` · #12 tag `v1.0.0` and CEO review.

## Blockers
- **OpenCode Go 5-hour quota hit** (2026-10-22 ~02:20; resets ~06:42 EDT). Four workers on #14-#17 stopped after ~6 min with no usable output (only #15 left partial, untested edits in its worktree). Re-dispatch the same briefs after the reset, **one or two at a time**, not four.
- Parallel cargo builds under the Temp scratchpad hit `LNK1104` (file locked, likely antivirus on fresh build-script .exe). Put worker worktrees outside `%TEMP%` or share one `CARGO_TARGET_DIR` per worker run sequentially.

## Key findings
- Most fault damage spreads as congestion; replay attribution captures it.
- A robot dying on an induction cell costs ~10× more (now cleared after 5 min, per review).
