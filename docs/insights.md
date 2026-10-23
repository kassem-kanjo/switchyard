# Insights from building Switchyard

Raw notes for thought pieces (short, co-written with Kassem). Add as we learn.

- **A sort floor fails like a distributed system.** Most damage from a fault spreads as congestion through robots that never failed, so the root cause is rarely where the alarm is.
- **Where a robot dies matters ~10× more than that it died.** A dead robot on an induction cell cost about ten times one in an aisle.
- **Blame has to be proven, not guessed.** Replaying the shift without a fault gives a causal answer; Shapley values split shared blame fairly between faults that overlap.
- **Honest uncertainty is a feature.** Re-running with tiny perturbations gives a noise band; many "causes" a dashboard would rank are inside the noise.
- **Some faults are redundant, some compound.** Two identical jams overlap (fixing one barely helps); a surge plus a jam compounds (worse together than apart).
