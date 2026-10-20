import { Canvas2DRenderer } from "./renderer/canvas2d";
import { MockSource, type SimSource } from "./sim/source";
import { legendEntries, activeParked } from "./legend";
import { theme } from "./theme";
import { parsePreset } from "./preset";
import { WasmSource } from "./sim/wasmSource";

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

async function openSource(): Promise<SimSource> {
  try {
    return await WasmSource.create(preset);
  } catch (err) {
    console.warn("WASM sim unavailable, falling back to mock data:", err);
    badgeEl.hidden = false;
    return new MockSource();
  }
}

async function main(): Promise<void> {
  const source = await openSource();
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
  renderer.init(canvas, source.layout());

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
