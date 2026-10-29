/** Left rail: grouped, keyboard-navigable object rows. DOM only; all text goes through text nodes. */

import { h } from "../dom";
import { barWidthPct, groupItems, nextId } from "./railModel";
import type { ObjectId, RailItem, Tone } from "./types";

const TONE_VAR: Record<Tone, string> = {
  danger: "var(--danger)",
  ok: "var(--ok)",
  muted: "var(--muted)",
};

export class Rail {
  private items: RailItem[] = [];
  private current: ObjectId | null = null;
  /** Live row elements by id, so arrow keys can focus the freshly rendered row. */
  private rows = new Map<ObjectId, HTMLElement>();

  constructor(
    private readonly root: HTMLElement,
    private readonly onSelect: (id: ObjectId) => void,
  ) {
    this.root.setAttribute("role", "listbox");
    this.root.setAttribute("aria-label", "Objects");
    this.render();
  }

  setItems(items: RailItem[]): void {
    this.items = items;
    // Keep the selection only while it still exists; a removed object must not stay active.
    if (this.current !== null && !items.some((item) => item.id === this.current)) {
      this.current = null;
    }
    this.render();
  }

  select(id: ObjectId | null): void {
    // Ignore ids that are not on the rail, matching setItems' "selection must exist" rule.
    this.current = id !== null && this.items.some((item) => item.id === id) ? id : null;
    // Restyle the live rows rather than rebuilding them, so keyboard focus survives a selection.
    for (const [rowId, row] of this.rows) {
      const active = rowId === this.current;
      row.className = active ? "item active" : "item";
      row.setAttribute("aria-selected", String(active));
    }
  }

  private render(): void {
    this.rows.clear();
    const nodes: HTMLElement[] = [];

    for (const group of groupItems(this.items)) {
      nodes.push(h("h3", { class: "lbl" }, group.title));
      for (const item of group.items) {
        const active = item.id === this.current;
        const row = h(
          "div",
          {
            class: active ? "item active" : "item",
            role: "option",
            tabindex: "0",
            "data-id": item.id,
            "aria-selected": String(active),
          },
          h(
            "div",
            { class: "t" },
            ...(item.tag ? [h("span", { class: "id" }, item.tag)] : []),
            item.title,
          ),
          h("div", { class: "s" }, item.subtitle),
          ...(item.sharePct === undefined
            ? []
            : [
                h(
                  "div",
                  { class: "bar" },
                  h("i", {
                    style: `width:${barWidthPct(item)}%;background:${TONE_VAR[item.tone]}`,
                  }),
                ),
              ]),
        );

        const choose = () => {
          this.select(item.id);
          this.onSelect(item.id);
        };
        row.addEventListener("click", choose);
        row.addEventListener("keydown", (event) => {
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            choose();
          } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            const id = nextId(this.items, this.current, event.key === "ArrowDown" ? 1 : -1);
            if (id !== null) {
              this.select(id);
              this.rows.get(id)?.focus();
            }
          }
        });

        this.rows.set(item.id, row);
        nodes.push(row);
      }
    }

    this.root.replaceChildren(...nodes);
  }
}
