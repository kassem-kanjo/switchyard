# Switchyard: product specification

**Status:** V1 scope agreed · **Owner:** Engagement Lead, Switchyard · **Last updated:** 2026-10-19
**Research:** [industry and client](research/2026-10-19-industry-and-client.md) · [robot coordination](research/2026-10-19-robot-coordination.md) · [reliability methods](research/2026-10-19-reliability-methods.md) · [tech stack](research/2026-10-19-tech-stack.md)

## 1. The client and the problem

**Client archetype:** a parcel carrier running a robotic sortation hub. Hundreds of small robots carry parcels from induction stations to destination chutes. Examples: Aramex Jeddah (120 robots, ~4,000 parcels/h), SPL Riyadh (200 robots), and Geek+/Tompkins t-Sort/Libiao deployments worldwide.

**Problem.** When a shift underperforms, nobody can say *why* with evidence:
- Design-time simulators (FlexSim, Emulate3D, AnyLogic) answer "will this layout work?" before go-live.
- Robot-developer tools (Formant, Foxglove) answer "what is this robot doing?"
- Process mining (Celonis) shows *that* the process deviated, after the fact.
- None of them answers the operations question: **"Throughput dropped 30 % at 14:02. What caused it, how much did each cause cost, and what would have fixed it?"** Today that answer is a meeting, a guess and a spreadsheet. Public explanations of big failures are coarse ("volume exceeded capacity").

## 2. The insight

> **A sortation hub is a distributed system.**

| In software | On the sort floor |
|---|---|
| Requests | Parcels |
| Workers / servers | Robots |
| Queues | Induction stations, chutes |
| Contention, lock waits | Robots blocking robots |
| Outages | Dead robots, blocked aisles, jammed chutes, scanner drift, volume surges |

Software engineering has mature tools for exactly this question: distributed tracing, chaos engineering and blameless post-mortems. Switchyard brings them to the floor. One property makes our version stronger than the software original: **the floor model is deterministic, so a root cause can be proven by replay**, not estimated statistically.

## 3. Users (inferred; to be validated with interviews)

| Persona | Question they bring | What Switchyard gives them |
|---|---|---|
| **Site operations manager** | "Why did yesterday's peak miss the truck cut-off?" | An automatic post-mortem with replay-verified causes and their cost in parcels |
| **Shift lead** | "Which parcels are late right now, and why?" | Per-parcel traces: click a late parcel, see where its time went and who blocked it |
| **Network / capacity planner** | "Will this hub survive peak season?" | A chaos suite and a resilience score before peak, plus a fault catalog |
| **Robotics / controls engineer** | "Is congestion a planner problem or a layout problem?" | Blame chains through the robot wait-for graph, plus what-if replays |

## 4. Product capabilities (V1)

1. **Sortation-floor twin.** A deterministic simulation of a configurable floor: grid, induction stations, chutes, guidance lanes and 50–1,000 robots coordinated by guided PIBT. It is calibrated to the industry ranges (≈ 25–50 parcels/h per robot).
2. **Parcel tracing.** One trace per parcel, with phase spans (queued → awaiting robot → loading → travelling → blocked → chute wait → drop) that exactly cover its lifetime. Every *blocked* span names its direct blocker and its root blocker, found by walking the robot wait-for chain. Traces export to Perfetto (Chrome trace JSON) and OpenTelemetry (OTLP JSON).
3. **Chaos engineering.** A fault catalog: robot failure, aisle block, chute jam, induction outage, volume surge and scanner drift. Every experiment runs against a fault-free twin with the same seed, so the comparison has no noise. The resilience score reports lost parcels (the area of the throughput dip), maximum dip, time to recover and the graceful-degradation ratio, always next to baseline utilisation.
4. **Automatic post-mortems.**
   1. A CUSUM detector raises an incident on the throughput series.
   2. Blame from the traces shortlists candidate causes.
   3. Each candidate is **replayed with that cause removed** and ranked by the share of loss it recovers. When causes interact, exact Shapley values split the blame.
   4. The output is a Google-SRE-format post-mortem in which every number traces back to a replay.
5. **Viewer.** A web app running the core in WASM: a live floor, a throughput chart, and click-a-parcel to see its trace. Visuals are deliberately simple in V1 and sit behind a swappable renderer interface.
6. **CLI.** Batch runs, chaos suites, post-mortems and trace export: `switchyard run | chaos | postmortem | export`.

## 5. Success metrics (V1)

| Metric | Target |
|---|---|
| Determinism | The same seed gives a bit-identical state hash on Windows, macOS, Linux and WASM (checked in CI) |
| Scale | 500 robots at ≥ 20× real time natively (tick = 0.5 s) |
| Calibration | Throughput per robot within 25–50 parcels/h on the default medium hub |
| Trace integrity | Every parcel's phase spans sum exactly to its latency (property test) |
| Causal correctness | On seeded "planted" incidents (we inject a known cause), the post-mortem ranks the planted cause first in ≥ 90 % of cases |
| Time to explanation | Post-mortem for a 1-hour shift in under 60 s on a laptop |
| Demo | The viewer loads and runs in a browser with no install |

## 6. Non-goals for V1
- Real-site integration (WES/WCS connectors, live telemetry). V1 explains the twin, not a real floor.
- Robot rotation and kinematics, multi-level floors, human pickers.
- Learned or ML models of any kind.
- Polished visual design. V1 is functional and clear; the renderer is replaceable.

## 7. The 60-second demo (what V1 must deliver)
1. Open the viewer: a medium hub running smoothly at ~7,000 parcels/h.
2. Inject chaos: two robots die in a central aisle and a chute jams. The throughput chart dips.
3. Click a late parcel: its trace shows 41 s blocked, with the blame chain "R112 → R087 → dead robot R031".
4. Run the post-mortem: *"Throughput fell 31 % at 14:02. The R031 failure in aisle 7 explains 78 % of the lost parcels (replay-verified); the C12 jam explains 19 %. Rerouting aisle 7 would have recovered 88 %."*
