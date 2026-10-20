import { Canvas2DRenderer } from "./renderer/canvas2d";
import { MockSource, type SimSource } from "./sim/source";

const canvas = document.getElementById("canvas") as HTMLCanvasElement;
const playBtn = document.getElementById("play") as HTMLButtonElement;
const speedSel = document.getElementById("speed") as HTMLSelectElement;
const tickEl = document.getElementById("tick") as HTMLSpanElement;

const source: SimSource = new MockSource();
const renderer = new Canvas2DRenderer();
renderer.init(canvas, source.layout());

let playing = true;
playBtn.addEventListener("click", () => {
  playing = !playing;
  playBtn.textContent = playing ? "Pause" : "Play";
});

new ResizeObserver(() => renderer.resize(canvas.clientWidth, canvas.clientHeight)).observe(canvas);

function frame(): void {
  if (playing) source.step(Number(speedSel.value));
  const snap = source.snapshot();
  renderer.draw(snap);
  tickEl.textContent = `tick ${snap.tick}`;
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
