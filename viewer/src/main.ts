import { Canvas2DRenderer } from "./renderer/canvas2d";
import { MockSource, type SimSource } from "./sim/source";
import { legendEntries, activeParked } from "./legend";
import { theme } from "./theme";
import { parsePreset } from "./preset";
import { WasmSource } from "./sim/wasmSource";
import "./ui/app.css";
import "./ui/console/console.css";
import { applyThemeVars } from "./theme";
import { createMockServices } from "./data/mock";
import { createWasmServices } from "./data/wasmServices";
import type { Services } from "./data/types";
import { TracePanel } from "./ui/TracePanel";
import { FaultPanel } from "./ui/FaultPanel";
import { PostmortemPanel } from "./ui/PostmortemPanel";
import { mountTabs } from "./ui/tabs";
import { fitToCanvas } from "./renderer/fit";
import { RobotState } from "./renderer/Renderer";
import { chuteCells, floorMarks, incidentCells, incidentFrame, jammedCellsOf, type Frame } from "./renderer/floorMarks";
import { h } from "./ui/dom";
import { Rail } from "./ui/console/Rail";
import { Timeline } from "./ui/console/Timeline";
import { Inspector } from "./ui/console/Inspector";
import { clockOf, parseReport, railItems, type ConsoleData } from "./ui/console/reportData";
import { consoleTimeline, dipVsTwin, shiftClock } from "./ui/console/consoleModel";
import { focusRect } from "./ui/console/floorFocus";
import type { ObjectId } from "./ui/console/types";
import { TickPacer } from "./sim/pacer";

applyThemeVars();

const canvas = document.getElementById("canvas") as HTMLCanvasElement;
const playBtn = document.getElementById("play") as HTMLButtonElement;
const speedBtns = [...document.querySelectorAll<HTMLButtonElement>("#speeds [data-speed]")];
const siteEl = document.getElementById("site") as HTMLSpanElement;
const dipEl = document.getElementById("dip") as HTMLSpanElement;
const chipEl = document.getElementById("incident-chip") as HTMLSpanElement;
const focusChip = document.getElementById("focus-chip") as HTMLSpanElement;
const focusEl = document.getElementById("focus") as HTMLDivElement;
const tickEl = document.getElementById("tick") as HTMLSpanElement;
const statsEl = document.getElementById("stats") as HTMLSpanElement;
const fleetEl = document.getElementById("fleet") as HTMLSpanElement;
const presetSel = document.getElementById("preset") as HTMLSelectElement;
const legendEl = document.getElementById("legend") as HTMLDivElement;
const preset = parsePreset(window.location.search);
const badgeEl = document.getElementById("badge") as HTMLSpanElement;

/** Latest sim tick, read by the mock services and the fault panel. */
let nowTick = 0;

/** The real sim and services, or, if the WASM cannot load, mock ones behind the visible badge. */
async function openSource(): Promise<{ source: SimSource; services: Services }> {
  try {
    const source = await WasmSource.create(preset);
    return { source, services: createWasmServices(source.client) };
  } catch (err) {
    console.warn("WASM sim unavailable, falling back to mock data:", err);
    badgeEl.hidden = false;
    return { source: new MockSource(), services: createMockServices(() => nowTick) };
  }
}

async function main(): Promise<void> {
  const { source, services } = await openSource();
  presetSel.value = preset;
  presetSel.addEventListener("change", () => {
    const url = new URL(window.location.href);
    url.searchParams.set("preset", presetSel.value);
    window.location.href = url.toString();
  });
  for (const e of legendEntries()) {
    const item = document.createElement("span");
    item.className = "item";
    const sw = document.createElement("i");
    if (e.kind === "outlined-robot") {
      sw.className = "outlined";
      sw.style.background = e.colour;
      sw.style.borderColor = theme.idleOutline;
    } else {
      sw.style.background = e.colour;
      if (e.kind === "cell") sw.style.borderRadius = "2px";
    }
    item.append(sw, e.label);
    legendEl.append(item);
  }
  const renderer = new Canvas2DRenderer();
  const layout = source.layout();
  renderer.init(canvas, layout);

  // Inspector. Data providers come from the same worker as the floor.
  siteEl.textContent = `${layout.width}×${layout.height} floor · ${source.snapshot().robotState.length} robots`;
  const panels = { selection: h("div"), trace: h("div"), faults: h("div") };
  document.getElementById("side")!.append(panels.selection, panels.trace, panels.faults);
  const tabs = mountTabs(
    document.getElementById("tabbar")!,
    [
      { id: "selection", label: "Selection" },
      { id: "trace", label: "Trace" },
      { id: "faults", label: "Faults" },
    ],
    panels,
  );
  const tracePanel = new TracePanel(panels.trace, services.traces);
  const faultPanel = new FaultPanel(panels.faults, services.faults, () => nowTick);

  // Console state: the post-mortem data and the one shared selection.
  let data: ConsoleData | null = null;
  let selected: ObjectId | null = "inc";
  const incidentHost = h("div");
  const inspector = new Inspector(panels.selection, incidentHost);
  const rail = new Rail(document.getElementById("rail")!, (id: ObjectId) => select(id));
  const timeline = new Timeline(document.getElementById("timeline")!, {
    onSelect: (id: string) => select(id),
    // The sim only runs forward, so scrubbing reads any minute of the run so far rather than seeking.
    onScrub: (min: number) => {
      const s = services.metrics.series();
      const i = Math.floor(min) - s.startMinute;
      const a = s.delivered[i];
      const tw = s.twin?.[i];
      timeline.setReadout(Number.isNaN(min) || a === undefined ? "" : `${clockOf(Math.floor(min))} · ${a}/min${tw !== undefined ? ` vs ${tw} normal` : ""}`);
    },
  });
  new PostmortemPanel(incidentHost, services.postmortem, preset, (md) => {
    data = parseReport(md);
    rail.setItems(railItems(data));
    rail.select(selected);
    if (data.incident) {
      chipEl.hidden = false;
      chipEl.textContent = `INCIDENT · ${clockOf(data.incident.startMin)}–${clockOf(data.incident.endMin)}`;
    }
    select(selected ?? "inc");
  });

  const chutes = chuteCells(layout);
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let floorFrame: Frame | null = null;
  const selectionTab = document.getElementById("tab-selection")!;
  const selectedTitle = (): string => {
    const hit = railItems(data ?? { card: null, sections: [], incident: null, causes: [], fixes: [] }).find((i) => i.id === selected);
    return hit ? hit.title : "";
  };
  // The floor zooms to the incident while the selection is shown, and the focus ring rides the same transform.
  const applyFrame = () => {
    // Overview by default (whole floor, incident marked); zoom in only when a cause is selected.
    const causeSelected = data?.causes.some((c) => c.id === selected) ?? false;
    const on = selectionTab.getAttribute("aria-selected") === "true" && floorFrame !== null && causeSelected;
    for (const el of [canvas, focusEl]) {
      el.style.transformOrigin = "0 0";
      el.style.transition = reduceMotion ? "none" : "transform 400ms ease";
      el.style.transform = on ? `translate(${floorFrame!.tx}px, ${floorFrame!.ty}px) scale(${floorFrame!.k})` : "none";
    }
  };
  new MutationObserver(applyFrame).observe(selectionTab, { attributes: true, attributeFilter: ["aria-selected"] });

  /** Ring the selected cause on the floor while its fault is active (the engine only reports active faults). */
  const refreshFocus = async (): Promise<void> => {
    const ring = document.createElement("div");
    ring.className = "ring";
    const snap = source.snapshot();
    const fit = fitToCanvas(layout.width, layout.height, canvas.clientWidth, canvas.clientHeight, theme.padding);
    const cause = data?.causes.find((c) => c.id === selected);
    const fault = cause ? (await services.faults.active(snap.tick)).find((f) => `F${f.id}` === cause.id) : undefined;
    let cell: [number, number] | null = null;
    if (fault?.kind === "chute_jam") {
      const c = chutes[fault.params.chute ?? -1];
      if (c !== undefined) cell = [c % layout.width, Math.floor(c / layout.width)];
    } else if (fault?.kind === "robot_failure" && fault.params.robot !== undefined) {
      const r = fault.params.robot;
      cell = [snap.robotXY[2 * r]!, snap.robotXY[2 * r + 1]!];
    }
    // A broken-down robot is focused through the renderer (#45); a chute gets the ring.
    renderer.setFocus?.(fault?.kind === "robot_failure" && fault.params.robot !== undefined ? [fault.params.robot] : []);
    if (!cell || fault?.kind !== "chute_jam") return void focusEl.replaceChildren();
    const rc = focusRect(fit, cell[0], cell[1]);
    Object.assign(ring.style, { left: `${rc.left}px`, top: `${rc.top}px`, width: `${rc.size}px`, height: `${rc.size}px` });
    focusEl.replaceChildren(ring);
  };

  function select(id: ObjectId | null): void {
    selected = id;
    rail.select(id);
    inspector.show(id, data);
    tabs.select("selection");
    focusChip.hidden = !id || !data;
    focusChip.textContent = id === "inc" ? "Focus: incident area" : `Focus: ${selectedTitle()}`;
    applyFrame();
    void refreshFocus().catch(() => focusEl.replaceChildren());
    refreshTimeline();
  }

  const refreshTimeline = () => timeline.update(consoleTimeline(services.metrics.series(), data, selected, nowTick));
  const refreshMarks = async () => {
    const snap = source.snapshot();
    const jammed = jammedCellsOf(await services.faults.active(snap.tick), chutes);
    const marks = floorMarks(jammed);
    renderer.setMarks?.(marks);
    floorFrame = incidentFrame(incidentCells(layout, snap, jammed), layout, canvas.clientWidth, canvas.clientHeight, theme.padding) ?? floorFrame;
    applyFrame();
    await refreshFocus();
  };
  const refreshSide = () => {
    refreshTimeline();
    const dip = dipVsTwin(services.metrics.series());
    dipEl.textContent = dip === null ? "" : `${dip < 0 ? "−" : "+"}${Math.abs(dip)} %`;
    void faultPanel.refresh();
    void refreshMarks().catch((err) => console.warn("floor marks unavailable:", err));
  };
  refreshSide();
  setInterval(refreshSide, 1000);

  // Click a robot to see its parcel's trace. A failed robot may still hold its parcel (stuck on it until repair).
  const carrying: readonly number[] = [RobotState.ToChute, RobotState.Dropping, RobotState.Failed];
  canvas.addEventListener("click", (e) => {
    const r = canvas.getBoundingClientRect();
    const snap = source.snapshot();
    // The canvas may be zoomed by the incident frame: map the click back to canvas pixels.
    const px = ((e.clientX - r.left) * canvas.clientWidth) / r.width;
    const py = ((e.clientY - r.top) * canvas.clientHeight) / r.height;
    const id = renderer.pickAt?.(px, py) ?? null;
    if (id === null) {
      renderer.highlight([]);
      return;
    }
    tabs.select("trace");
    renderer.highlight([id]);
    if (!carrying.includes(snap.robotState[id]!)) {
      tracePanel.message(`R${id} is not carrying a parcel. Click a robot that is.`);
      return;
    }
    void services.traces
      .parcelOfRobot(id)
      .then((p) => (p === null ? tracePanel.message(`R${id} has no parcel.`) : tracePanel.showParcel(p)));
  });

  let playing = true;
  let speed = 1;
  // Wall-clock pacing: 1x is real time (1 tick = 0.5 s), not "N ticks per frame".
  const pacer = new TickPacer();
  let pending = 0;
  const showPlaying = () => {
    playBtn.textContent = playing ? "❚❚" : "▶";
    playBtn.setAttribute("aria-label", playing ? "Pause" : "Play");
  };
  playBtn.addEventListener("click", () => {
    playing = !playing;
    pacer.reset();
    showPlaying();
  });
  for (const b of speedBtns) {
    b.addEventListener("click", () => {
      speed = Number(b.dataset.speed);
      speedBtns.forEach((o) => o.classList.toggle("on", o === b));
    });
  }

  // Hover label: the renderer draws it next to the robot under the pointer.
  const hoverAt = (e: PointerEvent) => {
    const r = canvas.getBoundingClientRect();
    const px = ((e.clientX - r.left) * canvas.clientWidth) / r.width;
    const py = ((e.clientY - r.top) * canvas.clientHeight) / r.height;
    renderer.setHover?.(renderer.pickAt?.(px, py) ?? null);
  };
  canvas.addEventListener("pointermove", hoverAt);
  canvas.addEventListener("pointerleave", () => renderer.setHover?.(null));

  new ResizeObserver(() => renderer.resize(canvas.clientWidth, canvas.clientHeight)).observe(canvas);

  let busy = false;
  async function frame(): Promise<void> {
    if (playing) pending += pacer.take(performance.now(), speed);
    if (playing && !busy && pending > 0) {
      busy = true;
      const n = pending;
      pending = 0;
      try {
        await source.step(n);
      } catch (err) {
        console.error("sim step failed, pausing:", err);
        playing = false;
        showPlaying();
      } finally {
        busy = false;
      }
    }
    const snap = source.snapshot();
    nowTick = snap.tick;
    faultPanel.tickBars();
    renderer.draw(snap);
    tickEl.textContent = shiftClock(snap.tick, theme.ticksPerMinute);
    tickEl.dataset.tick = String(snap.tick);
    const fl = activeParked(snap.robotState);
    fleetEl.textContent = `active ${fl.active} / parked ${fl.parked}`;
    const st = source.stats();
    statsEl.textContent = `${Math.round(st.throughput_per_hour_milli / 1000).toLocaleString("en-US")}/h`;
    requestAnimationFrame(() => void frame());
  }
  requestAnimationFrame(() => void frame());
}

void main();
