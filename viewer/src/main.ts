import { Canvas2DRenderer } from "./renderer/canvas2d";
import { MockSource, type SimSource } from "./sim/source";
import { legendEntries, activeParked } from "./legend";
import { theme } from "./theme";
import { parsePreset } from "./preset";
import { WasmSource } from "./sim/wasmSource";
import "./ui/app.css";
import { applyThemeVars } from "./theme";
import { createMockServices } from "./data/mock";
import { createWasmServices } from "./data/wasmServices";
import type { Services } from "./data/types";
import { ChartView } from "./ui/ChartView";
import { TracePanel } from "./ui/TracePanel";
import { FaultPanel } from "./ui/FaultPanel";
import { PostmortemPanel } from "./ui/PostmortemPanel";
import { mountTabs } from "./ui/tabs";
import { pickRobot } from "./ui/pick";
import { fitToCanvas } from "./renderer/fit";
import { RobotState } from "./renderer/Renderer";
import { chuteCells, floorMarks, incidentFrame, jammedCellsOf, type Frame } from "./renderer/floorMarks";
import { h } from "./ui/dom";

applyThemeVars();

const canvas = document.getElementById("canvas") as HTMLCanvasElement;
const playBtn = document.getElementById("play") as HTMLButtonElement;
const speedSel = document.getElementById("speed") as HTMLSelectElement;
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

  // Sidebar. Data providers come from the same worker as the floor.
  const chart = new ChartView(document.getElementById("chart") as HTMLCanvasElement);
  const panels = { trace: h("div"), faults: h("div"), postmortem: h("div") };
  document.getElementById("side")!.append(panels.trace, panels.faults, panels.postmortem);
  const tabs = mountTabs(
    document.getElementById("tabbar")!,
    [
      { id: "trace", label: "Trace" },
      { id: "faults", label: "Faults" },
      { id: "postmortem", label: "Post-mortem" },
    ],
    panels,
  );
  const tracePanel = new TracePanel(panels.trace, services.traces);
  const faultPanel = new FaultPanel(panels.faults, services.faults, () => nowTick);
  new PostmortemPanel(panels.postmortem, services.postmortem, preset);
  // Floor marks: jammed chutes from the active faults, a glow around failed robots and jams, and the
  // frame the floor eases to when the Post-mortem tab is open (kept after the incident ends).
  const chutes = chuteCells(layout);
  const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  let floorFrame: Frame | null = null;
  const postmortemTab = document.getElementById("tab-postmortem")!;
  const applyFrame = () => {
    const on = postmortemTab.getAttribute("aria-selected") === "true" && floorFrame !== null;
    canvas.style.transformOrigin = "0 0";
    canvas.style.transition = reduceMotion ? "none" : "transform 400ms ease";
    canvas.style.transform = on ? `translate(${floorFrame!.tx}px, ${floorFrame!.ty}px) scale(${floorFrame!.k})` : "none";
  };
  new MutationObserver(applyFrame).observe(postmortemTab, { attributes: true, attributeFilter: ["aria-selected"] });
  const refreshMarks = async () => {
    const snap = source.snapshot();
    const marks = floorMarks(layout, snap, jammedCellsOf(await services.faults.active(snap.tick), chutes));
    renderer.setMarks?.(marks);
    floorFrame = incidentFrame(marks.glow, layout, canvas.clientWidth, canvas.clientHeight, theme.padding) ?? floorFrame;
    applyFrame();
  };
  const refreshSide = () => {
    chart.update(services.metrics.series());
    void faultPanel.refresh();
    void refreshMarks().catch((err) => console.warn("floor marks unavailable:", err));
  };
  refreshSide();
  setInterval(refreshSide, 1000);

  // Click a robot to see its parcel's trace. A failed robot may still hold its parcel (stuck on it until repair).
  const carrying: readonly number[] = [RobotState.ToChute, RobotState.Dropping, RobotState.Failed];
  canvas.addEventListener("click", (e) => {
    const r = canvas.getBoundingClientRect();
    const fit = fitToCanvas(layout.width, layout.height, canvas.clientWidth, canvas.clientHeight, theme.padding);
    const snap = source.snapshot();
    // The canvas may be zoomed by the incident frame: map the click back to canvas pixels.
    const px = ((e.clientX - r.left) * canvas.clientWidth) / r.width;
    const py = ((e.clientY - r.top) * canvas.clientHeight) / r.height;
    const id = pickRobot(snap, fit, px, py, null);
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
  playBtn.addEventListener("click", () => {
    playing = !playing;
    playBtn.textContent = playing ? "Pause" : "Play";
  });

  new ResizeObserver(() => renderer.resize(canvas.clientWidth, canvas.clientHeight)).observe(canvas);

  let busy = false;
  async function frame(): Promise<void> {
    if (playing && !busy) {
      busy = true;
      try {
        await source.step(Number(speedSel.value));
      } catch (err) {
        console.error("sim step failed, pausing:", err);
        playing = false;
        playBtn.textContent = "Play";
      } finally {
        busy = false;
      }
    }
    const snap = source.snapshot();
    nowTick = snap.tick;
    faultPanel.tickBars();
    renderer.draw(snap);
    tickEl.textContent = `tick ${snap.tick}`;
    const fl = activeParked(snap.robotState);
    fleetEl.textContent = `active ${fl.active} / parked ${fl.parked}`;
    const st = source.stats();
    statsEl.textContent = `delivered ${st.delivered} · ${(st.throughput_per_hour_milli / 1000).toFixed(1)} / h`;
    requestAnimationFrame(() => void frame());
  }
  requestAnimationFrame(() => void frame());
}

void main();
