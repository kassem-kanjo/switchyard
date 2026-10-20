import { Canvas2DRenderer } from "./renderer/canvas2d";
import { MockSource, type SimSource } from "./sim/source";
import { WasmSource } from "./sim/wasmSource";

const canvas = document.getElementById("canvas") as HTMLCanvasElement;
const playBtn = document.getElementById("play") as HTMLButtonElement;
const speedSel = document.getElementById("speed") as HTMLSelectElement;
const tickEl = document.getElementById("tick") as HTMLSpanElement;
const statsEl = document.getElementById("stats") as HTMLSpanElement;
const badgeEl = document.getElementById("badge") as HTMLSpanElement;

async function openSource(): Promise<SimSource> {
  try {
    return await WasmSource.create("small");
  } catch (err) {
    console.warn("WASM sim unavailable, falling back to mock data:", err);
    badgeEl.hidden = false;
    return new MockSource();
  }
}

async function main(): Promise<void> {
  const source = await openSource();
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
    const st = source.stats();
    statsEl.textContent = `delivered ${st.delivered} · ${(st.throughput_per_hour_milli / 1000).toFixed(1)} / h`;
    requestAnimationFrame(() => void frame());
  }
  requestAnimationFrame(() => void frame());
}

void main();
