import { theme } from "../theme";
import type { ActiveFault, FaultControl, FaultKindInfo } from "../data/types";
import { h } from "./dom";
import { formatDuration } from "./traceFormat";
import { DURATION_KEY, remainingFraction, validateFault } from "./faultValidation";

const TPS = theme.ticksPerMinute / 60;

/** Pick a fault kind, set its parameters, inject at the current tick; lists active faults with a time-left bar. */
export class FaultPanel {
  private kinds: FaultKindInfo[] = [];
  private readonly select = h("select", { "aria-label": "Fault kind" });
  private readonly fields = h("div", { class: "fields" });
  private readonly duration = h("input", { type: "text", inputmode: "numeric", value: "300", "aria-label": "Duration in seconds" });
  private readonly desc = h("p", { class: "note" });
  private readonly status = h("p", { class: "note", role: "status" });
  private readonly activeList = h("ul", { class: "faults" });
  private active: ActiveFault[] = [];
  private inputs = new Map<string, HTMLInputElement>();

  constructor(
    root: HTMLElement,
    private readonly control: FaultControl,
    private readonly nowTick: () => number,
  ) {
    const form = h(
      "form",
      { class: "fault-form" },
      h("label", {}, "Fault", this.select),
      this.desc,
      this.fields,
      this.field(DURATION_KEY, "Lasts", "s", this.duration),
      h("button", { type: "submit", class: "primary", title: "Break something on the live floor now, for example jam a chute, then watch what it costs." }, "Inject at current tick"),
      this.status,
    );
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      void this.submit();
    });
    this.select.addEventListener("change", () => this.buildFields());
    root.append(form, h("h4", {}, "Active faults"), this.activeList);
    void this.load();
  }

  private async load(): Promise<void> {
    try {
      this.kinds = await this.control.kinds();
    } catch (err) {
      this.status.textContent = `Could not load fault kinds: ${String(err)}`;
      return;
    }
    this.select.replaceChildren(...this.kinds.map((k) => h("option", { value: k.id }, k.label)));
    this.buildFields();
    this.renderActive();
  }

  private field(key: string, label: string, unit: string | undefined, input: HTMLInputElement): HTMLElement {
    return h(
      "label",
      { class: "field", "data-key": key },
      label,
      h("span", { class: "input-wrap" }, input, ...(unit ? [h("span", { class: "unit" }, unit)] : [])),
      h("span", { class: "err", role: "alert" }),
    );
  }

  private current(): FaultKindInfo | undefined {
    return this.kinds.find((k) => k.id === this.select.value);
  }

  private buildFields(): void {
    const kind = this.current();
    this.inputs.clear();
    this.fields.replaceChildren();
    if (!kind) return;
    this.desc.textContent = kind.description;
    for (const p of kind.params) {
      const input = h("input", { type: "text", inputmode: "decimal", value: String(p.default), "aria-label": p.label });
      this.inputs.set(p.key, input);
      this.fields.append(this.field(p.key, p.label, p.unit, input));
    }
    this.showErrors({});
  }

  private showErrors(errors: Record<string, string>): void {
    for (const el of this.fieldEls()) {
      const msg = errors[el.dataset.key ?? ""] ?? "";
      el.classList.toggle("invalid", msg !== "");
      el.querySelector(".err")!.textContent = msg;
    }
  }

  private fieldEls(): HTMLElement[] {
    return [...this.fields.children, this.duration.closest("label")!].filter((e): e is HTMLElement => e instanceof HTMLElement);
  }

  private async submit(): Promise<void> {
    const kind = this.current();
    if (!kind) return;
    const values: Record<string, string> = {};
    for (const [k, input] of this.inputs) values[k] = input.value;
    const res = validateFault(kind, values, this.duration.value, this.nowTick(), TPS);
    if (!res.ok) {
      this.showErrors(res.errors);
      this.status.textContent = "Fix the highlighted fields.";
      return;
    }
    this.showErrors({});
    try {
      const f = await this.control.inject(res.request);
      this.status.textContent = `${kind.label} injected, ends in ${formatDuration(f.endTick - this.nowTick(), TPS)}.`;
      await this.refresh();
    } catch (err) {
      this.status.textContent = `Injection failed: ${String(err)}`;
    }
  }

  /** Poll active faults; call about once a second. */
  async refresh(): Promise<void> {
    try {
      this.active = await this.control.active(this.nowTick());
    } catch {
      return;
    }
    this.renderActive();
  }

  /** Cheap per-frame update of the remaining-time bars without asking the provider. */
  tickBars(): void {
    const now = this.nowTick();
    this.activeList.querySelectorAll<HTMLElement>("li[data-id]").forEach((li) => {
      const f = this.active.find((a) => String(a.id) === li.dataset.id);
      if (!f) return;
      const frac = remainingFraction(f, now);
      (li.querySelector(".bar > i") as HTMLElement).style.width = `${frac * 100}%`;
      li.querySelector(".left")!.textContent = `${formatDuration(f.endTick - now, TPS)} left`;
    });
  }

  private renderActive(): void {
    if (this.active.length === 0) {
      this.activeList.replaceChildren(h("li", { class: "empty" }, "No faults active. The floor is running clean."));
      return;
    }
    const label = (id: string) => this.kinds.find((k) => k.id === id)?.label ?? id;
    this.activeList.replaceChildren(
      ...this.active.map((f) =>
        h(
          "li",
          { "data-id": String(f.id) },
          h("span", { class: "name" }, label(f.kind)),
          h("span", { class: "params" }, Object.entries(f.params).map(([k, v]) => `${k} ${v}`).join(", ")),
          h("span", { class: "left" }),
          h("span", { class: "bar" }, h("i", {})),
        ),
      ),
    );
    this.tickBars();
  }
}
