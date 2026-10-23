import { theme } from "../theme";
import type { ParcelTrace, TraceProvider } from "../data/types";
import { h } from "./dom";
import { PHASE_LABEL, blameChain, describeSpan, formatDuration, parseParcelId, summarize } from "./traceFormat";

const TPS = theme.ticksPerMinute / 60;

/** Parcel trace: search box, a horizontal phase timeline and a plain-language list of spans. */
export class TracePanel {
  private readonly body = h("div", { class: "trace-body" });
  private readonly input = h("input", {
    type: "text",
    inputmode: "numeric",
    placeholder: "Parcel id, e.g. 1041",
    "aria-label": "Parcel id",
  });
  private seq = 0;

  constructor(root: HTMLElement, private readonly provider: TraceProvider) {
    const form = h("form", { class: "row" }, this.input, h("button", { type: "submit" }, "Show trace"));
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      const id = parseParcelId(this.input.value);
      if (id === null) this.message("Enter a parcel number, like 1041.", true);
      else void this.showParcel(id);
    });
    root.append(form, this.body);
    this.message("Click a robot that is carrying a parcel on the floor, or search for a parcel id.");
  }

  message(text: string, isError = false): void {
    this.seq++;
    this.body.replaceChildren(h("p", { class: isError ? "note error" : "note" }, text));
  }

  async showParcel(parcelId: number): Promise<void> {
    const mine = ++this.seq;
    this.input.value = String(parcelId);
    this.body.replaceChildren(h("p", { class: "note" }, "Loading trace..."));
    try {
      const trace = await this.provider.getTrace(parcelId);
      if (mine !== this.seq) return; // a newer request superseded this one
      if (!trace) this.message(`No trace for parcel ${parcelId}. It may not have been inducted yet.`, true);
      else this.render(trace);
    } catch (err) {
      if (mine === this.seq) this.message(`Could not load the trace: ${String(err)}`, true);
    }
  }

  private render(trace: ParcelTrace): void {
    const sum = summarize(trace);
    const origin = trace.spans[0]?.startTick ?? 0;
    const headline = sum.worstBlocked
      ? describeSpan(sum.worstBlocked, TPS)
      : "Never blocked.";
    const bar = h("div", { class: "timeline", role: "img", "aria-label": `Phase timeline, ${formatDuration(sum.totalTicks, TPS)} in total` });
    for (const s of trace.spans) {
      const seg = h("div", { class: "seg", title: describeSpan(s, TPS) });
      seg.style.flexGrow = String(s.endTick - s.startTick);
      seg.style.background = theme.phase[s.phase];
      bar.append(seg);
    }
    const axis = h("div", { class: "axis" }, h("span", {}, "0 s"), h("span", {}, formatDuration(sum.totalTicks, TPS)));

    const list = h("ul", { class: "spans" });
    for (const s of trace.spans) {
      const chain = s.phase === "blocked" ? blameChain(s) : null;
      const dot = h("i", {});
      dot.style.background = theme.phase[s.phase];
      list.append(
        h(
          "li",
          { class: s.phase === "blocked" ? "blocked" : "" },
          dot,
          h("span", { class: "name" }, PHASE_LABEL[s.phase]),
          h("span", { class: "dur" }, formatDuration(s.endTick - s.startTick, TPS)),
          h("span", { class: "at" }, `+${formatDuration(s.startTick - origin, TPS)}`),
          ...(chain ? [h("span", { class: "chain" }, `by ${chain}`)] : []),
        ),
      );
    }

    this.body.replaceChildren(
      h(
        "div",
        { class: "trace-head" },
        h("strong", {}, `Parcel ${trace.parcelId}`),
        h("span", { class: "muted" }, trace.robot === null ? "not on a robot" : `carried by R${trace.robot}`),
        h("span", { class: "muted" }, `${formatDuration(sum.totalTicks, TPS)} end to end`),
      ),
      h("p", { class: sum.worstBlocked ? "blame" : "note" }, headline),
      bar,
      axis,
      list,
    );
  }
}
