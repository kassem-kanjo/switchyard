# Post-mortem: two-dead-robots-and-a-jam (seed 1)

**Incident 00:30–00:52 · up to 47 % below the fault-free twin**

The C289 jam caused 70 % of the delay.
Rerouting C289's parcels to C269 would have saved 83 %.

**6,264** parcel-min of delay · **110** parcels late · **83 %** recoverable

*How we know: proven by 96 replays of this shift · error bars from 8 versions of the day.*

## Summary

From 00:30 sorted throughput fell up to 47 % below the fault-free twin. By 01:02 the incident had cost **6264 parcel-minutes** of delay, and 110 parcels were more than 10 min late.
Chute C289 jams (F0) explains 70 % ± 3 % (4329 parcel-min); robot R119 breaks down (F2) explains 15 % ± 1 % (947 parcel-min); robot R57 breaks down (F1) explains 15 % ± 2 % (921 parcel-min).

Best fix: **send chute C289's destination to C269 from the jam on (never undone)** would have recovered 5162 parcel-min (83 % of the loss).

How we know: the real shift and its fault-free twin, plus replays with each cause removed (R0–R7); see the replay index at the end.

<details>
<summary><b>Impact</b> — 110 parcels late · peak backlog 596 (twin 91)</summary>

## Impact

| Measure | Value |
|---|---|
| Delay at 01:02 vs the fault-free twin | 6264 parcel-min (± 356) |
| Parcels more than 10 min late at 01:02 | 110 |
| Parcels more than 10 min late (whole run, to 02:00) | 110 |
| Peak backlog | 596 parcels (twin 91) |
| Worst minute / time to recover | 47 % below the twin / 15 min |
| Stations / chutes out of action at once | 2 / 1 |
| Parcels left unsorted at the end of the run | 7 |

How we know: the real shift compared with its fault-free twin up to 01:02.

</details>

<details>
<summary><b>Root causes</b> — 3 causes tested · C289 jam first</summary>

## Root causes (system conditions)

Each cause was removed in every combination and the shift replayed (exact Shapley values over 8 subsets). *Alone* = recovered by removing only this cause; *last* = what it adds once every other cause is gone.

| # | Condition | Shapley (parcel-min) | Share | Alone | Last | Needed to explain it? |
|---|---|---|---|---|---|---|
| 1 | chute C289 jams (F0) | 4329 ± 195 | 70 % ± 3 % | 5076 | 3627 | yes, with 2 other causes (possibly: yes, with 1 other cause) |
| 2 | robot R119 breaks down (F2) | 947 ± 55 | 15 % ± 1 % | 1329 | 610 | no (possibly: yes, with 1 other cause) |
| 3 | robot R57 breaks down (F1) (tie with the row above: within noise) | 921 ± 151 | 15 % ± 2 % | 1479 | 408 | no (possibly: yes, with 1 other cause) |

How we know: shares use every replay of the lattice (R0–R7); Alone and Last each come from one replay.

Interaction: chute C289 jams (F0) and robot R57 breaks down (F1) compound: together they caused 901 parcel-min more damage than each did alone.
Interaction: chute C289 jams (F0) and robot R119 breaks down (F2) compound: together they caused 549 parcel-min more damage than each did alone.

How we know: interactions come from every replay (R0–R7), not just the three named.

Smallest set of causes whose removal recovers at least 90 % of the recoverable loss (each judged against its own replay noise): {F0, F1, F2}.

Near misses (within the noise of the 90 % threshold, so possibly just as sufficient): {F0, F1} or {F0, F2}.

</details>

<details>
<summary><b>Capacity hotspots</b> — 2 limits found, a different question from the causes</summary>

## Capacity hotspots (a different question)

Limits the floor hit with no fault involved. Testing one means relaxing capacity from 00:30, not removing a fault, so these are not part of the shares above.

| Hotspot | Robot-minutes waiting | Relaxed: parcel-min recovered |
|---|---|---|
| robots queuing at busy station S3 (loading time; no fix modelled) | 21 | - |
| robots queuing at busy station S2 (loading time; no fix modelled) | 20 | - |

How we know: relaxed values come from a replay with the capacity limit lifted, not from removing a fault.

</details>

<details>
<summary><b>Trigger and detection</b> — alarm at 00:31</summary>

## Trigger and detection

Trigger: chute C289 jams at 00:30 (F0).
Detection: a one-sided CUSUM on the per-minute shortfall against the twin (k = 0.5, h = 3 in units of the twin's Poisson noise) alarmed at 00:31; the incident window runs 00:30 to 00:52.

How we know: the trigger is the scenario's earliest fault; detection compares the real run with its fault-free twin.

</details>

<details>
<summary><b>Resolution</b> — recovered 15 min after the window opened</summary>

## Resolution

Throughput was back within 5 % of the twin, and held there, 15 min after the window opened.

How we know: throughput is the real run's deliveries per minute compared with its fault-free twin.

</details>

<details>
<summary><b>What would have helped</b> — 4 fixes tested · best 83 %</summary>

## What would have helped

Each fix was applied to the real shift at the time shown and replayed to 01:02. Each fix's gain is the median over the histories of each history's own difference.

| Fix | From | Parcel-min recovered | Share of loss | Cost |
|---|---|---|---|---|
| send chute C289's destination to C269 from the jam on (never undone) | 00:30 | 5162 ± 372 | 83 % | sort-plan change |
| clear failed robots off loading points within 1 min (now 5 min) | 00:30 | 1731 ± 399 | 28 % | floor procedure |
| allow 8 robots inbound per station (now 6) | 00:30 | 775 ± 376 | 13 % | dispatch setting |
| release 10 spare robots at the first fault | 00:30 | 118 ± 446 (within noise) | 2 % | 10 robots |

How we know: each fix was replayed from the time shown (X1–X4).

</details>

<details>
<summary><b>Action items</b> — 3 actions</summary>

## Action items

| Action | Type | Expected effect | Owner |
|---|---|---|---|
| Reduce the frequency or duration of: chute C289 jams | prevent | up to 5076 ± 356 parcel-min | site ops |
| Send chute C289's destination to C269 from the jam on (never undone) | mitigate | 5162 ± 372 parcel-min | site ops |
| Clear failed robots off loading points within 1 min (now 5 min) | mitigate | 1731 ± 399 parcel-min | site ops |

How we know: the prevent row is the cause's replay, and the mitigate rows are the fix replays.

</details>

<details>
<summary><b>Timeline</b> — 12 events</summary>

## Timeline

| Time | Tick | Event |
|---|---|---|
| 00:30 | 3600 | F0 starts: chute C289 jams |
| 00:30 | 3600 | F1 starts: robot R57 breaks down |
| 00:30 | 3600 | F2 starts: robot R119 breaks down |
| 00:30 | 3600 | incident window opens |
| 00:31 | 3720 | CUSUM alarm |
| 00:35 | 4200 | R57 pushed off S3's loading cell by operators |
| 00:35 | 4200 | R119 pushed off S2's loading cell by operators |
| 00:45 | 5400 | F0 ends |
| 00:50 | 6000 | F1 ends |
| 00:50 | 6000 | F2 ends |
| 00:52 | 6240 | incident window closes |
| 01:02 | 7440 | loss scored (horizon) |

How we know: fault times come from the scenario; the alarm and window come from detection on the real run and its twin.

</details>

<details>
<summary><b>Method and confidence</b> — how the numbers were made and how far to trust them</summary>

## Method and confidence

- **Generated by** switchyard-analysis 1.0.0 · 96 replays (12 changes × 8 histories) · loss scored at 01:02 (window end + 10 min) · **noise band** ± 356 parcel-min
- **Counterfactual replay.** The shift was re-run with each combination of causes removed, resuming from the last 5 min checkpoint before the earliest removed cause, and scored at 01:02 as net parcel-minutes of delay against the fault-free twin (same seed, same parcels). Shapley values are exact over all subsets, in integer arithmetic, so the shares add up exactly to the recovered loss.
- **Noise.** Robot paths diverge after any small change, so every replay ran on 8 histories: the real one, and 7 copies (of 15 tried) in which one moving robot (R79, R120, R142, R172, R6, R25, R34) paused for a single tick at 00:30 and the shift diverged. The headline delay and late count are the real history's; every other value is the median across histories; ± is half their range. *Alone*, *Last* and each fix's gain are the median over the histories of each history's own difference. Two values that differ by less than twice that band are reported as a tie.
- **Detection** uses the twin's Poisson noise as the scale. h = 3 is 3.0 times the largest CUSUM value (1.0) seen in 160 simulated hours of fault-free runs whose robot paths were perturbed.


</details>

<details>
<summary><b>Appendix: replay index</b> — 12 changes × 8 histories, one row per change</summary>

## Appendix: replay index

| Replay | Change | From tick | Delay (parcel-min, median) | Range | Late (median) | World hash (real history) |
|---|---|---|---|---|---|---|
| R0 | actual shift | 7200 | 6190 | 5680..6393 | 87 | `e24dc1356c98cfa6` |
| R1 | remove F0 | 3600 | 1081 | 1032..1134 | 2 | `a545bdd213c06432` |
| R2 | remove F1 | 3600 | 4720 | 4466..4826 | 51 | `309304796dedb218` |
| R3 | remove F0, F1 | 3600 | 615 | 545..640 | 1 | `e513811053434d5e` |
| R4 | remove F2 | 3600 | 4880 | 4553..4979 | 83 | `69af1c7d1440b45a` |
| R5 | remove F0, F2 | 3600 | 409 | 390..431 | 1 | `79204649083d8685` |
| R6 | remove F1, F2 | 3600 | 3646 | 3362..3773 | 49 | `b159825c148180a9` |
| R7 | remove F0, F1, F2 | 3600 | -12 | -16..0 | 0 | `b531cfcd419d4f8f` |
| X1 | clear after 1 min | 3600 | 4390 | 4243..4581 | 52 | `f909b6a97edfdbee` |
| X2 | remap 1 destination to C269 | 3600 | 1090 | 1003..1168 | 2 | `7c2bdae4eb1f3e28` |
| X3 | add 10 robots | 3600 | 5973 | 5682..6339 | 94 | `fe2d298238b86a2d` |
| X4 | station cap 8 | 3600 | 5398 | 5302..5588 | 71 | `efddbd7c186648d6` |

[A] actual run and [T] fault-free twin, 14400 ticks (120 min).

</details>
