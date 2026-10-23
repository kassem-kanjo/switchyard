# Insights from building Switchyard

Raw notes for thought pieces (short, co-written with Kassem). Add as we learn.

- **A sort floor fails like a distributed system.** Most damage from a fault spreads as congestion through robots that never failed, so the root cause is rarely where the alarm is.
- **Where a robot dies matters ~10× more than that it died.** A dead robot on an induction cell cost about ten times one in an aisle.
- **Blame has to be proven, not guessed.** Replaying the shift without a fault gives a causal answer; Shapley values split shared blame fairly between faults that overlap.
- **Honest uncertainty is a feature.** Re-running with tiny perturbations gives a noise band; many "causes" a dashboard would rank are inside the noise.
- **Some faults are redundant, some compound.** Two identical jams overlap (fixing one barely helps); a surge plus a jam compounds (worse together than apart).

## Surprises from the build (edge cases are real-world problems in miniature)
- **"Identical" faults aren't.** Two outages of equal length at different induction stations cost 3,930 vs 4,690 parcel-min. A station's position on the floor sets its cost; a uniform maintenance SLA is the wrong contract. (planted_hard, first design)
- **Our intuition about overlap was wrong.** An outage plus a surge at the same station *compounded* in 8 of 8 runs; we expected them to overlap. Ops teams reasoning by intuition will misjudge combined incidents. (planted_hard)
- **The floor absorbs small shocks.** Most single-robot pauses change nothing: the system re-routes around them. Resilience is real, but it also hides how close to the edge a floor runs. (#14 dither)
- **A workaround can beat fixing the cause.** Rerouting a jammed chute's parcels recovered more than removing the jam itself, "Find the root cause" may not always be the best move. *To verify: the Auditor suspects this is partly a median artefact (#22).*
- **Faults have long memories.** A 2-minute jam 40 minutes earlier still showed up as a candidate cause, and the engine had to learn to forget it. Real post-mortems over-blame old events the same way. (#21 audit)
- **Averages lie in headlines.** The averaged loss said 87 late parcels; what actually happened was 110. Report what happened, use the spread only for confidence. (#20 audit)
