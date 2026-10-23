import { h } from "./dom";

export interface TabSet {
  select(id: string): void;
}

/** Accessible tab strip (arrow keys move between tabs). `panels` maps tab id to its panel element. */
export function mountTabs(bar: HTMLElement, tabs: { id: string; label: string }[], panels: Record<string, HTMLElement>): TabSet {
  const buttons = new Map<string, HTMLButtonElement>();
  const select = (id: string, focus = false) => {
    for (const [tid, b] of buttons) {
      const on = tid === id;
      b.setAttribute("aria-selected", String(on));
      b.tabIndex = on ? 0 : -1;
      panels[tid]!.hidden = !on;
      if (on && focus) b.focus();
    }
  };
  tabs.forEach((t, i) => {
    const b = h("button", { role: "tab", type: "button", id: `tab-${t.id}`, "aria-controls": `panel-${t.id}` }, t.label);
    panels[t.id]!.setAttribute("role", "tabpanel");
    panels[t.id]!.id = `panel-${t.id}`;
    b.addEventListener("click", () => select(t.id));
    b.addEventListener("keydown", (e) => {
      const d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
      if (d) select(tabs[(i + d + tabs.length) % tabs.length]!.id, true);
    });
    buttons.set(t.id, b);
    bar.append(b);
  });
  bar.setAttribute("role", "tablist");
  select(tabs[0]!.id);
  return { select };
}
