# Switchyard: industry and client research brief

Date: 2026-10-19. Scope: robotic sortation industry, operator KPIs and tooling, the "why did the shift underperform" pain, prior art, personas.
Method: web search only (no interviews). Tags: [V] vendor/press-release claim, [I] independent or trade press, [INF] my inference, [GAP] searched and not found.
Source IDs in brackets refer to the Sources list at the end.

## 1. Conclusions first

1. Robotic sortation is a mature, vendor-delivered category. Typical hub: 30-430 robots, 1,500-11,000 parcels/h, and per-robot throughput clusters at roughly 25-50 parcels/robot/h (section 4). Numbers are almost all vendor claims, so calibrate to ranges, not points. [S1-S6]
2. Operators have plenty of KPI dashboards (throughput, accuracy, uptime) but I found NO public evidence of a per-parcel causal trace that explains why a shift underperformed. Public RCA practice is CMMS-style downtime coding, post-hoc process mining, or simulation before go-live. [S14, S17, S21]
3. Novelty verdict: the individual ingredients are NOT novel. OpenTelemetry for robot fleets (ROS 2, AGV/AMR metrics), chaos engineering on industrial digital twins (academic papers, one patent) and AI-assisted downtime RCA all exist. The COMBINATION (per-parcel trace as the unit of analysis, chaos with a resilience score on a sort-floor twin, auto-generated post-mortem tied to the parcel traces) was NOT found in any source I could reach. Verdict: moderately novel as a composition and framing, weak as a technology claim. I could not search paywalled analyst reports, vendor demos or patents exhaustively, so "not found" is not "does not exist". [S9, S10, S11, S22]
4. Sharpest framing: do not sell "tracing" or "chaos" as buzzwords. Sell "a shift post-mortem in 60 seconds that names the causal chain (jam at induct 3 -> robots rerouted -> chute 41 starved -> 212 parcels missed cut-off)" and "a resilience score you can show before peak." Evidence the pain is real: UPS 2013 blamed "volume exceeded capacity," a root cause too coarse to act on. [S18]
5. For a hiring panel (Gulf carriers, robotics vendors): realism of the numbers, a credible failure taxonomy (jams, scanner drift, robot deadlock, induction starvation, chute full), and a before/after demo on one concrete incident. See section 7.

## 2. Robotic sortation in practice (sourced findings)

- Geek+ / Athinaiki (Greece, 2021): 29 robots, 562 m2 floor, 104 destination cages, 1,400-1,500 parcels/h. [V] [S1]
- Geek+ / Asda Logistics Services (UK): 60 robots, 2,000 parcels/h, 99.99% accuracy claimed; scale by adding robots and chutes. [V] [S1]
- Geek+ / Paack (Madrid): 270+ sorting and picking robots, 10,000+ deliveries/h (sort and pick combined, not comparable). [V] [S1]
- Geek+ / China Post (Wuhan): 320 robots, 600,000+ parcels/day claimed. [V] [S1]
- Libiao / Correo Argentino (Buenos Aires): 240 robots, up to 9,000 parcels/h, 3x prior capacity. [V] [S5]
- Libiao / Viettel Post (Hanoi): 160 robots, ~6,000 parcels/h. [V] [S5]
- Libiao / THG Fulfil (UK): 11,151 items/h reported (34% above target); trade press cites 430 robots; daily capacity quoted inconsistently (625,000 vs 600,000). [V] [S5]
- Libiao / Kuryenet (Istanbul): 120 robots, 700+ chutes, "45,000 items/h". This implies ~375 per robot/h, an order of magnitude above every other site; treat as an outlier or a different metric. [V] [S5]
- Tompkins t-Sort: up to 36,000 units/h at launch; MODEX 2026 listing claims 40,000 items/h to 6,000+ destinations; micro-fulfilment 2,500/h; systems up to 50,000/h; sets up or breaks down in a single shift. No robot counts or independent data found. [V] [S2]
- Amazon Pegasus (2019): first robot in Amazon sortation buildings; associate scans and places parcel on robot, which travels to an eject station and chutes it; ~2 min per journey; early deployment ~800 units in Denver; claimed 50% fewer mis-sorts; needs "robot traffic control" with flow-control staff watching real-time robot traffic maps. No parcels/h published. Sparrow is item picking, not sortation; Robin is an arm that loads drive units. [I/V] [S3, S4]
- JD Logistics: Shanghai Jiading "Asia No.1" (2017) ~1,000 robots over 40,000 m2, 3,600 items/h cited; Kunshan phase 2 claims 4.5M parcels/day, 10,000 sorting robots, 80+ sorting lines. Other hourly figures (16,000/h) uncorroborated. [I/V] [S6]
- DHL: found only Dorabot arm pilots (Atlanta, >1,000 parcels/h per cell, 3.6 s/parcel; Singapore/Korea DHLBot 1,000 small parcels/h, 99%). No DHL deployment of Geek+/Libiao/Tompkins found. [V] [S7]
- Hai Robotics: storage and case-picking (ACR), not parcel sortation; no Gulf parcel-hub deployment found. [V] [S8]
- Ocado/AutoStore: not researched in depth; Ocado blog notes peak temp staff meeting robots for the first time (training and on-site support are a peak concern). [V] [S19]
- Gulf: Aramex Jeddah Islamic Port robotic hub: 3 feeding lines, 120 robotic guided vehicles, 4,000 shipments/h, 96,000/day (two outlets carry the same announcement). Aramex Riyadh KKIA express site: 5,900 m2, up to 100,000 shipments/day. Aramex Dubai crossbelt spec 9,000 parcels/h (2017 schedule). [V] [S15]
- Gulf: SPL (Saudi Post) with IQ Robotics and AlKhereiji Group: 200 robots at a Riyadh sorting centre, billed as the largest robotic sorting centre in the Middle East; SPL "super sorting center" announced Oct 2024, status and throughput unconfirmed. [V/I] [S16]
- [GAP] SMSA Express and Naqel: no public robot or throughput data found.

## 3. KPIs and software today

KPIs commonly cited: sortation accuracy / mis-sort rate, throughput, labour minutes per 1,000 items, dock dwell time, on-time departures, first-attempt delivery. [S13] Honeywell advertises a mis-sort target below 0.1% (vendor, not a benchmark). [S13] Amazon claims Pegasus halved mis-sorts. [S4]
- [GAP] No published benchmark for sorter availability or parcel-hub OEE; the "99.5%" figure I tested was not supported anywhere.
- Unplanned downtime causes named by a maintenance-software vendor: belt/roller wear, divert-arm misalignment, sensor faults, deferred PM; an uncalibrated scanner can mis-sort with no alarm. All vendor claims, unverified. [S14]
- Software layers: WES/WCS (vendor-specific), robot fleet manager (vendor-specific, e.g. Pegasus traffic maps), CMMS for maintenance, simulation for design. [S3, S14, S20]
- Simulation / digital twin: FlexSim (Autodesk) 3D discrete-event with scenario manager; Emulate3D (Rockwell) controls-level emulation and pre-commissioning validation; AnyLogic multimethod (agent-based plus discrete-event). Mostly design-time tools. [V] [S20]
- A WSC 2023 paper ran an AMR-warehouse simulation twin before each shift to set robot counts and order release, reporting ~4.6% average efficiency gain. [I/academic] [S17]
- Process mining: Celonis in warehouses is thinly documented; one case (MUELLER Die Lila Logistik with INFOMOTION) cites transparency and many process variants but no quantified result and no WES link. Celonis conformance checking is described for deviation-vs-throughput-time analysis in a procurement context. [V] [S21]
- Fleet observability: Formant (live fleet state, alerts, incident management) and Foxglove (record/replay of MCAP/ROS logs) exist but are robot-developer tools, with no warehouse sortation case studies found. [V] [S22]

## 4. Calibration numbers for the simulation

Derived per-robot rates (my arithmetic from the sourced pairs above, [INF]):

| Site | Robots | Parcels/h | Per robot/h |
|---|---|---|---|
| Athinaiki (Geek+) | 29 | 1,450 | ~50 |
| Asda (Geek+) | 60 | 2,000 | ~33 |
| Viettel Post (Libiao) | 160 | 6,000 | ~37 |
| Correo Argentino (Libiao) | 240 | 9,000 | ~37 |
| Aramex Jeddah | 120 | 4,000 | ~33 |
| THG (Libiao, items) | 430 | 11,151 | ~26 |
| Kuryenet (Libiao) | 120 | 45,000 | ~375 (outlier, exclude) |

Suggested simulation defaults:
- Per-robot design rate 25-50 parcels/h; central default ~35/h. Robots: small 30-60, medium 120-250, large 300-450. Cap the hero scenario near 450 (largest credible single-site figure found; JD Kunshan is a campus with 10,000 robots).
- Hub throughput bands: small 1,500-2,000/h, medium 4,000-9,000/h, large ~11,000/h. Treat 36,000-45,000/h claims as a stress tier only.
- Chutes/destinations: ~100 (Athinaiki) to 700+ (Kuryenet); Tompkins claims 6,000+. Default medium hub 300-500 chutes; robots-to-chutes roughly 1:2 to 1:4 from the sites with both numbers (29:104, 120:700+). [INF]
- Induction: three feeding lines at Aramex Jeddah for 4,000/h gives ~1,300/h per line [INF]; Pegasus is manual scan-and-place, so induction is a human-rate bottleneck. [S4] Model induction as the first starvation point.
- Robot trip time ~2 min (Pegasus) as the sort-leg duration anchor. [S4]
- Accuracy / mis-sort: baseline 0.01-0.1% (vendor claims 99.99% Asda, <0.1% Honeywell); a scanner-drift fault should push this well above 0.1% silently. [S1, S13]
- Failure taxonomy to inject: divert/eject jam, scanner miscalibration (silent mis-sort), robot deadlock/congestion, WES-to-fleet latency or outage, chute full, induction starvation, battery/charging contention, network partition. Sources: [S14, S4, S17].
- Mark every default in code as "vendor-claimed, range-calibrated, not measured."

## 5. The pain: how do they find out why a shift underperformed?

What the evidence shows (thin; this is the biggest honesty caveat):
- Public explanations of big failures are coarse. UPS 2013: volume "exceeded the capacity of our network"; critics paraphrased this as "demand was higher than forecast"; anonymous sources added air-fleet, weather and compressed calendar. No formal post-mortem published. [I] [S18]
- Amazon's Pegasus floors rely on human flow-control staff watching real-time robot traffic maps on tablets: congestion diagnosis is visual and real-time, not causal and retrospective. [I] [S4]
- Maintenance-side practice is failure-code libraries and per-event RCA in a CMMS (vendor claims 79% fewer repeat downtime events; unverified). [V] [S14]
- Generic "warehouse outage postmortem" guides exist but are 3PL marketing with unverifiable stats. [V] [S19]
- Stress testing before peak is recommended (DC Velocity) and Locus publishes a peak-readiness checklist, but no sortation-specific chaos or resilience scoring appeared. [V] [S19]
- [GAP] I found no first-person operator quote about how they actually diagnose a missed shift. Primary research needed: 5-8 interviews (section 8). Do not fabricate quotes in the portfolio.
- [INF] Likely reality: shift-lead notes, WCS alarm exports, vendor support tickets, spreadsheets of hourly throughput. Treat as a hypothesis to validate.

## 6. Prior art and competitors

| Who / what | What it does | Overlap with Switchyard | Source |
|---|---|---|---|
| OneUptime AGV monitoring post | OpenTelemetry metrics for AGV fleets: mission completion, navigation latency, utilisation | Same tool family; metrics, not per-parcel traces; blog, not product | S9 |
| ROS 2 x OpenTelemetry library | End-to-end OTel spans for ROS 2 nodes; complements ros2_tracing | OTel on robots is accepted; scope is software nodes, not parcels | S9 |
| AIVA robotic systems (GrafanaCON 2024) | OTel + Grafana LGTM correlating cloud, edge and robot firmware telemetry | Same stack idea on a robot; not a sort floor | S9 |
| Formant / Foxglove | Fleet observability; log record/replay | Closest commercial "observability for robots"; robot-centric, no parcel causality | S22 |
| Univ. Ferrara (2024), ChaosTwin (2021) | Chaos engineering profiles for industrial digital twins | Direct academic prior art for chaos on twins; not warehouse, no score product | S10 |
| US patent 11409272 | Fault situations in a real facility and its cloud twin via chaos tool on edge | Prior art on the mechanism; industrial plant, not sortation | S10 |
| Orive et al. ETFA 2019; Larsen et al. 2023 | Fault injection in digital twins / co-simulation for robots | Academic fault injection; no hub resilience scoring | S11 |
| FlexSim, Emulate3D, AnyLogic | Design-time simulation, scenario manager, controls emulation | Compete for the "twin" budget line; none advertise per-parcel trace RCA | S20 |
| CreateASoft | Digital twin as live monitor/alert after rollout (vendor claim) | Closest to a "live twin"; marketing-level evidence | S17 |
| Celonis / process mining | Conformance and root-cause on event logs | Same insight (event logs -> why), different substrate | S21 |
| OxMaint-style CMMS AI | Downtime RCA, predictive maintenance for sorters | Overlap on equipment RCA; asset-centric, not parcel-centric | S14 |
| Dematic / Honeywell Intelligrated software | Vendor analytics and controls | [GAP] could not retrieve internals; assume dashboards and alarms exist | S13 |

Honest novelty assessment:
- Not novel: OTel on robots, chaos on twins, AI downtime RCA, digital twin for peak planning.
- Plausibly novel: (a) the parcel as the trace (journey from induct through robot legs to chute as spans, with causes attached), (b) a resilience score for a sort hub computed from chaos runs, (c) auto post-mortems citing those traces, (d) all three in one workflow.
- Risk: a panel member who knows Foxglove/Formant or SRE practice may say "this is observability applied to robots." Pre-empt by naming that lineage openly and showing what is new (parcel causality, hub-level resilience score).

## 7. Personas and the "wow" test

Personas are [INF] from role logic, not interviewed; validate before claiming.
- Site operations manager (buyer-adjacent): owns daily cut-off performance and SLA penalties. Wants "why did we miss today" in one screen and peak-readiness confidence. Pain: coarse explanations (cf. S18).
- Shift lead (daily user): watches live flow, clears jams, reroutes. Wants a ranked list of "what is hurting throughput right now." Role evidence: Pegasus flow-control staff watch traffic maps live. [S4]
- Network planner (economic buyer/influencer): sizes robots, chutes and hubs before peak; already uses FlexSim-class tools. Wants a resilience score per hub and scenario, comparable across sites. [S17, S20]
- Robotics / controls engineer (technical gatekeeper): integrates vendor fleet manager and WES; cares about OTel export, fault taxonomy, no lock-in. Knows ROS, Foxglove, Grafana. [S9, S22]
- Buying context: Gulf carriers are expanding with vendor-delivered robotics (Aramex 120 robots, SPL 200 robots), so integrators and carriers are the likely audience for a portfolio demo. [S15, S16]

What would make a hiring panel say "wow" [INF]:
1. A single replayable incident: inject scanner drift or a deadlock, watch the hub degrade, click a missed parcel, see its trace, and get an auto post-mortem naming root cause and blast radius.
2. Numbers that match published reality (section 4), with calibration sources visible in the UI.
3. A resilience score with a clear, defensible definition and a before/after comparison of two configurations (e.g. 120 vs 150 robots, or WES failover on/off).
4. Honest scoping: states what is simulated, what is vendor-claimed, and how a real hub would feed OTel data in.
5. Gulf framing: Aramex Jeddah-sized (120 robots, 4,000/h) and SPL-sized (200 robots) preset scenarios.

## 8. How to sharpen differentiation

- Lead with the artifact, not the stack: "post-mortem in 60 seconds" and "resilience score before peak."
- Make the parcel the primary key: every anomaly must be answerable as "which parcels, which cause, how many late."
- Define the resilience score explicitly (for example: % of design throughput retained and time-to-recover under a fixed fault catalog) and publish the catalog.
- Cite lineage (SRE, chaos engineering, OTel) and the academic twin/chaos work (S10, S11) so the claim is "applied and productised for sort floors," not "invented."
- Contrast explicitly with the three incumbents: design-time twins (FlexSim/Emulate3D), robot-dev observability (Formant/Foxglove), post-hoc process mining (Celonis). Switchyard sits at operations-time causal explanation plus pre-peak stress.
- Validate with 5-8 operator interviews (shift leads, planners) to replace the [INF] personas and fill the quote gap; until then do not present persona claims as fact.

## 9. Gaps and caveats

- Search was US-oriented and returned many vendor/marketing pages; several statistics (UPS Columbus pages, 79% repeat-downtime, "40% of outages") were unverifiable and excluded or caveated.
- No data for SMSA, Naqel, Hai Robotics parcel hubs, Honeywell/Dematic software internals, Amazon Pegasus parcels/h, or DHL use of Geek+/Libiao/Tompkins.
- Throughput numbers are claims on different bases (items vs parcels, peak vs design, sort-only vs sort+pick).

## Sources
- S1 Geek+ (Athinaiki, Asda, Paack, China Post): https://www.geekplus.com/resources/news/geek-and-fdl-group-accelerate-e-commerce-in-greece-and-cyprus-with-smart-sorting-robots ; https://www.ajot.com/news/geek-and-amh-material-handling-deliver-robotic-sortation-project-with-asda-logistics-services ; https://siliconcanals.com/paack-deploys-ai-logistics-robots/ ; https://www.geekplus.com/case-studies/geek-and-china-post-revolutionize-logistics-with-smart-sorting-technology
- S2 Tompkins t-Sort: https://cargofacts.com/allposts/tompkins-international-releases-t-sort-robot-fulfillment-technology/ ; https://www.robotics247.com/photo/modex2026-cutting-edge-warehouse-robots-AGV-AMR-ASRS-cobot-humanoid/32/ ; https://www.mmh.com/article/tompkins_robotics_t_sort_tackles_micro_fulfillment/Sortation
- S3 Amazon robots: https://www.aboutamazon.com/news/operations/new-robots-new-jobs ; https://retaildive.com/news/amazon-deploys-automated-warehouse-robot-sparrow/636607
- S4 Pegasus: https://www.dcvelocity.com/articles/20190606-amazon-unveils-xanthus-and-pegasus-fulfillment-robots/ ; https://venturebeat.com/ai/amazon-says-its-new-pegasus-fulfillment-center-robot-improves-throughput-by-50/
- S5 Libiao: https://www.thgingenuity.com/resources/case-studies/libiao-scaling-through-smart-sortation ; https://logisticsbusiness.com/materials-handling/amr-agv/first-fully-robotic-parcel-hub-in-argentina/ ; https://tlimagazine.com/news/europes-largest-robotic-sorting-system-goes-live
- S6 JD: https://www.chinadaily.com.cn/regional/bda/2017-10/16/content_33357616.htm ; https://postandparcel.info/?p=153247 ; https://yicaiglobal.com/news/jdcom-starts-operations-at-its-first-staff-less-sorting-center-near-shanghai
- S7 DHL/Dorabot: https://www.robotics247.com/article/dhl_successfully_pilots_dorabot_sorting_robots_atlanta_distribution_center ; https://portcalls.com/dhl-express-deploys-ai-powered-sorting-robot-sg-korea/
- S8 Hai Robotics: https://www.therobotreport.com/hai-robotics-raises-200m-autonomous-case-handling-robots/
- S9 OTel and robots: https://oneuptime.com/blog/tag/agv ; https://discourse.openrobotics.org/t/ros2-x-opentelemetry-end-to-end-telemetry-for-robotics/50658 ; https://grafana.com/events/grafanacon/2024/aiva-robotic-systems-observing-smart-robots-with-opentelemetry-and-grafana-lgtm-stack/
- S10 Chaos on twins: https://iris.unife.it/handle/11392/2510654 ; https://iris.unife.it/handle/11392/2470605 ; https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/11409272 ; https://researcher.watson.ibm.com/publications/chaos-engineering-using-dt-lessons-learned-from-a-client-engagement
- S11 Fault injection: https://ekoizpen-zientifikoa.ehu.eus/documentos/5ee555bc29995234a5df0a0f ; https://apps-eprint4-l.ncl.ac.uk/294768
- S13 Honeywell and KPI lists: https://automation.honeywell.com/us/en/products-backup/warehouse-automation/solutions-by-technology/sortation-systems ; https://racklify.com/encyclopedia/what-is-last-mile-sortation-definition-benefits-and-core-components/
- S14 Sorter downtime / CMMS RCA (vendor): https://oxmaint.com/industries/delivery-operations-management/parcel-sorter-maintenance-software ; https://oxmaint.com/blog/post/warehouse-conveyor-downtime-root-cause-cmms
- S15 Aramex: https://www.gccbusinessnews.com/?p=109702 ; https://www.parcelandpostaltechnologyinternational.com/news/sorting-systems/aramex-opens-express-courier-handling-facility-at-king-khalid-international-airport.html ; https://postandparcel.info/?p=83212
- S16 SPL: https://www.arabnews.com/node/2575136/amp ; https://www.gdnonline.com/Details/1301678/IQ-Robotics,-SPL-and-AlKhereiji-Group-to-deploy-200-robots ; https://enterpriseam.com/logistics/2024/02/01/iq-robotics-spl-and-al-khereiji-ink-landmark-robotics-agreement-for-processing-and-sorting/
- S17 AMR twin / live twin: https://www.informs-sim.org/wsc23papers/cea157.pdf ; https://www.createasoft.com/company/digital-twin-and-simulation-blog/category/blog/you-shouldnt-let-your-amr-provider-size-your-fleet ; https://industrialautomationindia.in/editorschoice/digital-twins-amr-fleets-timing-value-guide
- S18 UPS 2013: https://www.cnbc.com/2013/12/27/ups-fiasco-may-be-due-to-shrunken-fleet-report.html ; https://abcnews.go.com/blogs/headlines/2013/12/ups-overload-leaves-miffed-customers-short-of-presents-meals/ ; https://www.industryweek.com/supply-chain/santa-thwarted-ups-fedex
- S19 Peak/postmortem guidance (vendor/blog): https://fulfillmenthubusa.com/the-ultimate-guide-to-conducting-effective-warehouse-outage-postmortems/ ; https://ocadointelligentautomation.com/insights/blog-how-we-prepare-your-warehouse-for-peak-season ; https://locusrobotics.com/wp-content/uploads/2025/08/Is-Your-Warehouse-Peak-Ready_-A-Strategic-Checklist.pdf
- S20 Simulation tools: https://www.autodesk.com/solutions/design-manufacturing/warehouse-simulation ; https://dev-aem.rockwellautomation.com/en-ca/industries/warehouse-fulfillment.html ; https://gitnux.org/best/warehouse-modeling-software/
- S21 Process mining: https://barc.com/wp-content/uploads/2025/04/Case_Study_Lila-Logistik-2.pdf ; https://community.celonis.com/process-analysis-5/process-ai-to-detect-which-variables-are-leading-to-a-throughput-higher-than-x-days-2735
- S22 Formant / Foxglove: https://docs.formant.io/docs/fleet-observability ; https://docs.foxglove.dev
