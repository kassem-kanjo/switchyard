# Switchyard - Robot Coordination Research (2026-10-19)

Status: research input, not a decision record. Scope: the lifelong multi-agent path finding (LMAPF) and task-assignment layer only.
Numbers come from papers that used different hardware, maps and action models. Treat them as orders of magnitude, not benchmarks.
"Unverified" marks claims I could not confirm in a primary source.

## 1. Recommendation (read this first)

**Use guided, causally instrumented PIBT as the per-tick planner. Put an action-dependency layer (ADG-style) under it for
delays and failures. Use a greedy scheduler plus an offline destination-to-chute mapping optimiser. Write it natively in Rust.
Treat LNS refinement and LaCAM as later, optional layers with deterministic iteration budgets.**

| Layer | V1 choice | Why |
|---|---|---|
| Motion model | 4-connected "pebble" moves (no rotation) | Rotation hurts PIBT-family throughput badly. With four-way moves, WPPL beat PIBT by about 185-192% on Game, Warehouse and Sortation ([Jiang et al. 2024](https://arxiv.org/html/2404.16162)). Add rotation in V2 through EPIBT-style multi-action operations ([Yukhnevich & Andreychuk](https://arxiv.org/abs/2511.09193)). |
| Per-tick planner | PIBT with dynamic priorities ([Okumura et al.](https://arxiv.org/abs/1901.11282)) | It takes about 1-1.5 ms per step at 1,000 agents ([LLLG table](https://arxiv.org/html/2605.16855v1)). Reachability is guaranteed on biconnected graphs. It is greedy and single-pass, so it is deterministic under a seeded tie-break. Its priority-inheritance tree *is* the causal explanation (section 7). |
| Congestion control | Guidance graph: directed or weighted lanes, hand-designed first, then optimised offline | This is the biggest single lever. On Sortation 10000, PIBT went from 10.20 to 29.99 and WPPL from 10.19 to 39.34 with a manual guidance graph ([Jiang et al.](https://arxiv.org/html/2404.16162)). Optimisation methods: [GGO](https://arxiv.org/abs/2402.01446), [online GGO](https://arxiv.org/abs/2411.16506), [mixed directions](https://arxiv.org/pdf/2602.23468), and [traffic-flow guide paths](https://ojs.aaai.org/index.php/AAAI/article/view/30054). |
| Admission control | Agent "disabling" or throttling: park the robots furthest from their goals when the floor is over-saturated | Without it, throughput fell 47-52% on dense maps ([Jiang et al.](https://arxiv.org/html/2404.16162)). The 2024 winner also used it, with n = 2,500 active on Game ([EPIBT report](https://expo25.leagueofrobotrunners.org/resources/No-Man-Sky.pdf)). |
| Execution / failures | ADG (action dependency graph) between planner and robots. Dead robot = permanent obstacle. Wait-for-cycle detector. | ADG keeps execution collision-free under delays ([Hönig et al. 2019](https://whoenig.github.io/publications/2019_RA-L_Hoenig.pdf)), and its edges are explicit "who waits for whom" facts. |
| Task assignment | Greedy nearest-free-robot each tick, with lookahead to chute queue length | The 2024 winner found that Hungarian assignment did not pay for its time budget ([EPIBT report](https://expo25.leagueofrobotrunners.org/resources/No-Man-Sky.pdf)). |
| Chute mapping | Offline evolutionary-algorithm/MILP destination-to-chute mapping, scored by our own sim | +8-29% throughput over greedy mappings in the sortation simulator ([Zhang et al. 2025](https://arxiv.org/html/2510.03472)). |
| V2 quality layer | LNS refinement with a fixed iteration count (not wall-clock), or Lifelong LaCAM | WPPL, EPIBT and PUSH all use LNS on top of PIBT. A wall-clock "anytime" budget breaks bit-reproducibility, so use iteration counts. |

**Why not the leaderboard stack as-is (EPIBT + parallel LNS):** the leaderboard is optimised for throughput under a 1 s wall-clock
budget on multicore servers. It runs parallel instances and keeps the best ([EPIBT report](https://expo25.leagueofrobotrunners.org/resources/No-Man-Sky.pdf)).
That conflicts with our single-threaded, bit-reproducible core (see `2026-10-19-tech-stack.md`). It also makes waits harder to explain:
a wait chosen by simulated-annealing LNS has no local reason. At 200-1,000 robots, plain guided PIBT already sits in the
millisecond range, so the extra 10-30% from LNS is not worth V1 complexity.

**Why not RHCR / PBS / ECBS:** RHCR handles only hundreds of agents within 1 s ([Jiang et al.](https://arxiv.org/html/2404.16162)).
It took 2.13 s per step at 1,000 agents and failed at 2,000 ([LLLG](https://arxiv.org/html/2605.16855v1)). Its code is under a USC research licence ([repo](https://github.com/Jiaoyang-Li/RHCR)).

**Why not learned policies (SILLM, MAPF-GPT style):** SILLM reports +16% over the best search-based method at 10k agents ([Jiang et al. ICRA 2025](https://arxiv.org/abs/2410.21415)).
But it needs a GPU, it is not explainable per step, and it is hard to make bit-deterministic across platforms (WASM vs native).

## 2. State of the art (findings by area)

### 2.1 Algorithm families
- **PIBT** (IJCAI-19, AIJ) plans one step at a time. Each agent picks its best neighbour cell. If that cell is occupied, the occupant
  *inherits* the agent's priority and must move first; if it cannot, it backtracks. Priorities grow until the agent reaches its goal,
  which guarantees reachability on graphs where every adjacent pair lies on a cycle, for example biconnected graphs ([arXiv 1901.11282](https://arxiv.org/abs/1901.11282)).
  It can get stuck on maps with dead ends ([PIBT-for-MAPD study, SciTePress 2025](https://www.scitepress.org/Papers/2025/132438/132438.pdf)).
- **Causal-PIBT** (AAAI-21) is the event-based, delay-tolerant version. It models each PIBT step as a virtual DFS tree of
  parent/child dependencies and defines a deadlock as a cycle of requesting agents ([Okumura et al. arXiv 2005.13187](https://arxiv.org/abs/2005.13187)).
  The EPIBT repo ships a `CAUSAL_PIBT` planner ([Straple/EPIBT](https://github.com/Straple/EPIBT)).
- **RHCR** (AAAI-21) is rolling-horizon windowed MAPF with PBS, ECBS or WHCA. It was evaluated with up to 1,000 agents and includes a SORTING scenario
  ([arXiv 2005.07371](https://arxiv.org/abs/2005.07371), [code](https://github.com/Jiaoyang-Li/RHCR)).
- **LaCAM / LaCAM\*** is search over configurations, with PIBT as the successor generator. It is complete (LaCAM\* is eventually optimal), and
  the README says it handles "more than 1k agents" in real time ([lacam3](https://github.com/Kei18/lacam3), [AAMAS-24 paper](https://arxiv.org/html/2308.04292v2)).
  Base LaCAM solved 10,000-agent instances in at most 10 s ([AAAI-23](https://ojs.aaai.org/index.php/AAAI/article/download/26377/26149)).
- **Lifelong LaCAM with Local Guidance (LLLG)** (SoCS 2026) is windowed LaCAM with spatiotemporal local guidance. It claims an advantage over
  PIBT because LaCAM search can prevent local livelock and support deadlock detection ([arXiv 2605.16855](https://arxiv.org/html/2605.16855v1), [code](https://github.com/allegorywrite/lllg)).
- **WPPL** (LoRR 2023 winner) is windowed PIBT for an initial plan, refined by parallel MAPF-LNS, plus a guidance graph and agent disabling.
  It scored 9.755 vs 9.502 for second place ([Jiang et al. arXiv 2404.16162](https://arxiv.org/html/2404.16162)).
- **EPIBT** (LoRR 2024 winner, all categories, AAAI-26) is PIBT over multi-action *operations* of length 3-5, so rotating robots can clear cells.
  It adds simulated-annealing LNS, a guidance graph and a greedy scheduler, and allows collisions with higher-priority agents, which breaks
  PIBT's reachability guarantee ([report](https://expo25.leagueofrobotrunners.org/resources/No-Man-Sky.pdf), [arXiv 2511.09193](https://arxiv.org/abs/2511.09193)).
- **Guided PIBT / traffic-flow optimisation** (AAAI-24) gives PIBT congestion-avoiding guide paths as its heuristic and refines them iteratively.
  It was the LoRR 2024 baseline ([Chen et al.](https://ojs.aaai.org/index.php/AAAI/article/view/30054), [code](https://github.com/nobodyczcz/Guided-PIBT)).
  A follow-up uses a probabilistic flow over equal-cost paths ([Lin et al. AAMAS-26](https://ifaamas.org/Proceedings/aamas2026/pdfs/IRBO1733.pdf)).
- **MD-PIBT** (IROS-26) searches over *agent dependencies*. It generalises PIBT and EPIBT, handles up to 10k agents including differential-drive
  robots, and is built on an "Agent Dependency Graph" ([arXiv 2603.23405](https://arxiv.org/abs/2603.23405), [code](https://github.com/lunjohnzhang/MD-PIBT)).
- **PUSH** (Aug 2026) combines token-passing-style staggered partial replanning with windowed paths and EPIBT pushing. It reports about +25%
  (warehouse) and about +300% (random-32-32-20) max throughput over EPIBT-LNS at up to 10k agents and 1 s/step. Its code is not linked ([arXiv 2608.06702](https://arxiv.org/html/2608.06702)).
- **GD-RHCR** (Aug 2026) parallelises RHCR by spatial groups, with near-optimality theory and PIBT as the fallback for congested groups
  ([arXiv 2608.17928](https://arxiv.org/abs/2608.17928)).

### 2.2 Sortation-specific work
- **Idle-time optimisation** (AAAI-20) is a min-cost max-flow assignment that minimises sorting-station idle time in a window, extended
  to the lifelong setting. It was tested with up to 350 agents in an industrial simulator ([Kou et al.](https://ojs.aaai.org/index.php/AAAI/article/view/6547)).
  Key insight: sortation throughput is limited by *station idle time*, not by robot travel alone.
- **Destination-to-chute task mapping** (IEEE MRS 2025) uses an EA+MILP mapping. Its simulator uses PIBT on the LoRR-2023 winner's code. Maps were 33x57
  and 50x86, with 105-703 chutes and 600-1,200 robots. It gained 8-29% throughput over greedy mappings ([arXiv 2510.03472](https://arxiv.org/html/2510.03472), [code](https://github.com/lunjohnzhang/tmo_public)).
- **Dynamic chute mapping with distributionally robust MARL** holds up under varying induction rates ([arXiv 2503.09755](https://arxiv.org/html/2503.09755v1)).
- **Guidance graphs on sortation maps:** a reported Amazon-sortation-style map is 179x69 with about 1,000 agents ([Zang et al. online GGO, AAAI-25](https://arxiv.org/abs/2411.16506);
  the map size comes from a search summary and is unverified).

## 3. Throughput, scaling and per-tick compute

| Source | Setting | Result |
|---|---|---|
| [LoRR 2023](https://arxiv.org/html/2404.16162) | Budget | 1 s per tick for all agents, 30 min preprocessing |
| [LoRR 2024](https://expo25.leagueofrobotrunners.org/resources/No-Man-Sky.pdf) | Budget | 1 s per tick, up to 10,000 agents, rotation model, scheduling + planning share the budget |
| [LLLG](https://arxiv.org/html/2605.16855v1) | random-64-64-10, 1,000 agents (throughput / s per step) | PIBT 14.7 / 0.0015; Guided-PIBT 16.0 / 0.015; LaCAM 17.1 / 0.043; RHCR 20.7 / 2.13; LLLG 21.1 / 0.21 |
| [LLLG](https://arxiv.org/html/2605.16855v1) | empty-48-48, 1,000 agents | PIBT 19.4 / 0.0011; LaCAM 21.4 / 0.032; LLLG 27.3 / 0.26; RHCR failed |
| [LLLG](https://arxiv.org/html/2605.16855v1) | warehouse up to 10k agents | LLLG under 1 s/step, about +30% over PIBT; RHCR fails at 2,000 |
| [WPPL paper](https://arxiv.org/html/2404.16162) | PIBT runtime | about 250 ms on Sortation 10000 (rotation), about 20 ms on Random 800 |
| [WPPL paper](https://arxiv.org/html/2404.16162) | Sortation 10000 throughput | No GG: PIBT 10.20, WPPL 10.19. Manual GG: PIBT 29.99, WPPL 39.34 |
| [SILLM](https://arxiv.org/abs/2410.21415) | 10k agents, 6 maps | +16.0% vs best search-based, +137.7% vs best learned (GPU) |
| [GRAND](https://www.arxiv.org/abs/2512.03194) | Scheduler, ≤500 agents | Up to +10% over the LoRR-2024 winning scheduler, within 1 s |

**Budget for Switchyard:** with 1,000 robots on a 4-connected grid, a Rust PIBT should take about 1-2 ms per tick (the C++ figures above;
Rust is unverified but should be comparable). That leaves more than 95% of a 50-100 ms simulated-tick budget for tracing, ADG bookkeeping and chaos hooks.
That headroom is why PIBT fits a sim that also has to emit per-parcel spans.

## 4. Deadlock, livelock and failure handling
- **What PIBT guarantees:** reachability holds only on suitable topologies such as biconnected graphs ([Okumura](https://arxiv.org/abs/1901.11282)). Dead-end aisles can cause stalls
  ([SciTePress 2025](https://www.scitepress.org/Papers/2025/132438/132438.pdf)). **Implication:** after each failure injection, re-check
  the free-cell graph's biconnectivity (Tarjan, O(V+E)). Treat articulation points next to a dead robot as a predicted-risk span.
- **EPIBT trades the guarantee for throughput.** It allows collisions with higher-priority agents ([report](https://expo25.leagueofrobotrunners.org/resources/No-Man-Sky.pdf)).
- **LaCAM-based planners** can detect and escape local livelock through search ([LLLG](https://arxiv.org/html/2605.16855v1)). Use one as a "stuck recovery" planner for a local
  sub-region (V2).
- **Delays:** ADG/TPG execution keeps plans collision-free under delays, but waits propagate ([Hönig et al.](https://whoenig.github.io/publications/2019_RA-L_Hoenig.pdf)).
  A Causal-PIBT-style ordering argument shows that one slow or crashed robot can constrain the whole system under dependency-preserving
  execution ([Okumura et al.](https://arxiv.org/abs/2005.13187)). Delay-aware PIE gives up to 3x throughput vs simple delay policies ([Zhang et al. AAAI-25](https://ojs.aaai.org/index.php/AAAI/article/view/34506)).
  Learning *when* to replan recovers up to 94.6% of the achievable delay reduction ([Zahrádka et al. 2026](https://arxiv.org/abs/2604.25567)).
- **Fail policies when the planner returns nothing:** replan with PIBT, all-wait, LRGW, generalised ADG, or Guided PIBT. LSMART compares them,
  and found that higher replanning frequency does *not* reliably resynchronise a real fleet ([Yan et al. 2026](https://arxiv.org/abs/2602.15721)).
- **The 2026 LoRR edition** adds stochastic delay probabilities and an Execution track. Start-Kit v3.1.0 generates delays at runtime ([CFP](https://groups.google.com/g/search-list/c/l-4k-EREQbw)).
  This matches our chaos injection almost one to one.
- **Proposed failure semantics for Switchyard** (design, not sourced):
  - dead robot: remove it from the agent set, mark its cell as an obstacle, release its parcel task back to the scheduler.
  - blocked cell: remove the edge or vertex, recompute guidance heuristics incrementally.
  - delay: the robot misses its action, and ADG successors wait.
  - watchdog: a cycle in the wait-for graph, or no progress for N ticks, triggers a local LaCAM or agent-disable recovery.
  Because PIBT replans every tick, injected failures need no separate "replan" event; the next tick sees the new world.

## 5. Task assignment and congestion
- Greedy beats elaborate assignment under tight budgets. The LoRR-2024 winner tried a parallel Hungarian method, abandoned it, and blamed imperfect cost
  estimates ([report](https://expo25.leagueofrobotrunners.org/resources/No-Man-Sky.pdf)).
- Assignment that is aware of congestion and rebalancing helps. GRAND (GNN + min-cost-flow rebalancing + local assignment) reports +10% ([arXiv 2512.03194](https://www.arxiv.org/abs/2512.03194)).
- In sortation, the **chute mapping** sets the traffic pattern. A good mapping is worth 8-29% ([arXiv 2510.03472](https://arxiv.org/html/2510.03472)).
  Station idle time is the true objective ([Kou et al.](https://ojs.aaai.org/index.php/AAAI/article/view/6547)).
- More robots is not always more throughput. The best active count on Game 4000 was about 2,500 ([Jiang et al.](https://arxiv.org/html/2404.16162)).
  Switchyard should report throughput against fleet size as a first-class curve.

## 6. Open-source implementations

| Repo | Lang | Licence | Quality signals | Reuse verdict |
|---|---|---|---|---|
| [Kei18/pibt2](https://github.com/Kei18/pibt2) | C++17 | MIT | CI, GoogleTest, MAPD lifelong mode, 119★ | **Port** the PIBT core to Rust (a few hundred lines); use it as a reference oracle |
| [Kei18/pypibt](https://github.com/Kei18/pypibt) | Python | MIT (stated by pibt2 README; unverified) | Minimal | Readable spec for the port |
| [Kei18/lacam3](https://github.com/Kei18/lacam3) | C++17 | MIT (asks to be contacted for commercial use) | CI test, pybind branch, 135★ | V2 stuck-recovery/oracle; port later |
| [MAPF-Competition/Start-Kit](https://github.com/MAPF-Competition/Start-Kit) | C++/Python | MIT | Official LoRR kit v3.1.0, delay generation | **Reuse maps and instance format** for benchmarking; cross-validate our throughput |
| [DiligentPanda/MAPF-LRR2023](https://github.com/DiligentPanda/MAPF-LRR2023) (WPPL) | C++ | MIT | 7 commits, "may clean later" | Reference only |
| [Straple/EPIBT](https://github.com/Straple/EPIBT) | C++20 | MIT | 156 commits, sortation map, PIBT/CAUSAL_PIBT/EPIBT/LNS, GREEDY scheduler | **Best reference** for the scheduler, guidance and Causal-PIBT |
| [nobodyczcz/Guided-PIBT](https://github.com/nobodyczcz/Guided-PIBT) | C++ | MIT | 8 commits, good flag docs | Reference for the traffic-flow heuristic |
| [lunjohnzhang/MD-PIBT](https://github.com/lunjohnzhang/MD-PIBT) | C++ | MIT | Young (7 commits) | Watch; dependency-graph ideas |
| [lunjohnzhang/tmo_public](https://github.com/lunjohnzhang/tmo_public) | n/a | not checked | Chute-mapping optimiser | Reference for offline chute mapping |
| [Jiaoyang-Li/RHCR](https://github.com/Jiaoyang-Li/RHCR) | C++ | USC Research Licence | 221★, SORTING scenario | **Do not reuse** (licence); idea reference only |
| [LSMART](https://smart-mapf.github.io/lifelong-smart) | n/a | not checked | ADG + fail policies + physics | Design reference for the execution layer |
| [openplan-labs/pymapf](https://github.com/openplan-labs/pymapf) | Python | MIT | 194 commits, PIBT/LaCAM/LNS | Python oracle for tests |

**Recommendation:** do not FFI-link C++. It breaks the "one `cargo run`" and WASM story. Port PIBT natively, and use pibt2 or pymapf as
differential-test oracles on small instances (same seed, same tie-break, then compare paths).

## 7. Causal information for tracing
Per step, we want to answer "why did robot R wait or detour?". Ranked by how naturally each algorithm gives the answer:
1. **PIBT / Causal-PIBT:** each tick builds a DFS tree. "j inherited priority from i" means *i pushed j*. Backtracking "invalid" means *j could not vacate:
   blocked by k, or by an obstacle*. A stay with no requester means *idle, or no better cell*. The tree is explicit in Causal-PIBT ([paper](https://arxiv.org/abs/2005.13187)).
   Emit one span event per inheritance or backtrack: `{tick, robot, cause: PushedBy|BlockedBy|PriorityLoss|Obstacle|ChuteQueue, other}`.
2. **ADG / TPG execution:** each wait is an explicit edge "R waits for S to leave cell c" ([Hönig et al.](https://whoenig.github.io/publications/2019_RA-L_Hoenig.pdf)).
   This is ideal for delay propagation chains in post-mortems: the critical path is a walk through the ADG.
3. **MD-PIBT:** an explicit agent-dependency graph ([arXiv 2603.23405](https://arxiv.org/abs/2603.23405)).
4. **Prioritised planning / PBS (RHCR):** you get "lower priority than X", but the detour is implicit in the space-time A* reservation table.
5. **LNS refinement, LaCAM search, learned policies:** the final plan has no local reason. You can only get attribution by counterfactual re-runs,
   which our deterministic core supports, at higher cost.
Chute queues are not a MAPF concept. Model the chute as a capacity-limited resource with a FIFO queue, and emit `WaitingForChute{queue_pos}`.

## 8. Options comparison

| Option | Throughput | Determinism | Deadlock story | Causal attribution | Effort (Rust) | Fit |
|---|---|---|---|---|---|---|
| **A. Guided PIBT + ADG + greedy (recommended V1)** | Good with a guidance graph | Exact | Reachability on biconnected graphs + watchdog | Native (inheritance tree + ADG) | Low | Best |
| B. A + iteration-budget LNS | +10-30% (WPPL/EPIBT range) | Exact if iteration-budgeted | Same as A | Mixed (LNS edits are opaque) | Medium | V2 |
| C. EPIBT (rotation model) | LoRR-2024 SOTA | Exact if single-threaded | Weaker (guarantee dropped) | Good (operations tree) | Medium | V2 if rotation is needed |
| D. Lifelong LaCAM (LLLG) | Above RHCR, about +30% over PIBT | Exact | Search-based livelock escape | Weak | High | Recovery layer only |
| E. RHCR (PBS/ECBS) | High at ≤1k | Exact | Fails when the start is infeasible | Medium | High + licence | No |
| F. Learned (SILLM) | Highest reported | Hard (GPU/float) | None formal | None | Very high | No |

## 9. Open questions
1. Grid model: pure 4-connected, or rotation? This decides PIBT vs EPIBT and changes throughput 2-3x ([Jiang et al.](https://arxiv.org/html/2404.16162)).
2. Is our sortation layout (induction rows, chute field, return lanes) biconnected after realistic failures? We need a topology checker in CI.
3. Guidance graph: hand-designed one-way lanes in V1, or offline GGO (CMA-ES) on our maps? Should the optimised graph be versioned as an artefact?
4. Do we adopt LoRR Start-Kit instance formats so our throughput numbers are comparable with published ones?
5. Delay model: match the LoRR-2026 delay probabilities so chaos results are comparable? 2026 results were due 7 Aug 2026 and were not found.
6. How much tracing overhead per tick is acceptable at 1,000 robots (number of events per tick, Parquet volume)?
7. Is PUSH's code released, and does it keep PIBT's explainability? (Aug 2026 preprint, no code found.)
