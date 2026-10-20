# Switchyard: agent entry point

**Product:** Switchyard · **Industry:** logistics · **Thesis:** A sortation hub is a distributed system. Observability, chaos engineering and automatic post-mortems for robotic parcel sortation.

## Before you do anything
1. The company handbook lives in the sibling repo `studio-hq` (private, `kassem-kanjo/studio-hq`). Read its `CLAUDE.md` first: persona, process, org and model-selection rules apply here.
2. Read `docs/STATUS.md` in this repo: the current phase, what's in flight, what's next.
3. Then read only what your task needs: `docs/PRODUCT.md` (spec), `docs/ARCHITECTURE.md`, `docs/adr/`, `docs/research/`.

## Repo map
| Path | Contents |
|---|---|
| `docs/STATUS.md` | Living status (≤ 120 lines). Update it at the end of every session |
| `docs/PRODUCT.md` | Product spec: client, problem, insight, metrics, scope |
| `docs/ARCHITECTURE.md` | Components, languages, data flow, invariants |
| `docs/adr/` | Architecture decision records |
| `docs/research/` | Prior art and research briefs, with sources |
| `docs/checkpoints/` | Checkpoint gate reports (visual, code, tests) |

## Rules
- Product-grade material only. Company or planning material goes in `studio-hq`.
- Branch, open a PR, keep CI green, push at the end of every session.
