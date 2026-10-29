import { h } from "./dom";

export interface CollapsibleOptions {
  title: string;
  teaser?: string;
  open?: boolean;
  id?: string;
}

let sectionCount = 0;

/** A section with a header button (title, teaser, chevron) and a body. Pure DOM; text via textContent only. */
export function collapsible(opts: CollapsibleOptions, body: Node[]): HTMLElement {
  const id = opts.id ?? `sec-${++sectionCount}`;
  let open = opts.open ?? false;

  const head = h(
    "button",
    {
      type: "button",
      class: "collapsible-head",
      "aria-expanded": String(open),
      "aria-controls": id,
    },
    h("span", { class: "collapsible-title" }, opts.title),
    ...(opts.teaser ? [h("span", { class: "collapsible-teaser" }, opts.teaser)] : []),
    h("span", { class: "collapsible-chevron", "aria-hidden": "true" }, "▸"),
  );

  const bodyEl = h("div", { class: "collapsible-body", id, role: "region" }, ...body);
  if (!open) bodyEl.setAttribute("hidden", "");

  const section = h("section", { class: "collapsible", "data-open": String(open) }, head, bodyEl);

  head.addEventListener("click", () => {
    open = !open;
    head.setAttribute("aria-expanded", String(open));
    section.setAttribute("data-open", String(open));
    if (open) bodyEl.removeAttribute("hidden");
    else bodyEl.setAttribute("hidden", "");
  });

  return section;
}
