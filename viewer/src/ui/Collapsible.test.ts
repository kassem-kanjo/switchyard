import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { collapsible } from "./Collapsible";

/**
 * Minimal DOM stand-in. The viewer has no jsdom/happy-dom dependency (see the
 * FakeEl precedent in ui.test.ts), so these tests drive a tiny element model.
 */
class FakeEvent {
  defaultPrevented = false;
  target: FakeEl | null = null;
  constructor(
    public type: string,
    public key = "",
  ) {}
  preventDefault(): void {
    this.defaultPrevented = true;
  }
}

class FakeEl {
  readonly children: FakeEl[] = [];
  readonly attrs = new Map<string, string>();
  private readonly listeners = new Map<string, ((e: FakeEvent) => void)[]>();
  className = "";
  private ownText = "";

  constructor(public readonly tag: string) {}

  static text(v: string): FakeEl {
    const n = new FakeEl("#text");
    n.ownText = v;
    return n;
  }

  get textContent(): string {
    return this.tag === "#text" ? this.ownText : this.children.map((c) => c.textContent).join("");
  }

  set textContent(v: string) {
    this.children.length = 0;
    if (v !== "") this.children.push(FakeEl.text(v));
  }

  get hidden(): boolean {
    return this.attrs.has("hidden");
  }

  set hidden(v: boolean) {
    if (v) this.attrs.set("hidden", "");
    else this.attrs.delete("hidden");
  }

  append(...nodes: (FakeEl | string)[]): void {
    for (const n of nodes) this.children.push(typeof n === "string" ? FakeEl.text(n) : n);
  }

  setAttribute(name: string, value: string): void {
    this.attrs.set(name, value);
  }

  getAttribute(name: string): string | null {
    return this.attrs.get(name) ?? null;
  }

  hasAttribute(name: string): boolean {
    return this.attrs.has(name);
  }

  removeAttribute(name: string): void {
    this.attrs.delete(name);
  }

  addEventListener(type: string, fn: (e: FakeEvent) => void): void {
    const list = this.listeners.get(type) ?? [];
    list.push(fn);
    this.listeners.set(type, list);
  }

  dispatchEvent(e: FakeEvent): void {
    e.target = this;
    for (const fn of this.listeners.get(e.type) ?? []) fn(e);
  }

  click(): void {
    this.dispatchEvent(new FakeEvent("click"));
  }
}

function findAll(root: FakeEl, cls: string): FakeEl[] {
  const out: FakeEl[] = [];
  const walk = (n: FakeEl) => {
    if (n.className.split(" ").includes(cls)) out.push(n);
    for (const c of n.children) walk(c);
  };
  walk(root);
  return out;
}

function one(root: FakeEl, cls: string): FakeEl {
  const found = findAll(root, cls);
  expect(found).toHaveLength(1);
  return found[0]!;
}

function section(opts: Parameters<typeof collapsible>[0], body: Node[] = []): FakeEl {
  return collapsible(opts, body) as unknown as FakeEl;
}

beforeEach(() => {
  vi.stubGlobal("document", {
    createElement: (tag: string) => new FakeEl(tag),
    createTextNode: (v: string) => FakeEl.text(v),
  });
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("collapsible", () => {
  it("renders the title, teaser, chevron and supplied body nodes", () => {
    const para = new FakeEl("p");
    para.textContent = "details";
    const root = section({ title: "Trace", teaser: "3 min" }, [para as unknown as Node]);

    expect(root.tag).toBe("section");
    expect(root.className).toBe("collapsible");
    expect(one(root, "collapsible-title").textContent).toBe("Trace");
    expect(one(root, "collapsible-teaser").textContent).toBe("3 min");

    const chevron = one(root, "collapsible-chevron");
    expect(chevron.textContent).toBe("▸");
    expect(chevron.getAttribute("aria-hidden")).toBe("true");

    const head = one(root, "collapsible-head");
    expect(head.tag).toBe("button");
    expect(head.getAttribute("type")).toBe("button");

    const body = one(root, "collapsible-body");
    expect(body.getAttribute("role")).toBe("region");
    expect(body.children).toContain(para);
  });

  it("is closed by default", () => {
    const root = section({ title: "Trace" });
    expect(one(root, "collapsible-head").getAttribute("aria-expanded")).toBe("false");
    expect(root.getAttribute("data-open")).toBe("false");
    expect(one(root, "collapsible-body").hasAttribute("hidden")).toBe(true);
  });

  it("opens when asked to", () => {
    const root = section({ title: "Trace", open: true });
    expect(one(root, "collapsible-head").getAttribute("aria-expanded")).toBe("true");
    expect(root.getAttribute("data-open")).toBe("true");
    expect(one(root, "collapsible-body").hasAttribute("hidden")).toBe(false);
  });

  it("toggles aria-expanded, hidden and data-open on click", () => {
    const root = section({ title: "Trace" });
    const head = one(root, "collapsible-head");
    const body = one(root, "collapsible-body");

    head.click();
    expect(head.getAttribute("aria-expanded")).toBe("true");
    expect(root.getAttribute("data-open")).toBe("true");
    expect(body.hasAttribute("hidden")).toBe(false);

    head.click();
    expect(head.getAttribute("aria-expanded")).toBe("false");
    expect(root.getAttribute("data-open")).toBe("false");
    expect(body.hasAttribute("hidden")).toBe(true);
  });

  it("toggles exactly once for one Enter/Space activation", () => {
    for (const key of ["Enter", " "]) {
      const root = section({ title: "Trace" });
      const head = one(root, "collapsible-head");

      // A bare keydown must not toggle: the native button supplies the click.
      head.dispatchEvent(new FakeEvent("keydown", key));
      expect(root.getAttribute("data-open")).toBe("false");

      head.click();
      expect(root.getAttribute("data-open")).toBe("true");
    }
  });

  it("omits the teaser when there is none", () => {
    const root = section({ title: "Trace" });
    expect(findAll(root, "collapsible-teaser")).toHaveLength(0);
    expect(one(root, "collapsible-head").children.map((c) => c.className)).toEqual([
      "collapsible-title",
      "collapsible-chevron",
    ]);
  });

  it("shows markup-looking text as text, never as elements", () => {
    const root = section({ title: "<b>x</b>", teaser: "<i>t</i>" });
    const title = one(root, "collapsible-title");
    expect(title.textContent).toBe("<b>x</b>");
    expect(title.children.map((c) => c.tag)).toEqual(["#text"]);
    expect(one(root, "collapsible-teaser").textContent).toBe("<i>t</i>");
    expect(root.children.some((c) => c.tag === "b")).toBe(false);
  });

  it("wires aria-controls to the supplied or a unique generated id", () => {
    const custom = section({ title: "A", id: "custom-body" });
    expect(one(custom, "collapsible-head").getAttribute("aria-controls")).toBe("custom-body");
    expect(one(custom, "collapsible-body").getAttribute("id")).toBe("custom-body");

    const b = section({ title: "B" });
    const c = section({ title: "C" });
    const idB = one(b, "collapsible-body").getAttribute("id")!;
    const idC = one(c, "collapsible-body").getAttribute("id")!;
    expect(idB).toMatch(/^sec-\d+$/);
    expect(idC).not.toBe(idB);
    expect(one(b, "collapsible-head").getAttribute("aria-controls")).toBe(idB);
  });
});
