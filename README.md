# Switchyard

**See when your sort floor goes wrong, why, and what fixes it.**

[![CI](https://github.com/kassem-kanjo/switchyard/actions/workflows/ci.yml/badge.svg)](https://github.com/kassem-kanjo/switchyard/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/kassem-kanjo/switchyard)](https://github.com/kassem-kanjo/switchyard/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-black.svg)](LICENSE)

<!-- Hero GIF for later: docs/media/demo.gif -->
![The Switchyard console: the floor, the incident headline and the cause-by-cause breakdown](docs/checkpoints/img/v2-console-1440.png)

When a parcel hub loses a third of its throughput, nobody can say for sure what caused it or what each problem cost. Switchyard shows every robot and parcel on your floor, traces each delay to the robot, chute or station that caused it, and re-runs the same shift with each problem removed to measure what comes back. You get the cause-by-cause cost, honest error bars, and the fix that would have recovered the most, before you spend anything.

It is for operations leaders who own throughput and service levels, and for robotics vendors who have to explain an incident to their customer.

**Video:** [Switchyard in 52 seconds](docs/media/switchyard-demo.mp4) | **Paper:** [Proving what broke a robotic sort floor](docs/paper/switchyard-v1.pdf) (3 pages: how it works and what it found)

## Run it in one command

[Download](https://github.com/kassem-kanjo/switchyard/releases) the `switchyard` binary for Windows, macOS or Linux, then:

```bash
switchyard view
```

This serves the console on your own machine (localhost only). Nothing leaves the computer and nothing needs installing.

## What it does

- **Simulates** a sort floor robot by robot: 200 robots moving about 6,300 parcels an hour. Two hours of floor time take half a second on a laptop.
- **Traces every parcel.** Every robot that waits records why, so each late parcel shows where its time went and who held it up.
- **Breaks the floor on purpose:** dead robots, blocked aisles, jammed chutes, induction outages, volume surges.
- **Writes the post-mortem.** It finds the incident, splits the blame between causes, says which fix would have helped most, and proves each claim with a replay.

## One incident, explained

A chute jams and two robots die in the same minute. Throughput falls 47 %, and 110 parcels end up more than ten minutes late. Switchyard runs 96 replays in about five seconds:

![The jam caused 70 % of the delay; ten spare robots would have fixed nothing](docs/media/incident.png)

The two robot failures are too close to rank, so the report calls them a tie instead of inventing an order. The usual reflex, sending more robots, would have done nothing, because the bottleneck was a chute. Clearing dead robots in one minute instead of five was worth 28 %, and it costs nothing.

## How it works

1. **A simulation you can rewind.** The simulation is fully deterministic: the same inputs give exactly the same shift, bit for bit, on Windows, macOS, Linux and WebAssembly. So any difference between two runs comes from the one thing that changed.
2. **Replay the day without each problem.** For an incident, every combination of active faults is removed and the shift is replayed from the nearest checkpoint. Blame is split fairly with the Shapley value from game theory. The same replays show whether two faults overlapped or made each other worse, and test candidate fixes on the real day.
3. **Honest error bars.** A deterministic simulation gives one exact answer, which looks precise but isn't: remove a fault and the robots take slightly different paths. So Switchyard replays several equally plausible versions of the day (one robot pauses for half a second at a moment that shouldn't matter) and reports the spread. Anything inside it is called a tie or "nothing measurable".

```mermaid
flowchart LR
    S[Scenario + faults] --> C[core<br/>deterministic sim<br/>PIBT planner + wait records]
    C --> T[trace<br/>per-parcel blame<br/>Perfetto / OTLP]
    C --> A[analysis<br/>incident detection<br/>replays + Shapley<br/>post-mortem]
    T --> A
    A --> L[switchyard CLI]
    A --> V[local viewer]
```

Design decisions are in [docs/adr](docs/adr/); the full design is in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Results

| Check | Result |
|---|---|
| Planted incidents with one clear cause | **24 / 24** named correctly |
| Hard incidents (near ties, overlapping and compounding faults), graded against an independent check | **44 / 48** main cause right; large interactions **24 / 24** |
| Speed | 2 h of a 200-robot floor in **0.5 s**; a full post-mortem (96 replays) in **≈ 5 s** |
| Determinism | identical state hash on Linux, Windows, macOS and WebAssembly, checked in CI |
| Tests | 255 Rust tests and 58 viewer tests, plus property tests for collisions, parcel conservation and trace integrity |

Small interactions, the ones under about 500 parcel-minutes, sit inside the floor's own noise. The report says so instead of guessing.

![Averages invent fixes](docs/media/averages.png)

The most useful finding came from a bug. Two averages taken over different versions of the day made rerouting look better than removing the jam itself. Played the same day both ways, the two options tie. Any team that compares "days we did A" with "days we did B" can invent a fix this way. More findings: [docs/insights.md](docs/insights.md).

## Command-line use

If you prefer to build from source (Rust, stable):

```bash
git clone https://github.com/kassem-kanjo/switchyard && cd switchyard
cargo build --release -p switchyard-cli
```

Then:

```bash
# Write a post-mortem for a chaos preset
switchyard postmortem --scenario two-dead-robots-and-a-jam --ticks 14400 --out report.md

# Simulate 2 hours of the 200-robot hub (1 tick = 0.5 s)
switchyard run --scenario medium --ticks 14400

# Inject your own faults and score the floor against its fault-free run
switchyard chaos --scenario medium --ticks 14400 \
  --fault "robot-failure:robot=31,at=30m,for=20m" \
  --fault "chute-jam:chute=12,at=30m,for=15m"

# Export per-parcel traces and open them in https://ui.perfetto.dev
switchyard export --scenario medium --ticks 14400 --format perfetto --parcels late:p95 --out traces.json
```

Presets: `small`, `medium`, `large`, plus the chaos presets `aisle-7-blocked`, `peak-surge` and `two-dead-robots-and-a-jam`. Fault kinds: `robot-failure`, `aisle-block`, `chute-jam`, `induction-outage`, `volume-surge`, `scanner-drift`.

**Viewer from source.** `switchyard view` already includes the console. To develop it, you need Node 22:

```bash
cd viewer && npm ci && npm run dev
```

Full setup, including Windows: [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## How it was built, step by step

Each stage is one reviewed pull request, so the history reads as the roadmap:

| Stage | What it added |
|---|---|
| [Discovery](https://github.com/kassem-kanjo/switchyard/pull/1) | Research, product spec, architecture and decision records |
| [V0](https://github.com/kassem-kanjo/switchyard/pull/2) | Walking skeleton: the sim, the CLI and the viewer, end to end |
| [CP1](https://github.com/kassem-kanjo/switchyard/pull/3) | A floor that sorts: the guided PIBT planner and floor operations |
| [CP2](https://github.com/kassem-kanjo/switchyard/pull/4) | Parcel traces with blame chains, the fault catalog, chaos runs and a resilience score |
| [CP3](https://github.com/kassem-kanjo/switchyard/pull/5) | The post-mortem engine: incident detection, replays, Shapley blame, tested fixes |
| [V1 depth](https://github.com/kassem-kanjo/switchyard/pull/27), [hardening](https://github.com/kassem-kanjo/switchyard/pull/28) | Per-cause noise bands, the reroute check and the security review |
| [v1.0.0](https://github.com/kassem-kanjo/switchyard/releases/tag/v1.0.0) | The release, plus the [paper](https://github.com/kassem-kanjo/switchyard/pull/30) |
| [V2 plan](https://github.com/kassem-kanjo/switchyard/pull/37), [wireframes](https://github.com/kassem-kanjo/switchyard/pull/41) | Design spec, ADR 0007, research and wireframes for review |
| [V2 CP1](https://github.com/kassem-kanjo/switchyard/pull/39) | Headline card, folded report, plain wording |
| [V2 CP2](https://github.com/kassem-kanjo/switchyard/pull/40), [#45](https://github.com/kassem-kanjo/switchyard/pull/45), [#46](https://github.com/kassem-kanjo/switchyard/pull/46), [#48](https://github.com/kassem-kanjo/switchyard/pull/48) | Design tokens, chart, floor legibility, the operations console layout, first-run guide |
| [`switchyard view`](https://github.com/kassem-kanjo/switchyard/pull/38), [release builds](https://github.com/kassem-kanjo/switchyard/pull/47) | The console served from the CLI binary; Intel-Mac cross-build |

Each checkpoint went through an independent review; the reports are in [docs/checkpoints](docs/checkpoints/).

## Repo map

| Path | Contents |
|---|---|
| `crates/core` | Deterministic sim, PIBT planner, faults, causal wait records |
| `crates/trace` | Per-parcel traces, blame chains, exporters |
| `crates/analysis` | Incident detection, replays, Shapley attribution, post-mortem |
| `crates/cli` | The `switchyard` command |
| `crates/wasm`, `viewer/` | The local visual viewer |
| `scenarios/` | Floor presets and chaos presets |
| `docs/` | [Paper](docs/paper/switchyard-v1.pdf), [architecture](docs/ARCHITECTURE.md), [decision records](docs/adr/), [research](docs/research/), [checkpoint reviews](docs/checkpoints/), [insights](docs/insights.md) |

## Limits

The floor is simulated, not connected to a real hub yet. The next step is running the same replays on a copy of a real shift from a hub's logs. Small interactions between faults stay inside the noise. Exact Shapley values grow exponentially with the number of causes, so incidents with many causes at once will need sampled attribution.

## Built with AI

Built by Kassem Kanjo, with a team of AI agents: cheap models wrote well-specified code, stronger models reviewed it, and an independent auditor checked every checkpoint. The audit findings are published in [docs/checkpoints](docs/checkpoints/). The paper has a section on what AI made possible here, and where a human still had to catch it.

## License

[MIT](LICENSE)
