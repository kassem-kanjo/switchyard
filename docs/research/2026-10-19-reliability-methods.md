# Switchyard - Reliability Methods Research (2026-10-19)

Status: research input, not a decision record. Sources are numbered [n] and listed in section 7.
"Proposal" marks our own design built on the sources; it is not a sourced claim. "Unverified" marks what I could not confirm.

## 1. Recommendation (read this first)

The one property that changes everything: our sim is deterministic and seeded. A production system can only *estimate*
counterfactuals from observational data (Sage trains a generative model to do it [9]; causal-discovery RCA mostly ends up
near random [14]). We can *run* them. In Pearl's terms, the "abduction" step (recover the exogenous noise) is free because
the noise is the seed, so replay = exact counterfactual, not an estimate [12][13]. Every method below uses that.

**(a) Tracing - "conserved blame" spans + wait-for-graph blame chains (Proposal).**
- One trace per parcel, OTel-shaped (name, parent, start/end, attributes, events, links, status, kind [1]). The root span is
  `parcel`; children are *leaf phase spans that exactly tile its lifetime*, no gaps or overlaps: `induct.queue`,
  `assign.wait`, `travel.loaded`, `blocked`, `chute.wait_full`, `drop`. Invariant: sum of leaf durations = parcel latency.
  Because the phases are sequential, each parcel's critical path is the whole chain; CRISP's longest-path logic [2] is used
  at the *fleet* level (section 2.2).
- A `blocked` span carries `blocker.robot_id` and a **span link** to the blocker's current span in another parcel's trace.
  OTel defines links as "implying a causal relationship" without parent/child [1][3], which is what a cross-parcel block is.
- **Transitive blame**: each tick, build a wait-for graph (edge A->B = "A waits on B") [6]. Follow each chain to its
  terminal node (a dead robot, a full chute, an aisle closure, or a robot doing useful work) and charge the blocked time to
  that *root blocker*, keeping the full chain as an attribute. A cycle is a deadlock: wait-for cycle iff deadlock [6];
  charge it to a `deadlock` cause and record the members.
- Aggregate the way CRISP does: per-cause time at P50/P95/P99 across parcels, shown as a heatmap and flame graph [2].
- **Causal profiling without the Coz trick**: Coz has to fake speedups by slowing everything else ("virtual speedup")
  because it cannot change real code at run time [4][5]. We can make a real change and replay. Progress point = parcel
  sorted; impact = 1 - p_s/p_0 (Coz's formula, p = mean period between progress points) [5]. Output: "a robot 10% faster
  on aisle 7 gives +X% throughput", a ranked what-if table.

**(b) Chaos - paired-replay experiments scored on the Bruneau loss area (Proposal on top of [7][8][15][16]).**
- Experiment = steady-state hypothesis (throughput/min and P95 parcel latency within a band) + fault + blast radius +
  duration, following the Principles of Chaos [7]. The control group is the *same seed with no fault*, a noise-free twin.
  Netflix's ChAP has to approximate this with 1% baseline/canary clusters [8].
- Score each run with the resilience curve: DT (max dip), RR (time to recover), PL (Bruneau area) [15][16], plus a
  graceful-degradation ratio and a load-headroom context from Little/Kingman [17][18]. Formulas in section 3.
- Pick faults by lineage, as LDFI does: trace data shows which robots, aisles and chutes successful parcels depend on, and
  we inject there first [19].
- Run across N seeds and report the spread. Use per-entity RNG substreams (common random numbers) so a fault changes
  only what it causally touches [20][21].

**(c) Post-mortems - detect -> localize -> *prove by replay* -> rank -> remediate -> write (Proposal).**
1. Detect with one-sided CUSUM on throughput per minute [22]. BOCPD [23] is the optional upgrade for the time-series
   view. Both are deterministic given the data and need no training.
2. Localize candidates from the trace blame buckets: compare the incident window with a baseline window using a robust
   median/IQR z-score per entity. This is BARO-style scoring, which an independent study found among the strongest and
   fastest methods [14]. Add the injected-fault list and config/volume changes as candidates.
3. **Interventional replay**: for each candidate c, replay with do(c := healthy) from a snapshot before onset. Measure the
   fraction of lost throughput recovered: `rho(c) = (PL_actual - PL_do(c)) / PL_actual`. Rank by rho.
4. Interactions and overdetermination: if the top-k (k <= 5) do not each explain most of the loss, compute exact
   **Shapley values** over the candidates (2^k replays, 32 for k=5), as Budhathoki et al. do for outlier attribution
   [11]. Report the minimal sufficient set with ddmin [24]. Halpern-Pearl actual causation explains why a single but-for
   test fails under overdetermination [25].
5. Remediation: the same replay engine scores candidate fixes (re-route, re-assign chute, rate-limit induction, add a
   robot) by PL recovered. The report proposes the best fix *with its replayed effect*.
6. Write the report in Google SRE format (section 5) with every number traced back to a replay or a span query.

**Wow capability:** a post-mortem whose root cause is *proven, not guessed*. Example: "Had R7 not failed at 10:42:13,
412 of the 450 late parcels would have been on time (replay-verified, Shapley share 0.81). Remaining 0.19: chute C12 jam.
Re-routing aisle 7 recovers 88% of the loss." Plus a side-by-side "ghost" replay of the counterfactual floor. Production
RCA tools cannot do this, because they cannot re-run reality [9][14].

## 2. Tracing: sourced findings and definitions

### 2.1 Data model
- OTel span fields: name, parent span id, start/end timestamps, span context (trace/span id), attributes, events, links,
  status (unset/error/ok), kind (client/server/internal/producer/consumer) [1].
- Links "associate one span with one or more spans, implying a causal relationship", used for async and batch work [1][3].
  A span has one parent but many links, which suits fan-in [3]. Messaging conventions use links, not parent/child, to tie
  producer to consumer [3]. Mapping: induction station = producer, robot pickup = consumer, connected by a link.
- OpenTracing's `FollowsFrom` became OTel links with attributes [3]. Use a `link.type` attribute: `blocked_by`,
  `handoff`, `queued_behind`.
- The tech-stack brief already chose Parquet as the canonical store with OTLP and Chrome-trace as exports
  (`2026-10-19-tech-stack.md`). The span model above maps onto it at the edge.

### 2.2 Critical path (CRISP / Jaeger)
- CRISP models a trace as a DAG of spans. Algorithm: start at the end of the root span, recurse into the *last-finishing
  child* (LFC), then walk backwards to the child that finished just before the LFC started. Children that finished after
  the LFC started are not on the path [2].
- Clock-skew rules (truncate children to the parent's range, tolerate small sibling overlap) [2] are *not needed*: we
  have one clock. That is a selling point.
- Metrics: inclusive time vs exclusive (self) time on the critical path. Aggregate at P50/P95/P99 into heatmaps and flame
  graphs [2]. Jaeger UI also offers critical-path highlighting (a UI config option) [10].
- Our fleet-level use (Proposal): for a *wave* (e.g. all parcels for one outbound truck), the wave is a fork/join DAG.
  Its completion time = the last parcel, so run CRISP's LFC walk over the wave to find which parcels, and through their
  `blocked` links which robots, gate the truck's departure.

### 2.3 Blame chains (Proposal on top of [6])
- Wait-for graph: one node per robot, edge A->B when A is blocked by B. Deadlock iff a cycle exists; DFS finds it in
  O(V+E) [6].
- Definition: `root(A) = A` if A is not waiting, else `root(next(A))`. On a cycle, `root = deadlock{members}`.
  Per tick dt, blocked parcel p gets `blame[root(robot(p))] += dt`. This is *direct* vs *transitive* blame. Show both:
  direct blame says who was in the way, transitive blame says who started it.
- Conservation check (unit test): for every parcel, the leaf phase spans sum to the latency exactly, and the blame
  buckets sum to the blocked time exactly.

### 2.4 Causal profiling (Coz)
- Coz (SOSP'15 best paper): conventional profilers show where time goes, but optimizing that code may not help. Coz
  runs speedup experiments and measures the effect on progress points [4].
- Virtual speedup: pause other threads each time the target line runs. Speedups from 0-100% in 5% steps, with 0%
  chosen 50% of the time for the baseline. Effect = `1 - p_s/p_0` [5]. Real gains: Memcached +9%, SQLite +25%, PARSEC
  up to +68%, mostly from <10-line changes [4].
- For us: the core lesson holds, since time on the critical path is not the same as throughput impact. The mechanism
  becomes a real parameter change plus replay. Candidate "lines": per-robot speed, per-aisle speed, chute drain rate,
  induction rate. Run a sweep and plot impact against speedup per component, like a Coz plot [5].

## 3. Chaos engineering: sourced findings and formulas

- Steady state = "some measurable output of a system that indicates normal behavior". Four steps: define steady state;
  hypothesize it holds in control and experiment; inject real-world events; try to disprove via the difference.
  Advanced principles: vary real-world events, run in production, automate continuously, minimize blast radius [7].
- Netflix ChAP: canary vs baseline clusters (~1% each). It compares user KPIs, mainly stream starts per second, not the
  injected service's own health. Traffic exposure is capped, and there is a global "stop all chaos" switch [8].
  Translation: our KPI is parcels sorted per minute and on-time-to-truck rate, not robot health.
- LDFI (Alvaro et al., SIGMOD'15): reason backwards from a successful outcome's lineage to find faults that could have
  prevented it, then test those. Prunes the fault space; Netflix used it [19].
- Bruneau resilience loss: `R = integral_{t0}^{t1} [100 - Q(t)] dt`, Q = quality in %, t0 = event, t1 = full
  recovery [15]. Model dimensions: robustness, rapidity, resilience [16].
- Microservice adaptation [16], with benchmark PB(t) and performance P(t):
  - Disruption Tolerance `DT = max_{t in [ts,te]} (PB(t) - P(t))`
  - Recovery Rapidity `RR = te - ts`
  - Performance Loss `PL = integral_{ts}^{te} (PB(t) - P(t)) dt`. With throughput as P, PL = parcels not sorted
    that should have been [16].
- **Our score (Proposal):** PB(t) = the paired no-fault run (same seed), so PL is exact. Normalized resilience
  `S = 1 - PL / integral PB dt` over the fault window plus a recovery tail. Also report DT, RR, the P95 latency band
  breach time, and the **graceful-degradation ratio** `GDR = (Q_fault/Q_base) / (C_fault/C_base)`, where C = surviving
  capacity (live robots, open chutes). GDR >= 1 means the fleet loses less than its capacity share; GDR < 1 means
  congestion amplifies the fault. RR ends when rolling throughput stays within epsilon of PB for k windows.
- Little's law `L = lambda W` (WIP = arrival rate x time in system) is a theorem [17]. Use it as a runtime invariant
  check on the sim and to convert WIP into latency on dashboards.
- Kingman (G/G/1): `E[W_q] ~= (rho/(1-rho)) * ((c_a^2 + c_s^2)/2) * t_s`. The rho/(1-rho) term blows up as rho -> 1, and
  cutting variability cuts queueing at fixed rho [18]. Implication (Proposal): report baseline utilization rho = lambda/(c*mu)
  next to every score. Losing 20% of robots at rho = 0.6 vs 0.85 are different experiments. Find the "knee" with a load
  sweep: the lambda where P95 latency doubles from its low-load value.
- Prior art on blocking: in robotic mobile fulfilment, blocking between vehicles rises sharply with fleet size [26].
  This is the nonlinearity GDR is meant to expose.

## 4. Automated RCA: what fits a deterministic sim

| Method | Idea | Deterministic? | Needs training data? | Fit for us |
|---|---|---|---|---|
| MicroRCA [27] | attributed graph of services+hosts, personalized PageRank over the anomalous subgraph | yes | no | weak: correlation-based; our trace blame is already direct |
| Sage [9] | causal Bayesian net + graph VAE that *generates* counterfactuals ("what if service X were healthy?"); >93% accuracy | no (learned) | yes (unsupervised, historical traces) | idea yes, machinery no: we replay instead of learning |
| CIRCA [28] | RCA as "intervention recognition": a root-cause metric's distribution changes *given its parents*; graph from architecture knowledge | yes | no | good: we know the structural graph exactly |
| RCD [29] | failure = intervention on root cause; localized PC-variant (Psi-PC) learns only the relevant part of the graph | mostly | no | unnecessary: we have no graph to discover |
| CausalRCA [30] | gradient-based causal structure learning | no | fit per incident | no |
| PC-based (PC-PageRank, PC-RW) | discover the graph, then rank | yes | no | no: "no better or only slightly better than random" [14] |
| BARO / NSigma | robust (median/IQR) or sigma z-score vs pre-failure window | yes | no | yes, as the cheap localizer; BARO stays robust when onset time is off [14] |
| Budhathoki et al. [11] | Shapley attribution of an outlier to ancestors in a known DAG via conditional outlier scores | yes given model | no | yes, the *attribution* rule (we use replays as the value function) |
| Counterfactual RCA (CRFD) [12] | abduction-action-prediction, rank candidates by best counterfactual outcome | yes | partly | yes: this *is* our method, with exact abduction |

Key independent finding: across many methods and datasets, "no method stands out in all situations", causal graph F1
was only about 0.1-0.54, and PC-based RCA is about as good as random [14]. RCAEval has 735 failure cases and 15 baselines
[31]. Use it as the *metric vocabulary* (AC@k / top-k accuracy) for self-evaluating our RCA against injected ground truth.

Change-point detection:
- CUSUM (Page, Biometrika 1954) [22]: `S_t = max(0, S_{t-1} + (mu_0 - x_t) - k)` for a drop in throughput x. Alarm when
  S_t > h. k = half the shift you care about, h sets the false-alarm rate. Onset estimate = the last t where S_t = 0.
  Online, O(1), explainable ("throughput fell 2 sigma below baseline for 4 minutes").
- BOCPD (Adams & MacKay 2007) [23]: posterior over run length since the last change, by message passing, online.
  Deterministic given the data. Gives a probability of change, which looks good in the UI.

Ranking causes (Proposal, grounded in [11][24][25]):
- Value function `v(S) = PL_actual - PL_do(S := healthy)` over candidate sets S. Single-cause rank = v({c}). Shapley
  `phi_c = sum_{S without c} |S|!(n-|S|-1)!/n! * [v(S + c) - v(S)]`. Exact for n <= 6 (64 replays, parallel across
  cores). Shapley shares sum to v(all), so the report's percentages add up.
- Minimal cause set: ddmin over the candidates with test "replay without S keeps PL below threshold". Zeller's ddmin
  reduced 896 HTML lines to 1 in 139 tests [24].
- Overdetermination: two dead robots, either of which saturates the aisle, each fail the but-for test alone.
  Halpern-Pearl handles this with contingencies [25]; Shapley splits the blame symmetrically. Report both.
- Replay window: start from a snapshot at onset - delta, not at t=0, to bound cost. Requires a serializable sim state
  (Proposal; checks with the tech-stack brief's single-threaded core).

## 5. Post-mortem format (Google SRE)

- Example post-mortem header fields: Date, Authors, Status, Summary, Impact, Root Causes, Trigger, Resolution, Detection,
  Action Items. Body: Lessons Learned (What went well / What went wrong / Where we got lucky), Timeline, Supporting
  information [32].
- Culture: blameless, focused on contributing causes not people, assuming good faith. Define post-mortem triggers
  *before* incidents, e.g. user-visible downtime or data loss [33]. The workbook treats adoption as cultural change [34].
- Mapping (Proposal): Trigger = the fault event. Root Causes = Shapley-ranked replay-verified causes. Detection = the
  CUSUM alarm time and detection lag vs onset. Impact = PL in parcels, late-to-truck count, and P95 delta. Resolution =
  the self-recovery or fix. Action Items = replay-scored remediations with expected recovery %. "Where we got lucky" =
  a replay finding, e.g. "if the jam had started 3 min later, during the 11:00 wave, PL would have been 2.3x". Our
  triggers: throughput below SLO for >= N min, or any deadlock cycle.
- Blamelessness for robots (Proposal): "R7 blocked R3" is a fact about the system, so phrase causes as system
  conditions ("single-lane aisle 7 had no bypass"), as the SRE book recommends [33].

## 6. Prior art in logistics, and how we differ

- Process mining: an air-cargo terminal study (van der Aalst group) used event data to find bottleneck root causes, and
  noted noisy sensor data and abstraction limits [35]. Object-centric performance analysis (OPerA) handles events
  shared by several objects (parcel + robot + chute) where a single case notion fails [36].
- RMFS simulators: RAWSim-O is a discrete-event, agent-based simulator for robotic mobile fulfilment. It shows side
  effects such as task assignment causing path-planning congestion [37]. Digital twins of AMR warehouses use congestion
  heat maps for tuning [26]. Deadlock/collision prevention for warehouse robots is studied with agent-based models [26].
- Robotics fault injection exists at single-robot/ROS level (e.g. ROS Rescue, fault-tree recovery validated by injection
  campaigns), but I found no fleet-level chaos engineering [38]. A 2024 chaos-engineering review of 96 sources focuses on
  microservices [38].
- **How we differ (Proposal):** process mining is *descriptive* from logs, so it cannot prove causation. Digital twins
  and RMFS sims are used for *design-time* what-if, not incident RCA. We combine (1) per-parcel distributed traces with
  cross-entity causal links, (2) steady-state chaos experiments with a noise-free control twin, and (3) post-mortems whose
  causes are *interventionally verified*. No source I found combines these for robot fleets (absence claim, verify
  further).

## 6b. Open questions
1. Replay cost: from-snapshot replay needs serializable state. What is the cost of 64 replays of a 15-minute window at
   target fleet size? (Spike needed.)
2. Butterfly effect: even with per-entity RNG substreams, a removed fault can change task assignment downstream. Should
   attribution compare *aggregate* PL only (robust) or per-parcel outcomes (fragile)? CRN gains are "usually, not
   necessarily" positive [21].
3. "Healthy" for do(c) needs a definition per fault type (revive robot? unjam at t0? remove volume spike = which arrivals?).
   Needs an ADR.
4. Blame for robots doing *useful* work that block others: is that a cause (layout) or noise? Proposal: charge it to the
   aisle/layout, not the robot.
5. Shapley's exponential cost above n ~ 6: switch to sampling, or prune with single-cause rho first?
6. Unverified: Jaeger's critical-path release version and the exact UI algorithm [10]; Sage's GVAE internals beyond the
   abstract [9]; CIRCA's regression-based hypothesis test details [28].

## 7. Sources
1. OpenTelemetry, Traces concept page. https://opentelemetry.io/docs/concepts/signals/traces/
2. Uber Engineering, "CRISP: Critical Path Analysis for Microservice Architectures". https://www.uber.com/blog/crisp-critical-path-analysis-for-microservice-architectures/ (code: github.com/uber-research/CRISP)
3. SigNoz, "Span Links". https://signoz.io/docs/traces-management/guides/span-links/ ; OneUptime, "OpenTelemetry Span Links". https://oneuptime.com/blog/post/2026-01-07-opentelemetry-span-links/view
4. Curtsinger & Berger, "Coz: Finding Code that Counts with Causal Profiling", SOSP 2015. https://www.cl.cam.ac.uk/~ey204/teaching/ACS/R212_2015_2016/papers/curtsinger_SOSP_2015.pdf ; https://github.com/plasma-umass/coz
5. A. Colyer, "Coz: finding code that counts with causal profiling", The Morning Paper. https://blog.acolyer.org/2015/10/14/coz-finding-code-that-counts-with-causal-profling/
6. "Wait-for graph", Wikipedia. https://en.wikipedia.org/wiki/Wait-for_graph ; https://www.techinterview.org/post/3233469277/lld-deadlock-detection/
7. Principles of Chaos Engineering. https://principlesofchaos.org/
8. Basiri et al., "Automating chaos experiments in production" (ChAP). https://ar5iv.labs.arxiv.org/html/1905.04648 ; https://blog.acolyer.org/2019/07/05/automating-chaos-experiments-in-production/ ; https://www.gremlin.com/blog/podcast-break-things-on-purpose-ep-8-haley-tucker-resilience-engineering-at-netflix
9. Gan et al., "Sage: Practical & Scalable ML-Driven Performance Debugging in Microservices", ASPLOS 2021. https://www.csl.cornell.edu/~delimitrou/papers/2021.asplos.sage.pdf ; https://arxiv.org/pdf/2112.06263
10. Jaeger UI configuration docs (critical path option). https://www.jaegertracing.io/docs/latest/frontend-ui/ ; https://github.com/jaegertracing/jaeger-ui/issues/2179
11. Budhathoki, Minorics, Bloebaum, Janzing, "Causal structure-based root cause analysis of outliers", ICML 2022. https://proceedings.mlr.press/v162/budhathoki22a.html ; https://arxiv.org/abs/1912.02724
12. CRFD, "Causal and Interpretable Fault Diagnosis Using Counterfactual Reasoning for Microservices". https://dl.acm.org/doi/10.1145/3796706
13. "Counterfactual-based Root Cause Analysis for Dynamical Systems". https://arxiv.org/pdf/2406.08106
14. Pham et al., "Root Cause Analysis for Microservices based on Causal Inference: How Far Are We?" (ASE 2024). https://arxiv.org/html/2408.13729v1
15. Bruneau, "Enhancing the Resilience of Communities against Extreme Events", IABSE 2005 (eq. 1, from Bruneau et al. 2003, Earthquake Spectra 19(4), doi 10.1193/1.1623497). https://www.eng.buffalo.edu/~bruneau/IABSE%202005%20Bruneau.pdf
16. "On Representing Resilience Requirements of Microservice Architecture Systems". https://arxiv.org/pdf/1909.13096
17. J.D.C. Little, "Little's Law as Viewed on Its 50th Anniversary", Operations Research 59(3), 2011. https://people.cs.umass.edu/~emery/classes/cmpsci691st/readings/OS/Littles-Law-50-Years-Later.pdf
18. "Kingman's formula", Wikipedia. https://en.wikipedia.org/wiki/Kingman%27s_formula
19. Alvaro, Rosen, Hellerstein, "Lineage-driven Fault Injection", SIGMOD 2015. https://blog.acolyer.org/2015/03/26/lineage-driven-fault-injection/ ; https://www.theregister.co.uk/2016/02/01/netflix_tries_molly_in_quest_for_enhanced_fault_finding_perception/
20. "Noise-free comparison of stochastic agent-based simulations using common random numbers". https://arxiv.org/html/2409.02086v2 ; "Variance reduction", Wikipedia. https://en.wikipedia.org/wiki/Variance_reduction
21. Wright & Ramsay, "On the Effectiveness of Common Random Numbers", Management Science 1979. https://pubsonline.informs.org/doi/10.1287/mnsc.25.7.649
22. E.S. Page, "Continuous Inspection Schemes", Biometrika 41, 1954 (doi 10.1093/biomet/41.1-2.100); "CUSUM", Wikipedia. https://en.wikipedia.org/wiki/CUSUM
23. Adams & MacKay, "Bayesian Online Changepoint Detection", 2007. https://arxiv.org/abs/0710.3742
24. Zeller & Hildebrandt, "Simplifying and Isolating Failure-Inducing Input", IEEE TSE 2002. https://courses.cs.washington.edu/courses/cse503/10wi/readings/zeller-tse2002.pdf
25. Halpern, "A Modification of the Halpern-Pearl Definition of Causality". https://ar5iv.labs.arxiv.org/html/1505.00162 ; "A Game-Theoretic Account of Responsibility Allocation". https://arxiv.org/pdf/2105.09129
26. TUM RMFS study (blocking vs fleet size). https://mediatum.ub.tum.de/doc/1453020/document.pdf ; WSC 2023 AMR digital twin. https://www.informs-sim.org/wsc23papers/cea157.pdf ; Turhanlar & Ekren, deadlock/collision prevention. https://gcris.yasar.edu.tr/entities/publication/f91d746e-3296-4317-aa81-03234f214140/full
27. Wu et al., "MicroRCA: Root Cause Localization of Performance Issues in Microservices". https://hal.archives-ouvertes.fr/hal-02441640
28. Li et al., "Causal Inference-Based Root Cause Analysis for Online Service Systems with Intervention Recognition" (CIRCA), KDD 2022. https://arxiv.org/abs/2206.05871
29. Ikram et al., "Root Cause Analysis of Failures in Microservices through Causal Discovery" (RCD), NeurIPS 2022. https://proceedings.neurips.cc/paper_files/paper/2022/hash/c9fcd02e6445c7dfbad6986abee53d0d-Abstract.html
30. "CausalRCA: Causal Inference based Precise Fine-grained Root Cause Localization for Microservice Applications". https://arxiv.org/pdf/2209.02500
31. Pham et al., "RCAEval: A Benchmark for Root Cause Analysis of Microservice Systems with Telemetry Data". https://arxiv.org/pdf/2412.17015
32. Google SRE Book, "Example Postmortem". https://sre.google/sre-book/example-postmortem/
33. Google SRE Book, ch. 15 "Postmortem Culture: Learning from Failure". https://sre.google/sre-book/postmortem-culture/
34. Google SRE Workbook, ch. 10 "Postmortem Culture". https://sre.google/workbook/postmortem-culture/
35. Li, van der Aalst et al., air cargo terminal process mining case study. https://www.vdaalst.rwth-aachen.de/publications/p1423.pdf
36. "OPerA: Object-Centric Performance Analysis". https://arxiv.org/pdf/2204.10662
37. Merschformann, Xie, Li, "RAWSim-O: A Simulation Framework for Robotic Mobile Fulfillment Systems", Logistics Research 2018. https://arxiv.org/pdf/1710.04726
38. "ROS Rescue: Fault Tolerance System for Robot Operating System". https://arxiv.org/pdf/1910.01078 ; "Chaos Engineering: A Multi-Vocal Literature Review". https://arxiv.org/pdf/2412.01416
