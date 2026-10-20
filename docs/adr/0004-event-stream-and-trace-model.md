# 0004. The event stream as the core's only output; the parcel-trace model

**Status:** accepted · **Date:** 2026-10-19 · **Research:** [reliability methods](../research/2026-10-19-reliability-methods.md)

## Context
Traces, metrics, the viewer and post-mortems all need the same facts. Coupling them to sim internals would make every new analysis a change to the sim.

## Decision
- `core` emits a typed `Event` stream: arrivals, assignments, phase transitions, robot waits with direct and root causes, drops, mis-sorts, chute state, faults starting and ending. That stream is its only output; everything else is derived.
- **Trace model:** one trace per parcel. Its phase spans (`Queued`, `AwaitingRobot`, `Loading`, `Travelling`, `Blocked`, `ChuteWait`) tile its lifetime exactly, which is enforced by a property test. `Blocked` spans carry `direct_blocker` and `root_cause`, and act as an OpenTelemetry-style link to the blocking robot's work.
- **Exports:**
  - Chrome trace-event JSON, which opens in Perfetto with no setup, with one track per robot and per station
  - OTLP JSON (`resourceSpans`) for OpenTelemetry tooling such as Jaeger
  - JSONL for DuckDB and Python
- Parquet is deferred to V2.

## Alternatives considered
- **Spans emitted directly by the sim:** that couples the sim to one observability model.
- **The OpenTelemetry SDK inside the sim:** heavy, wall-clock-based, and not deterministic.

## Consequences
- Adding a new view or analysis needs no change to the sim.
- Large runs produce many events; consumers stream them and the viewer queries traces on demand.
