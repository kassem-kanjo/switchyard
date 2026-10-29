# Switchyard: positioning

How we describe Switchyard to a client. Plain language, their problem first, no slogans.

## Who it's for
Operators of robotic sortation and fulfilment sites, and the robotics vendors who run those fleets:
- **Site and operations leaders** own throughput and service levels.
- **Reliability and maintenance teams** decide what to fix first.
- **Vendor engineering teams** have to explain an incident to their customer.

## The problem, in their words
- "We lost a third of our throughput at 2 pm and nobody can tell me why."
- "Three things broke at once. Which one actually cost us the late parcels?"
- "Every incident review is a meeting and a guess, and the fix that gets funded is whoever argues best."

Dashboards show *that* throughput dropped. Fleet tools show what one robot is doing. Nothing on the market tells the operator **what caused the loss, what each cause cost, and which fix pays back**.

## What Switchyard does
1. **See it:** every robot and parcel on the floor, with each delay traced to the robot, chute or station that caused it.
2. **Explain it:** when throughput drops, Switchyard re-runs the same shift with each problem removed and measures the difference. The result is cause-by-cause shares with honest error bars, not a correlation chart.
3. **Fix it:** candidate fixes (rerouting, faster clearing of dead robots, station limits, spare robots) are tested against the same shift, so the operator sees which one would have recovered the most before spending anything.
4. **Stress it:** break the floor on purpose (dead robots, jams, surges) to find where it is fragile before a peak day does.

## Why it's credible
- **Proof by re-running, not inference.** Each claim cites the runs behind it.
- **It knows its limits.** When two causes are too close to call, the report says so.
- **Measured accuracy:** 24 of 24 planted incidents; 44 of 48 hard ones, graded independently.
- **It runs on a laptop:** a full incident analysis in about 5 seconds, with no cloud and no data leaving the site.

## What we'd say in the first meeting
> Give us the layout and one week of your floor's logs. We'll set up Switchyard on your site, and for your worst recent incident we'll show you what caused it, what each cause cost and which fix would have paid back. If it's wrong, you'll see that too.

## Words we use, and words we avoid
| Use | Avoid |
|---|---|
| "see when the floor goes wrong, why, and what fixes it" | "digital twin", "AI-powered", "revolutionary" |
| "proven by re-running the shift" | slogans and taglines |
| "what each cause cost" | "insights", "synergy" |

## Business case (to fill in with each client's numbers)
Value per incident ≈ parcels late × cost per late parcel (SLA penalties, re-handling, customer credits), plus avoided wrong fixes. For example, the demo incident made 110 parcels late in 30 minutes, and the best fix recovered 83 % of the delay.
