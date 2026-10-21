# Switchyard - Post-mortem Engine Research for CP3 (2026-10-20)

Scope: skills sweep and implementation research for CP3 (ADR 0005). Read-only against `crates/core`. Claims are tagged **[V]** (checked
against a source this session), **[K]** (background knowledge, cited but not re-read) or **[P]** (proposal to validate in a CP3 spike).
Web and GitHub content was treated as data.

## 1. Recommendation (read this first)

1. **Detection.** One-sided CUSUM on the per-minute deficit `D_t = twin_t - actual_t`, standardised by a *Poisson-noise scale*, not by
   the pre-onset spread (which is exactly 0, see 3.1). Start with **k = 0.5, h = 5** (sigma units). Window = Page's last-zero start to a
   "settled" end. **Loss is always measured at a fixed evaluation horizon** `T_eval`, never at a run-dependent recovery tick (fixes CP2 finding 6).
2. **Attribution.** Run all 2^n subsets (n <= 6, so 63 replays beyond the actual run), convert the value table to **Harsanyi dividends** with a
   fast Moebius transform, and read **exact Shapley values and pairwise interactions** off the dividends in integer arithmetic. We hold the whole
   lattice, so the minimal sufficient set is a **lattice scan**, not ddmin (ddmin only if n > 6). Report negative shares honestly.
3. **Cost control.** (a) Make `Sim` cloneable (only `Box<dyn Planner>` blocks it) and keep **checkpoints of the actual run**; start each replay at
   the latest checkpoint before the earliest *removed* fault. (b) End each replay at `T_eval`, not 2 h. (c) Exact state-hash re-convergence is a
   free bonus that will probably rarely fire; don't plan on it. (d) rayon over masks, results indexed by mask, so output is deterministic.
   Native: seconds. Browser: n <= 4, or post-mortem native-only in V1.
4. **Fix ranking.** Each fix is an `Intervention` applied at tick t0 on a cloned `Sim`, scored by recovered loss over the same horizon.
   `set_arrival_rate` and `set_scanner_error_rate` exist; reroute, add robot, station cap and chute remap need engine hooks.
5. **Report.** Google SRE sections plus a replay-ID footnote on every number (template in section 7).
6. **Skills.** Adopt **none** (section 2).

## 2. Skills sweep

`~/.claude/skills/README.md` has: mission, studio-operating-model, research-first, coding-discipline, skill-authoring, writing-plans, TDD,
systematic-debugging, dispatching-parallel-agents, requesting/receiving-code-review, verification, property-based-testing, rust-skills,
frontend-design, webapp-testing, portfolio-studio. Nothing covers causal inference, post-mortems, change-point detection or report writing.
`property-based-testing` and `rust-skills` already cover most CP3 test and code needs.

| Candidate | Stars | Licence | Last push | What its SKILL.md is | Verdict |
|---|---|---|---|---|---|
| mohitagw15856/pm-claude-skills (`incident-postmortem`) | 1,472 | MIT | 2026-10-20 | Generic blameless post-mortem: timeline, root cause vs contributing factors, owned action items (1,255-skill PM library) | Skip. Right tone, nothing sim- or replay-specific; our template is stricter |
| same repo (`agent-incident-postmortem`) | same | MIT | same | Post-mortem for LLM-agent incidents; "pin everything when replaying" | Skip. Wrong domain; the idea is already our design |
| anyshift-io/sre-skills | 17 | Apache-2.0 | 2026-10-07 | Kubernetes, IAM, S3 investigators | Skip. Low traction, infra-specific |
| K-Dense-AI/scientific-agent-skills (`aeon`, `shap`, `statsmodels`) | 53,268 | MIT | 2026-10-19 | Python time-series ML incl. change points; SHAP for fitted ML models | Skip. Python/ML; our CUSUM is ~15 lines of Rust and our Shapley is exact over replays |
| anthropics/skills (`doc-coauthoring`, `internal-comms`) | 182,500 | per skill | 2026-10-09 | Document-writing workflows | Skip. We generate the report from data |
| hanlulong/awesome-ai-for-economists; ericluo04/claude-academic-workflow | 678 / 25 | CC0 / other | 2026-10 | Observational causal inference (DiD, IV, RDD) | Skip. We intervene, not infer |

Searches: root-cause analysis, post-mortem, causal inference, incident response, SRE, time series / change point, technical report writing.
No repo offers counterfactual replay or Shapley over interventions. **Recommendation: none.** After CP3, write an in-house
`postmortem-engine` skill via `skill-authoring` (template and planted-incident test method).

## 3. Findings per item

### 3.1 CUSUM on the throughput deficit

- **Recursion [V, NIST e-Handbook 6.3.2.3].** `S_i = max(0, S_{i-1} + x_i - mu_0 - k)`, alarm when `S_i > h`. NIST: k = delta*sigma/2 (half the
  shift to catch); Montgomery rule of thumb k = 0.5 and h = 4 or 5 in sigma units.
- **False-alarm rate vs delay [K, Siegmund 1985; Hawkins & Olwell 1998].** One-sided, standardised: `ARL(D) ~ (exp(-2D(h+1.166)) + 2D(h+1.166) - 1)/(2D^2)`,
  with D = mean shift minus k; in control (D = -k): `(exp(2k(h+1.166)) - 2k(h+1.166) - 1)/(2k^2)`. A web search did not return the formula text,
  so confirm it in a reference before quoting. My arithmetic from it (minutes, one-sided; two-sided textbook values 168 and 465 are half):

  | k | h | ARL0 (false alarm) | ARL1, 1 sigma shift | ARL1, 2 sigma shift |
  |---|---|---|---|---|
  | 0.5 | 4 | ~340 | ~8 | ~3.5 |
  | 0.5 | 5 | ~940 | ~10 | ~3.9 |
- **Why the deficit is special [P].** Twin and actual share seed and per-entity RNG, so before the first fault `D_t = 0` exactly: the
  pre-onset variance is 0 and cannot scale the chart, and classical false alarms do not exist. h becomes a **materiality threshold** ("sustained
  loss worth a report"). Scale with the Poisson noise of the twin's throughput, `sigma_t ~ sqrt(twin_t)` per minute (about 11 parcels at ~7,000/h
  = 117/min), or a MAD scale from a fault-free calibration run. Illustration: a 31 % dip is ~36 parcels/min ~ 3.3 sigma, alarm in ~4 min.
  Calibrate ARL0 empirically (throughput is autocorrelated and not exactly Poisson): run M fault-free pairs with *different* seeds for twin
  and actual, feed `D_t`, pick h for ARL0 of at least ~10 sim-hours.
- **"Deficit >= 0 by construction" is not quite true.** After recovery the faulted floor drains backlog, so `D_t` goes *negative*, and butterfly
  noise exists. So: (1) do not clip `D_t` before accumulating (clipping biases the sum up); the `max(0, .)` in the recursion is the one-sided floor;
  (2) a lower arm `T_t = max(0, T_{t-1} - D_t - k)` detects *recovery*; (3) one-sidedness saves us the lower-arm false-alarm analysis.
- **Window [P, extends Page 1954].** Start = last `t` with `S_t = 0` before the alarm (Page's change-point estimate); optionally walk back to the
  first minute with `D_t > 0` for "first divergence". End = first minute after the alarm where `S_t = 0` and stays 0 for m = 5 min ("settled").
  Loss = cumulative deliveries gap at `T_eval = window end + 10 min`, i.e. the net parcels lost after backlog drain. Every replay uses the same
  `T_eval`, so rho and Shapley values are comparable.
- BOCPD (Adams & MacKay 2007, in the 10-19 research) stays out of CP3 scope.

### 3.2 Exact Shapley over <= 6 interventions

- **Setup [P].** Candidates `c_1..c_n`; mask `S` = causes *removed*. `L(S)` = lost parcels at `T_eval` in the replay with `S` removed;
  `v(S) = L(empty) - L(S)`, with `L(empty)` the actual run, so `v(empty) = 0`; `v(N)` = total recoverable. If the candidates are all the
  faults, `L(N)` is the twin and is reused. Replays: 2^n - 1 (63 at n = 6).
- **Enumeration.** Fill `v[mask]` in parallel. The **Harsanyi dividend** `m(T) = sum_{U subset T} (-1)^{|T|-|U|} v(U)` [K, Harsanyi 1963] comes from
  the in-place subset-Moebius transform (`for i in 0..n { for mask with bit i { m[mask] -= m[mask ^ (1<<i)] } }`), n*2^n steps (384 at n = 6).
  Shapley is each dividend split equally among its members: `phi_i = sum_{T contains i} m(T)/|T|` [V: stated in the sources the search returned].
  The pair interaction index is `sum_{T contains i,j} m(T)/(|T|-1)` [K, Grabisch & Roubens 1999; form only]. The classical weights `s!(n-s-1)!/n!`
  give the same numbers; the dividend route yields interactions for free and is easy to test.
- **Determinism.** Core is integer-only. Compute `n! * phi_i` in `i128` (`m(T) * n!/|T|`, an integer), divide only when rendering. Efficiency
  (`sum phi_i = v(N)`) then holds exactly and is a unit-test oracle.
- **Non-monotone `v`.** Removing a cause can *worsen* the outcome (unblocking an aisle floods a full chute). Shapley remains defined for any `v` with
  `v(empty) = 0` (efficiency, symmetry, null player hold) but `phi_i` and `m(T)` can be negative and percentages leave [0,1]. Rules: report **parcels as
  well as percent**; print a negative value as "removing this alone *hurts*; it was masking X"; divide by `v(N)` only if `v(N)` exceeds a floor (5 % of
  loss or 10 parcels), else say "no attributable loss"; list `unexplained = L(empty) - v(N)` ("intrinsic congestion", ADR 0005) separately; mark any
  `|v|` below the replay noise band "not distinguishable from noise" (3.3.4).
- **Interactions to report.** Pair interaction > 0 = complements (together recover more than the sum); < 0 = substitutes (overdetermination: each
  alone recovers most). Plus the top three dividends by |m(T)| as plain sentences, and leave-one-out `v(N) - v(N \ i)` beside Shapley so readers see
  where but-for and Shapley differ.
- **Minimal sufficient set.** ddmin [Zeller & Hildebrandt 2002, in 10-19] is 1-minimal and assumes monotone, unambiguous tests. With the lattice
  already computed, scan by size and return the smallest `S` with `v(S) >= tau * v(N)` (tau = 0.9), listing ties: globally minimal at zero extra
  replays. ddmin remains the fallback for n > 6.
- **Actual causation [V that the work exists; K for formulae].** Halpern-Pearl's but-for test fails under overdetermination (two dead robots, either
  of which closes the aisle) and uses *contingencies*. Chockler & Halpern (JAIR 22, 2004) grade it as degree of responsibility `1/(1+k)`, k = the
  smallest contingency that makes the cause critical [formula not re-read]. On our lattice: smallest `|T|` with `v(T + {c}) >= tau*v(N)` and
  `v(T) < tau*v(N)`. Print "responsibility 1/2" next to the Shapley share for overdetermined causes.
- **Budhathoki et al. (ICML 2022) [from 10-19].** Shapley attribution of an outlier over a set function in a *known* DAG; we keep the attribution
  rule and replace the learned set function with replays.

### 3.3 Replay cost control

**`Sim` today (read only).** `Sim` derives only `Debug`: **not `Clone`**. Every field type is `Clone`/`Arc` (`World`, `Faults`, `Stream`/ChaCha8,
`RootWalker`, `DeadlockWatch`, `PoissonTable`, `ZipfTable`, `PlanOutput`, `DistanceTables` via `Arc`, `ScenarioConfig`, `Layout`) **except**
`planner: Box<dyn Planner>` (`Planner: Send + Debug`, no clone method; `PibtPlanner` itself derives `Clone`). Fix: add `fn box_clone(&self) -> Box<dyn Planner>`
to `Planner` (or the `dyn-clone` crate [K]) and derive `Clone` on `Sim`. There is no serde snapshot of the sim (`snapshot.rs` is the viewer's flat buffer);
serialisation is only needed to persist checkpoints or cross a WASM boundary, so in-process `Clone` suffices. 1 tick = 0.5 s (7,200 per hour).

1. **Snapshot-and-resume [P].** Until the earliest tick at which any *removed* fault acts, replay and actual run are the same state (faults act in
   phase 1 of their window, and removal never shifts unrelated RNG draws; that is what `disabled_faults` and per-key streams are for, per `faults.rs`).
   - Run the actual scenario once, cloning a checkpoint every K = 600 ticks (5 min). Size per checkpoint at 1,000 robots: unmeasured.
   - For subset `S`: take the latest checkpoint at or before `min onset(f in S)`, set the faults in `S` to `Status::Disabled` (new
     `Sim::disable_fault(id)`, legal only while `Pending`), step to `T_eval`. Different subsets start at different checkpoints.
   - Mandatory tests: (T1) clone-then-step equals step, equal state hash at every tick; (T2) checkpoint + `disable_fault(f)` equals a from-tick-0
     `without_fault(f)` run, identical event stream after the checkpoint and equal final hash. Property-test over random scenarios.
2. **Early termination on re-convergence [P, low expectation].** Equal state at equal tick means equal futures, so stop and copy the outcome.
   Caveats from the code: `state_hash` includes the fault schedule and active-fault state, so a replay (with `Disabled` faults) never hashes equal to
   the actual run; add a **fault-free `world_hash`** and also require no remaining schedule difference. A perturbed floor (positions, queues, bag fills,
   planner history) rarely returns to *exact* equality, so hits are expected mainly for faults that never mattered. Hash every 120 ticks against
   a stored reference trace; merge two replays whose hashes meet (hash-consing the future). Measure the hit rate before building more.
   The reliable saving is stopping at `T_eval`.
3. **Parallelism.** `par_iter` over masks, `collect` in mask order; each replay owns its `Sim`, no shared mutation, so output is deterministic.
   CP2 measured 10 two-hour medium sims in 0.85 s. My estimate (unmeasured): 63 replays, each cut to 20-40 min after a checkpoint, are about 10-15
   full-sim equivalents: **~1-2 s on this 16-thread machine**, a few seconds on 4 cores. Browser rayon needs `wasm-bindgen-rayon`, SharedArrayBuffer,
   a nightly toolchain with atomics and COOP/COEP headers (GitHub Pages needs a service-worker workaround) [V: crate README, MDN, via search].
   Recommendation: **post-mortem native-only in V1**; the viewer shows CLI output, or runs n <= 4 (15 replays) in a worker.
4. **Replay noise floor [P].** The butterfly effect (10-19 open question 2) means tiny perturbations change assignment downstream. Estimate the floor
   as the spread of `L(empty)` under a neutral intervention (a no-op fault, or re-seeded twins) and print it as +/- on every `v`. Compare aggregates
   at `T_eval`, never per-parcel outcomes.

### 3.4 "What would have helped": ranking fixes

A **fix** is an `Intervention` applied at tick `t0` to a clone of the checkpoint at or before `t0`, scored by `gain = L(empty) - L(fix)` at `T_eval`.
It is applied on top of the real, faulty trajectory, so it is judged against what actually happened.

| Fix | Config transform at t0 | Engine hook needed | Notes |
|---|---|---|---|
| Reroute around a closed aisle | Treat the faulted aisle's cells as closed in the distance tables from t0 (before the fault would block robots inside), or lower `against_flow_cost` to open a bypass | Closure exists only while a fault is active; add `Sim::close_cells(set)` reusing the `TABLE_CACHE` | Only for `aisle-block` and `robot-failure`; "bypass" is a layout change, ask a domain contact |
| Remap a destination to another chute | Change `dest -> chute` for hot destinations of the jammed chute to the least-loaded healthy chute | Mapping is built once (`sim/mapping.rs`); need a mutable table and a rule for parcels already carried | Candidate list from the jammed chute |
| Add a robot | Append robots at free parking cells | Columns grow by append, existing ids unchanged | Report gain per robot (diminishing returns) |
| Raise a station cap | `max_inbound_per_station` + 1 for the station with the longest queue | Currently one global u16 | Cheap |
| Rate-limit induction | `set_arrival_rate` | **Exists** | Backpressure value |
| Scanner upgrade / faster bag swap | `set_scanner_error_rate`; chute swap ticks | scanner setter **exists** | For drift and jam incidents |

Rank by `gain`, tie-break by cost proxy (robots added, knobs touched). Report the best fix with its replay ID and noise band; never recommend one
that does not beat the noise floor. This is Coz's "virtual speedup" made real (10-19 §1a).

### 3.5 Post-mortem format for an operations audience

- **Google SRE example [V, 10-19 §5]:** header (Date, Authors, Status), Summary, Impact, Root Causes, Trigger, Resolution, Detection, Action Items,
  then Lessons Learned (What went well / What went wrong / Where we got lucky), Timeline, Supporting information. Blameless, contributing causes not people.
- **Ops needs [P]:** (1) the number first (parcels lost, minutes degraded, late-to-truck); (2) ranked causes as plain system conditions; (3) what to
  do on the floor next time; (4) a confidence line (noise band, unexplained share). Shapley math goes to an appendix. Every number carries `[R:n]`.
- Avoid statistical jargon in the summary and naming a robot as at fault ("aisle 7 had no bypass", not "R31 caused it").

### 3.6 Congestion attribution: why replay beats blocking chains (README story)

- **Argument.** A blocking chain (CP2 wait-for walk) charges delay to a root along the *direct* waiting dependence. A fault's real effect is the
  **total effect** `E[Y | do(c := healthy)] - E[Y]`, which sums *all* causal paths, including indirect ones (yields, queue build-up, busy chutes,
  reassigned robots, fleet-wide ripple). Pearl's separation of total, direct and path-restricted effects [V: UAI 2001, "Direct and Indirect Effects"]
  is the formal home of the claim: chain attribution is at best a path-restricted effect, so it undercounts mediated congestion. Replay computes the
  total effect exactly because abduction (the seed) is free (10-19 §1).
- **RCA literature [from 10-19 §4]:** counterfactual RCA ranks causes by counterfactual outcome [12][13]; learned-graph methods were near random
  in an independent benchmark [14]; Sage generates counterfactuals because correlation is not enough [9]. **No source states this for sortation
  floors.** It is a deduction, so *show it*.
- **Make it a number [P].** In the headline preset compare `blamed_delay(f) / total_delay` with `rho(f) = v({f}) / L(empty)`. CP2 already found
  800 of 827 blocked ticks unattributed in one case. Publish a "blame-vs-replay gap" chart in the README.
- **Caveats:** replay answers "what if healthy", not "which robot touched which"; hotspot interventions are coarser; part of a gap can be butterfly
  noise, so show the noise band.

## 4. Suggested CP3 order

1. `Sim: Clone`, `Planner::box_clone`, `Sim::disable_fault`, tests T1/T2. **Spike:** checkpoint size and one checkpointed replay time.
2. `crates/analysis`: CUSUM and window; fault-free calibration; planted-incident fixtures.
3. Lattice runner (rayon), Moebius transform, Shapley, interactions, minimal set; proptests: exact efficiency, symmetry, null player, inversion.
4. Interventions for fixes: start with `set_arrival_rate`, station cap, chute remap.
5. Report renderer, golden-file test, `switchyard postmortem`.

## 5. Planted-incident test plan

Inject known causes into a scripted scenario and assert: top-1 equals the injected cause; Shapley shares sum to `v(N)` exactly; two independent faults
have `m({a,b})` within the noise band; two substitute faults have interaction < 0; a no-op fault has `|phi|` within noise. Report AC@1/AC@3 as in RCAEval (10-19, ref 31).

## 6. Open questions

1. `T_eval` margin (10 min?) and tau (0.9?): tune on the chaos presets.
2. Is the Poisson scale adequate under Zipf-skewed destinations (over-dispersed per-minute throughput)? Calibrate by simulation.
3. Shortlisting must include *all* active faults, not only blamed ones, until CP2 blame findings 1-3 are fixed.
4. A capacity relaxation applies from t0, not tick 0, so it answers a different counterfactual than removing a fault; label it so.
5. What can operators actually do for "reroute"? Ask a domain contact.
6. Unverified here: Siegmund formula text; Chockler-Halpern `1/(1+k)`; Grabisch-Roubens exact weights; checkpoint size; replay times.

## 7. Post-mortem template (`{{ }}` = placeholder; `[R:n]` = replay n in the Replay Index)

```markdown
# Post-mortem: {{title}} ({{scenario}}, seed {{seed}})
**Date:** {{sim_date}} | **Status:** {{draft|final}} | **Generated by:** switchyard {{version}}, {{n_replays}} replays, horizon T_eval = {{t_eval}} | **Noise band:** +/- {{noise}} parcels [R:{{noise_ids}}]

## Summary
Throughput fell {{dip_pct}} % at {{onset_clock}} [R:0]. {{cause_1}} explains {{share_1}} % of lost parcels, {{cause_2}} {{share_2}} %; {{unexplained_pct}} % is intrinsic congestion [R:{{ids}}].
{{best_fix}} would have recovered {{fix_pct}} % [R:{{fix_id}}].

## Impact
| Measure | Value | Replay |
|---|---|---|
| Parcels lost at T_eval vs fault-free twin | {{lost}} | [R:0, R:{{twin}}] |
| Maximum dip / time to recover | {{max_dip}} per min / {{ttr}} min | [R:0] |
| Late-to-truck / P95 latency change | {{late}} / {{p95_delta}} s | [R:0, R:{{twin}}] |
| Mis-sorted parcels | {{missorts}} | [R:0] |

## Root causes (system conditions, blameless)
| # | Condition | Fault ID | Shapley (parcels) | Share | Alone (but-for) | Responsibility | Replay |
|---|---|---|---|---|---|---|---|
| 1 | {{condition}} | {{fid}} | {{phi}} | {{share}} % | {{v_alone}} | {{resp}} | [R:{{ids}}] |
Interactions: {{sentence, e.g. "A and B substitute: either alone recovers 80 %, both 85 %"}} [R:{{ids}}].
Smallest sufficient set (>= {{tau}} % of recoverable loss): {{set}} [R:{{ids}}]. Unexplained: {{unexp}} parcels.

## Trigger and detection
Trigger: {{fault event, tick, clock}}. CUSUM (k={{k}}, h={{h}}) alarmed at {{alarm_clock}}, {{lag}} min after onset; window {{start}} to {{end}} [R:0].

## Resolution
{{self-recovery or action}}; throughput back within {{eps}} % at {{clock}} [R:0].

## What would have helped
| Fix | Parcels recovered | Share of loss | Cost | Replay |
|---|---|---|---|---|
| {{fix}} | {{gain}} +/- {{noise}} | {{pct}} % | {{cost}} | [R:{{id}}] |

## Lessons learned
What went well: {{...}}. What went wrong: {{...}}. Where we got lucky: {{replay finding, e.g. "had the jam started at {{t}}, loss would have been {{x}}x" [R:{{id}}]}}.

## Action items
| Action | Type | Expected recovery | Owner | Replay |
|---|---|---|---|---|

## Timeline
| Clock | Tick | Event | Source |
|---|---|---|---|

## Appendix: Replay Index
| R | Removed set (mask) | From checkpoint tick | Horizon | Lost parcels | Final state hash |
|---|---|---|---|---|---|
```

## 8. Sources
- NIST/SEMATECH e-Handbook, 6.3.2.3 CUSUM: https://www.itl.nist.gov/div898/handbook/pmc/section3/pmc323.htm [V]
- Page, Biometrika 41, 1954; Siegmund, *Sequential Analysis*, 1985; Hawkins & Olwell, *Cumulative Sum Charts and Charting for Quality Improvement*, 1998 [K]
- Pearl, "Direct and Indirect Effects", UAI 2001: https://arxiv.org/pdf/1301.2300 [V via search]
- Chockler & Halpern, "Responsibility and Blame: A Structural-Model Approach", JAIR 22, 2004: https://jair.org/index.php/jair/article/view/10386
- Halpern, "A Modification of the Halpern-Pearl Definition of Causality": https://ar5iv.labs.arxiv.org/html/1505.00162 (10-19 ref 25)
- Budhathoki et al., ICML 2022: https://proceedings.mlr.press/v162/budhathoki22a.html (10-19 ref 11); Zeller & Hildebrandt, TSE 2002 (ref 24)
- Grabisch & Roubens, Int. J. Game Theory 28, 1999 [K]; Harsanyi dividends and Shapley: https://arxiv.org/pdf/2305.09204 , https://orbilu.uni.lu/bitstream/10993/2839/1/CardinalProbabilisticInteractionIndices.pdf (via search)
- wasm-bindgen-rayon: https://github.com/piotr-roslaniec/wasm-bindgen-rayon ; MDN SharedArrayBuffer
- Google SRE Book: https://sre.google/sre-book/example-postmortem/ , https://sre.google/sre-book/postmortem-culture/ (10-19 refs 32-33)
- Skills read via `gh api`: mohitagw15856/pm-claude-skills, anyshift-io/sre-skills, K-Dense-AI/scientific-agent-skills, anthropics/skills
