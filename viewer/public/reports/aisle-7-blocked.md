# Post-mortem: aisle-7-blocked (seed 1)

**Incident 00:30–00:51 · up to 30 % below the fault-free twin**

The aisle blockage caused 100 % of the delay.

**2,428** parcel-min of delay · **0** parcels late

*How we know: proven by 40 replays of this shift · error bars from 8 versions of the day.*

## Summary

From 00:30 sorted throughput fell up to 30 % below the fault-free twin. By 01:01 the incident had cost **2428 parcel-minutes** of delay, and 0 parcels were more than 10 min late.
Aisle blocked (98 cells) (F0) explains 100 % ± 25 % (2297 parcel-min).

How we know: the real shift and its fault-free twin, plus replays with each cause removed (R0–R1); see the replay index at the end.

<details>
<summary><b>Impact</b> — 0 parcels late · peak backlog 261 (twin 91)</summary>

## Impact

| Measure | Value |
|---|---|
| Delay at 01:01 vs the fault-free twin | 2428 parcel-min (± 576) |
| Parcels more than 10 min late at 01:01 | 0 |
| Parcels more than 10 min late (whole run, to 02:00) | 0 |
| Peak backlog | 261 parcels (twin 91) |
| Worst minute / time to recover | 30 % below the twin / 20 min |
| Stations / chutes out of action at once | 0 / 0 |
| Parcels left unsorted at the end of the run | 1 |

How we know: the real shift compared with its fault-free twin up to 01:01.

</details>

<details>
<summary><b>Root causes</b> — 1 cause tested · aisle blockage first</summary>

## Root causes (system conditions)

Each cause was removed in every combination and the shift replayed (exact Shapley values over 2 subsets). *Alone* = recovered by removing only this cause; *last* = what it adds once every other cause is gone.

| # | Condition | Shapley (parcel-min) | Share | Alone | Last | Needed to explain it? |
|---|---|---|---|---|---|---|
| 1 | aisle blocked (98 cells) (F0) | 2297 ± 579 | 100 % ± 25 % | 2297 | 2297 | yes, on its own |

How we know: shares use every replay of the lattice (R0–R1); Alone and Last each come from one replay.

Interactions: none beyond the noise; the causes add up independently.

Smallest set of causes whose removal recovers at least 90 % of the recoverable loss (each judged against its own replay noise): {F0}.

</details>

<details>
<summary><b>Capacity hotspots</b> — 3 limits found, a different question from the causes</summary>

## Capacity hotspots (a different question)

Limits the floor hit with no fault involved. Testing one means relaxing capacity from 00:30, not removing a fault, so these are not part of the shares above.

| Hotspot | Robot-minutes waiting | Relaxed: parcel-min recovered |
|---|---|---|
| robots queuing at busy station S2 (loading time; no fix modelled) | 20 | - |
| robots queuing at busy station S3 (loading time; no fix modelled) | 20 | - |
| robots waiting on chute C289 bag swaps | 13 | 98 ± 840 |

How we know: relaxed values come from a replay with the capacity limit lifted, not from removing a fault.

</details>

<details>
<summary><b>Trigger and detection</b> — alarm at 00:35</summary>

## Trigger and detection

Trigger: aisle blocked (98 cells) at 00:30 (F0).
Detection: a one-sided CUSUM on the per-minute shortfall against the twin (k = 0.5, h = 3 in units of the twin's Poisson noise) alarmed at 00:35; the incident window runs 00:30 to 00:51.

How we know: the trigger is the scenario's earliest fault; detection compares the real run with its fault-free twin.

</details>

<details>
<summary><b>Resolution</b> — recovered 20 min after the window opened</summary>

## Resolution

Throughput was back within 5 % of the twin, and held there, 20 min after the window opened.

How we know: throughput is the real run's deliveries per minute compared with its fault-free twin.

</details>

<details>
<summary><b>What would have helped</b> — 2 fixes tested · none clearly helps</summary>

## What would have helped

Each fix was applied to the real shift at the time shown and replayed to 01:01. Each fix's gain is the median over the histories of each history's own difference.

| Fix | From | Parcel-min recovered | Share of loss | Cost |
|---|---|---|---|---|
| release 10 spare robots at the first fault | 00:30 | 212 ± 799 (within noise) | 9 % | 10 robots |
| allow 8 robots inbound per station (now 6) | 00:30 | 189 ± 770 (within noise) | 8 % | dispatch setting |

How we know: each fix was replayed from the time shown (X1–X3).

</details>

<details>
<summary><b>Action items</b> — 1 action</summary>

## Action items

| Action | Type | Expected effect | Owner |
|---|---|---|---|
| Reduce the frequency or duration of: aisle blocked (98 cells) | prevent | up to 2297 ± 576 parcel-min | site ops |

How we know: the prevent row is the cause's replay, and the mitigate rows are the fix replays.

</details>

<details>
<summary><b>Timeline</b> — 6 events</summary>

## Timeline

| Time | Tick | Event |
|---|---|---|
| 00:30 | 3600 | F0 starts: aisle blocked (98 cells) |
| 00:30 | 3600 | incident window opens |
| 00:35 | 4200 | CUSUM alarm |
| 00:50 | 6000 | F0 ends |
| 00:51 | 6120 | incident window closes |
| 01:01 | 7320 | loss scored (horizon) |

How we know: fault times come from the scenario; the alarm and window come from detection on the real run and its twin.

</details>

<details>
<summary><b>Method and confidence</b> — how the numbers were made and how far to trust them</summary>

## Method and confidence

- **Generated by** switchyard-analysis 1.0.0 · 40 replays (5 changes × 8 histories) · loss scored at 01:01 (window end + 10 min) · **noise band** ± 576 parcel-min
- **Counterfactual replay.** The shift was re-run with each combination of causes removed, resuming from the last 5 min checkpoint before the earliest removed cause, and scored at 01:01 as net parcel-minutes of delay against the fault-free twin (same seed, same parcels). Shapley values are exact over all subsets, in integer arithmetic, so the shares add up exactly to the recovered loss.
- **Noise.** Robot paths diverge after any small change, so every replay ran on 8 histories: the real one, and 7 copies (of 12 tried) in which one moving robot (R100, R120, R172, R6, R11, R13, R25) paused for a single tick at 00:30 and the shift diverged. The headline delay and late count are the real history's; every other value is the median across histories; ± is half their range. *Alone*, *Last* and each fix's gain are the median over the histories of each history's own difference. Two values that differ by less than twice that band are reported as a tie.
- **Detection** uses the twin's Poisson noise as the scale. h = 3 is 3.0 times the largest CUSUM value (1.0) seen in 160 simulated hours of fault-free runs whose robot paths were perturbed.


</details>

<details>
<summary><b>Appendix: replay index</b> — 5 changes × 8 histories, one row per change</summary>

## Appendix: replay index

| Replay | Change | From tick | Delay (parcel-min, median) | Range | Late (median) | World hash (real history) |
|---|---|---|---|---|---|---|
| R0 | actual shift | 7200 | 2292 | 1947..3099 | 0 | `e91aec6e3c8edbb5` |
| R1 | remove F0 | 3600 | -10 | -15..0 | 0 | `055559671d1c1c24` |
| X1 | add 10 robots | 3600 | 2106 | 1507..2471 | 0 | `7e61d9482aaeecd9` |
| X2 | station cap 8 | 3600 | 2019 | 1458..2999 | 0 | `ef3b9dd4388c10f6` |
| X3 | remap 1 destination to C269 | 3600 | 2141 | 2007..2684 | 0 | `4b104a6d3ac361d0` |

[A] actual run and [T] fault-free twin, 14400 ticks (120 min).

</details>
