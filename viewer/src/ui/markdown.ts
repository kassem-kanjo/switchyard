/**
 * A deliberately tiny Markdown subset for post-mortem reports: headings,
 * paragraphs, bullet and numbered lists, **bold**, *italic*, `code`.
 * It parses to a plain AST and the DOM builder only ever uses createElement
 * and textContent, so report text can never become markup or script.
 * No links or images are supported on purpose.
 */

export type Inline = { t: "text" | "bold" | "italic" | "code"; v: string };

export type Block =
  | { t: "heading"; level: 1 | 2 | 3; inline: Inline[] }
  | { t: "p"; inline: Inline[] }
  | { t: "ul" | "ol"; items: Inline[][] };

export function parseInline(src: string): Inline[] {
  const out: Inline[] = [];
  const re = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*\s][^*]*\*)/g;
  let last = 0;
  for (let m = re.exec(src); m; m = re.exec(src)) {
    if (m.index > last) out.push({ t: "text", v: src.slice(last, m.index) });
    const tok = m[0];
    if (tok.startsWith("`")) out.push({ t: "code", v: tok.slice(1, -1) });
    else if (tok.startsWith("**")) out.push({ t: "bold", v: tok.slice(2, -2) });
    else out.push({ t: "italic", v: tok.slice(1, -1) });
    last = m.index + tok.length;
  }
  if (last < src.length) out.push({ t: "text", v: src.slice(last) });
  return out;
}

export function parseMarkdown(src: string): Block[] {
  const blocks: Block[] = [];
  let para: string[] = [];
  let list: { t: "ul" | "ol"; items: Inline[][] } | null = null;
  const flushPara = () => {
    if (para.length) blocks.push({ t: "p", inline: parseInline(para.join(" ")) });
    para = [];
  };
  const flushList = () => {
    if (list) blocks.push(list);
    list = null;
  };
  for (const raw of src.replace(/\r\n?/g, "\n").split("\n")) {
    const line = raw.trimEnd();
    const h = /^(#{1,3})\s+(.*)$/.exec(line);
    const ul = /^\s*[-*]\s+(.*)$/.exec(line);
    const ol = /^\s*\d+[.)]\s+(.*)$/.exec(line);
    if (h) {
      flushPara();
      flushList();
      blocks.push({ t: "heading", level: h[1]!.length as 1 | 2 | 3, inline: parseInline(h[2]!) });
    } else if (ul || ol) {
      flushPara();
      const kind = ul ? "ul" : "ol";
      if (list && list.t !== kind) flushList();
      list ??= { t: kind, items: [] };
      list.items.push(parseInline((ul ?? ol)![1]!));
    } else if (line.trim() === "") {
      flushPara();
      flushList();
    } else {
      flushList();
      para.push(line.trim());
    }
  }
  flushPara();
  flushList();
  return blocks;
}

const INLINE_TAG = { text: null, bold: "strong", italic: "em", code: "code" } as const;

function inlineNodes(items: Inline[]): Node[] {
  return items.map((i) => {
    const tag = INLINE_TAG[i.t];
    if (!tag) return document.createTextNode(i.v);
    const el = document.createElement(tag);
    el.textContent = i.v;
    return el;
  });
}

/** Build DOM for a report. Text goes through textContent only. Headings shift down two levels for the sidebar. */
export function renderMarkdown(src: string, into: HTMLElement): void {
  into.replaceChildren();
  for (const b of parseMarkdown(src)) {
    if ("inline" in b) {
      const el = document.createElement(b.t === "heading" ? `h${b.level + 2}` : "p");
      el.append(...inlineNodes(b.inline));
      into.append(el);
    } else {
      const list = document.createElement(b.t);
      for (const item of b.items) {
        const li = document.createElement("li");
        li.append(...inlineNodes(item));
        list.append(li);
      }
      into.append(list);
    }
  }
}
