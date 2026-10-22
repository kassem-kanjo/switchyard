/**
 * A deliberately tiny Markdown subset for post-mortem reports: headings,
 * paragraphs, bullet and numbered lists, GitHub-style pipe tables, **bold**, *italic*, `code`.
 * It parses to a plain AST and the DOM builder only ever uses createElement
 * and textContent, so report text can never become markup or script.
 * No links or images are supported on purpose.
 */

export type Inline = { t: "text" | "bold" | "italic" | "code"; v: string };

export type Block =
  | { t: "heading"; level: 1 | 2 | 3; inline: Inline[] }
  | { t: "p"; inline: Inline[] }
  | { t: "ul" | "ol"; items: Inline[][] }
  | { t: "table"; align: Align[]; head: Inline[][]; rows: Inline[][][] };

export type Align = "left" | "center" | "right" | null;

/** Split a table row into trimmed cell sources; a backslash-pipe is a literal pipe. */
function splitRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|") && !s.endsWith("\\|")) s = s.slice(0, -1);
  return s.split(/(?<!\\)\|/).map((c) => c.replace(/\\\|/g, "|").trim());
}

const ALIGN_CELL = /^:?-+:?$/;

/** The column alignments of a `|---|:-:|` row, or null if the line is not one. */
function parseAlignRow(line: string): Align[] | null {
  if (!line.includes("-") || !line.includes("|")) return null;
  const cells = splitRow(line);
  if (!cells.every((c) => ALIGN_CELL.test(c))) return null;
  return cells.map((c) => {
    const l = c.startsWith(":");
    const r = c.endsWith(":");
    return l && r ? "center" : r ? "right" : l ? "left" : null;
  });
}

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
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!.trimEnd();
    // A table is a header row, an alignment row of the same width, then body rows.
    const align = line.includes("|") && i + 1 < lines.length ? parseAlignRow(lines[i + 1]!.trimEnd()) : null;
    const headCells = align ? splitRow(line) : [];
    if (align && headCells.length === align.length) {
      flushPara();
      flushList();
      const rows: Inline[][][] = [];
      let j = i + 2;
      for (; j < lines.length && lines[j]!.includes("|"); j++) {
        const cells = splitRow(lines[j]!);
        // Ragged rows are padded or cut to the header's width, as GitHub does.
        rows.push(Array.from({ length: align.length }, (_, c) => parseInline(cells[c] ?? "")));
      }
      blocks.push({ t: "table", align, head: headCells.map(parseInline), rows });
      i = j - 1;
      continue;
    }
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

function tableNode(b: Extract<Block, { t: "table" }>): HTMLElement {
  const cell = (tag: "th" | "td", inline: Inline[], align: Align) => {
    const el = document.createElement(tag);
    el.append(...inlineNodes(inline));
    if (align) el.style.textAlign = align;
    return el;
  };
  const table = document.createElement("table");
  const headRow = document.createElement("tr");
  b.head.forEach((c, i) => headRow.append(cell("th", c, b.align[i] ?? null)));
  const thead = document.createElement("thead");
  thead.append(headRow);
  const tbody = document.createElement("tbody");
  for (const r of b.rows) {
    const tr = document.createElement("tr");
    r.forEach((c, i) => tr.append(cell("td", c, b.align[i] ?? null)));
    tbody.append(tr);
  }
  table.append(thead, tbody);
  // Wrapper so a wide table scrolls inside the narrow sidebar.
  const wrap = document.createElement("div");
  wrap.className = "table-wrap";
  wrap.append(table);
  return wrap;
}

/** Build DOM for a report. Text goes through textContent only. Headings shift down two levels for the sidebar. */
export function renderMarkdown(src: string, into: HTMLElement): void {
  into.replaceChildren();
  for (const b of parseMarkdown(src)) {
    if (b.t === "table") {
      into.append(tableNode(b));
    } else if ("inline" in b) {
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
