/**
 * Browse page sensor (ADR-006).
 *
 * Runs in an *isolated world* (CDP `Page.addScriptToEvaluateOnNewDocument`
 * with `worldName`, or `WKContentWorld` on iOS), so the page's own scripts
 * cannot see or tamper with it. It produces a compact semantic snapshot that
 * mirrors `core_types::Observation` byte-for-byte in JSON shape.
 *
 * Invariants:
 *  - The sensor never *acts*. Actions go through the engine adapter using
 *    trusted input events on the `ref` ids this sensor assigned.
 *  - Masked fields (`type=password`, `autocomplete=cc-*`, `one-time-code`) are
 *    reported with `state.masked=true` and no value.
 *  - Hidden/off-screen text that looks like instructions is kept out of
 *    `content` and surfaced as `hidden_text_signals`.
 *  - Cross-origin iframes are not traversed; the host injects the sensor into
 *    granted frames separately (each yields its own `frame_id`).
 *  - The sensor only runs at idle (`requestIdleCallback`) and is time-boxed.
 */

export interface ElementRef {
  id: number;
  path: string;
}

export interface ElementState {
  disabled?: boolean;
  checked?: boolean;
  expanded?: boolean;
  focused?: boolean;
  required?: boolean;
  masked?: boolean;
}

export interface BoundingBox {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface InteractiveElement {
  ref: ElementRef;
  role: string;
  name: string;
  state: ElementState;
  value?: string;
  href?: string;
  input_type?: string;
  bbox?: BoundingBox;
  in_viewport: boolean;
  landmark?: string;
  consequential_hint?: boolean;
}

export interface ContentChunk {
  obs_id: string;
  heading_path?: string;
  text: string;
  char_start: number;
  char_end: number;
  suspect_injection?: boolean;
}

export interface SiteTool {
  name: string;
  description: string;
  input_schema: unknown;
  read_only_hint: boolean;
  consequential_hint: boolean;
  untrusted_content_hint: boolean;
  source: "declarative" | "imperative";
}

export interface PageMeta {
  url: string;
  origin: string;
  title: string;
  lang?: string;
  page_kind: string;
  untrusted: true;
  frame_id: string;
  snapshot_hash: string;
  sensitivity: "public" | "personal" | "private" | "secret";
  has_session: boolean;
}

export interface Observation {
  page: PageMeta;
  interactive: InteractiveElement[];
  content: ContentChunk[];
  tools: SiteTool[];
  hidden_text_signals: string[];
  approx_tokens: number;
  captured_at: number;
}

export interface SensorOptions {
  frameId: string;
  /** Hard cap on interactive elements (LOCAL budget = 150). */
  maxInteractive: number;
  /** Hard cap on content characters before chunking. */
  maxContentChars: number;
  /** Time box in ms for one snapshot; partial snapshots are marked in `page_kind`. */
  timeBudgetMs: number;
}

export const DEFAULT_OPTIONS: SensorOptions = {
  frameId: "main",
  maxInteractive: 150,
  maxContentChars: 24_000,
  timeBudgetMs: 40,
};

// ---------------------------------------------------------------------------
// Ref assignment: stable ids per element for the lifetime of the document.
// Kept in a WeakMap so refs are unforgeable from the page's world.
// ---------------------------------------------------------------------------

const refs = new WeakMap<Element, number>();
const byRef = new Map<number, WeakRef<Element>>();
let nextRef = 1;

function refFor(el: Element): number {
  let id = refs.get(el);
  if (id === undefined) {
    id = nextRef++;
    refs.set(el, id);
    byRef.set(id, new WeakRef(el));
  }
  return id;
}

/** Used by the engine adapter to resolve a `ref` before dispatching input. */
export function resolveRef(id: number): Element | null {
  return byRef.get(id)?.deref() ?? null;
}

// ---------------------------------------------------------------------------
// Roles and names
// ---------------------------------------------------------------------------

const INTERACTIVE_SELECTOR = [
  "a[href]",
  "button",
  "input:not([type=hidden])",
  "select",
  "textarea",
  "summary",
  "[role=button]",
  "[role=link]",
  "[role=textbox]",
  "[role=combobox]",
  "[role=checkbox]",
  "[role=radio]",
  "[role=switch]",
  "[role=tab]",
  "[role=menuitem]",
  "[role=option]",
  "[contenteditable=true]",
  "[tabindex]:not([tabindex='-1'])",
].join(",");

const MASKED_AUTOCOMPLETE = /^(cc-|one-time-code|current-password|new-password)/;

function roleOf(el: Element): string {
  const explicit = el.getAttribute("role");
  if (explicit) return explicit;
  const tag = el.tagName.toLowerCase();
  if (tag === "a") return "link";
  if (tag === "button" || tag === "summary") return "button";
  if (tag === "select") return "combobox";
  if (tag === "textarea") return "textbox";
  if (tag === "input") {
    const t = ((el as HTMLInputElement).type || "text").toLowerCase();
    if (t === "checkbox" || t === "radio") return t;
    if (t === "submit" || t === "button" || t === "image" || t === "reset") return "button";
    if (t === "range") return "slider";
    return "textbox";
  }
  if ((el as HTMLElement).isContentEditable) return "textbox";
  return "generic";
}

function accessibleName(el: Element): string {
  const aria = el.getAttribute("aria-label");
  if (aria) return aria.trim();
  const labelledBy = el.getAttribute("aria-labelledby");
  if (labelledBy) {
    const txt = labelledBy
      .split(/\s+/)
      .map((id) => document.getElementById(id)?.textContent ?? "")
      .join(" ")
      .trim();
    if (txt) return txt;
  }
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
    const labels = (el as HTMLInputElement).labels;
    if (labels && labels.length) return (labels[0].textContent ?? "").trim();
    if (el instanceof HTMLInputElement && (el.type === "submit" || el.type === "button") && el.value) return el.value;
    if (!(el instanceof HTMLSelectElement) && el.placeholder) return el.placeholder;
    if (el.title) return el.title;
    if (el.name) return el.name;
  }
  const alt = el.querySelector("img[alt]")?.getAttribute("alt");
  if (alt) return alt.trim();
  return (el.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 120);
}

function isMasked(el: Element): boolean {
  if (el instanceof HTMLInputElement) {
    if (el.type === "password") return true;
    if (MASKED_AUTOCOMPLETE.test(el.autocomplete || "")) return true;
    if (/(card|cvv|cvc|iban|ssn|passport|otp|pin)/i.test(el.name + " " + (el.id ?? ""))) return true;
  }
  return false;
}

function isVisible(el: Element): boolean {
  const style = getComputedStyle(el);
  if (style.display === "none" || style.visibility === "hidden" || parseFloat(style.opacity) === 0) return false;
  const r = el.getBoundingClientRect();
  if (r.width === 0 && r.height === 0) return false;
  if (el.getAttribute("aria-hidden") === "true") return false;
  return true;
}

function inViewport(r: DOMRect): boolean {
  return r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth;
}

function landmarkOf(el: Element): string | undefined {
  const lm = el.closest("main,nav,header,footer,aside,form,[role=main],[role=navigation],[role=dialog],[role=search]");
  if (!lm) return undefined;
  const role = lm.getAttribute("role") ?? lm.tagName.toLowerCase();
  if (lm.tagName === "FORM") {
    const name = lm.getAttribute("name") || lm.getAttribute("id") || lm.getAttribute("aria-label");
    return name ? `form:${name}` : "form";
  }
  return role === "nav" ? "navigation" : role;
}

/** Multilingual lexicon for irreversible actions. PolicyEngine re-checks with its own copy. */
const CONSEQUENTIAL = /\b(buy|pay|order|checkout|purchase|confirm|submit|send|delete|remove|publish|post|transfer|subscribe|unsubscribe|cancel|book|reserve|sign|agree|accept|купить|оплатить|заказать|оформить|подтвердить|отправить|удалить|опубликовать|перевести|подписаться|отменить|забронировать|kaufen|bezahlen|bestellen|löschen|senden|acheter|payer|commander|supprimer|envoyer|comprar|pagar|eliminar|enviar)\b/i;

function consequentialHint(el: Element, role: string, name: string): boolean {
  if (role !== "button" && role !== "link") return false;
  if (el instanceof HTMLInputElement && el.type === "submit") return true;
  if (el instanceof HTMLButtonElement && (el.type === "submit" || !el.type) && el.form) return true;
  return CONSEQUENTIAL.test(name);
}

// ---------------------------------------------------------------------------
// Content extraction
// ---------------------------------------------------------------------------

const SKIP_TAGS = new Set(["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "SVG", "CANVAS", "IFRAME", "OBJECT"]);
const BLOCK_TAGS = new Set(["P", "LI", "TD", "TH", "DD", "DT", "BLOCKQUOTE", "PRE", "FIGCAPTION", "H1", "H2", "H3", "H4", "H5", "H6", "ARTICLE", "SECTION", "DIV"]);

/** Heuristics mirrored from `page_intelligence::injection` (signal only, never filters). */
const INJECTION = [
  /ignore (all |any )?(previous|prior|above) (instructions|prompts)/i,
  /(you are|you're) (now |an? )?(ai|assistant|chatgpt|claude|gemini|model|agent)/i,
  /^\s*(system|assistant|developer)\s*[:：]/im,
  /(do not|don't) (tell|inform|show) (the )?user/i,
  /(send|forward|post|exfiltrate|upload) .*(cookies?|tokens?|password|credentials|history)/i,
  /(игнорируй|забудь) (все )?(предыдущие|прежние) (инструкции|указания)/i,
  /ты (теперь|сейчас) (ассистент|модель|агент)/i,
];

function injectionSignal(text: string): boolean {
  return INJECTION.some((re) => re.test(text));
}

function isHiddenText(el: Element): boolean {
  const s = getComputedStyle(el);
  if (s.display === "none" || s.visibility === "hidden" || parseFloat(s.opacity) < 0.05) return true;
  if (parseFloat(s.fontSize) < 2) return true;
  if (s.color === s.backgroundColor && s.backgroundColor !== "rgba(0, 0, 0, 0)") return true;
  const r = el.getBoundingClientRect();
  if (r.width < 2 || r.height < 2) return true;
  if (r.right < -1000 || r.bottom < -1000) return true;
  if (s.clipPath === "inset(100%)" || s.clip === "rect(0px, 0px, 0px, 0px)") return true;
  return false;
}

interface Block {
  heading: string | undefined;
  text: string;
}

function collectContent(root: Element, hidden: string[], deadline: number, maxChars: number): Block[] {
  const blocks: Block[] = [];
  const headings: string[] = [];
  let total = 0;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);
  let node = walker.currentNode as Element | null;
  while (node && total < maxChars) {
    if (performance.now() > deadline) break;
    const el = node as Element;
    if (SKIP_TAGS.has(el.tagName) || el.closest("nav,header,footer,[role=navigation],[aria-hidden=true]")) {
      node = nextSkippingChildren(walker);
      continue;
    }
    if (/^H[1-6]$/.test(el.tagName)) {
      const level = Number(el.tagName[1]);
      headings.length = level - 1;
      headings[level - 1] = (el.textContent ?? "").trim();
      node = nextSkippingChildren(walker);
      continue;
    }
    if (isHiddenText(el)) {
      const t = (el.textContent ?? "").replace(/\s+/g, " ").trim();
      if (t.length > 20 && injectionSignal(t)) hidden.push(t.slice(0, 500));
      node = nextSkippingChildren(walker);
      continue;
    }
    if (BLOCK_TAGS.has(el.tagName) && !hasBlockChildren(el)) {
      const t = (el.textContent ?? "").replace(/\s+/g, " ").trim();
      if (t.length >= 12) {
        blocks.push({ heading: headings.filter(Boolean).join(" > ") || undefined, text: t });
        total += t.length;
      }
      node = nextSkippingChildren(walker);
      continue;
    }
    node = walker.nextNode() as Element | null;
  }
  return blocks;
}

function hasBlockChildren(el: Element): boolean {
  for (const c of el.children) if (BLOCK_TAGS.has(c.tagName) || /^H[1-6]$/.test(c.tagName)) return true;
  return false;
}

function nextSkippingChildren(walker: TreeWalker): Element | null {
  let n = walker.nextSibling();
  while (!n) {
    if (!walker.parentNode()) return null;
    n = walker.nextSibling();
  }
  return n as Element;
}

/** Coarse packing into ~300-token chunks; the Rust chunker re-chunks for indexing. */
function packChunks(blocks: Block[]): ContentChunk[] {
  const chunks: ContentChunk[] = [];
  let cur: string[] = [];
  let curHeading: string | undefined;
  let start = 0;
  let pos = 0;
  const flush = () => {
    if (!cur.length) return;
    const text = cur.join("\n");
    chunks.push({
      obs_id: `c${chunks.length}`,
      heading_path: curHeading,
      text,
      char_start: start,
      char_end: start + text.length,
      suspect_injection: injectionSignal(text) || undefined,
    });
    cur = [];
  };
  for (const b of blocks) {
    if (cur.length && (b.heading !== curHeading || cur.join("\n").length + b.text.length > 1200)) flush();
    if (!cur.length) {
      curHeading = b.heading;
      start = pos;
    }
    cur.push(b.text);
    pos += b.text.length + 1;
  }
  flush();
  return chunks;
}

// ---------------------------------------------------------------------------
// WebMCP site tools
// ---------------------------------------------------------------------------

function collectSiteTools(): SiteTool[] {
  const tools: SiteTool[] = [];
  // Declarative: <form toolname="..." tooldescription="...">
  document.querySelectorAll("form[toolname]").forEach((f) => {
    tools.push({
      name: f.getAttribute("toolname") ?? "",
      description: f.getAttribute("tooldescription") ?? "",
      input_schema: formSchema(f as HTMLFormElement),
      read_only_hint: (f.getAttribute("method") ?? "get").toLowerCase() === "get",
      consequential_hint: (f.getAttribute("method") ?? "get").toLowerCase() !== "get",
      untrusted_content_hint: true,
      source: "declarative",
    });
  });
  // Imperative: navigator.modelContext (origin trial). The page's world owns the
  // registry; we only read a snapshot the host exposes via a CDP binding.
  const snapshot = (globalThis as unknown as { __browseSiteTools?: SiteTool[] }).__browseSiteTools;
  if (Array.isArray(snapshot)) for (const t of snapshot) tools.push({ ...t, source: "imperative", untrusted_content_hint: true });
  return tools;
}

function formSchema(f: HTMLFormElement): unknown {
  const props: Record<string, unknown> = {};
  for (const el of Array.from(f.elements)) {
    const name = (el as HTMLInputElement).name;
    if (!name) continue;
    props[name] = { type: "string", description: accessibleName(el) };
  }
  return { type: "object", properties: props };
}

// ---------------------------------------------------------------------------
// Page kind, session, hashing
// ---------------------------------------------------------------------------

function pageKind(interactive: InteractiveElement[], blocks: Block[]): string {
  const url = location.href.toLowerCase();
  if (/\/(cart|checkout|basket|payment|order)/.test(url) || interactive.some((e) => e.input_type?.startsWith("cc-"))) return "checkout";
  if (interactive.some((e) => e.state.masked && e.input_type === "password")) return "login";
  if (/\/(search|results)\b|[?&]q=/.test(url)) return "search_results";
  if (document.querySelector("article") && blocks.length > 5) return "article";
  if (document.querySelectorAll("video").length && /watch|video/.test(url)) return "video";
  if (interactive.filter((e) => e.role === "textbox").length >= 3) return "form";
  return "unknown";
}

function hasSession(): boolean {
  // Cheap deterministic signals; the host also checks the cookie jar.
  return Boolean(
    document.querySelector("[href*='logout'],[href*='signout'],[href*='sign-out'],[data-testid*='account'],[aria-label*='account' i]"),
  );
}

function fnv1a(s: string): string {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(16).padStart(8, "0");
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

export function snapshot(opts: Partial<SensorOptions> = {}): Observation {
  const o = { ...DEFAULT_OPTIONS, ...opts };
  const t0 = performance.now();
  const deadline = t0 + o.timeBudgetMs;

  const interactive: InteractiveElement[] = [];
  const candidates = document.querySelectorAll(INTERACTIVE_SELECTOR);
  for (const el of Array.from(candidates)) {
    if (interactive.length >= o.maxInteractive || performance.now() > deadline) break;
    if (!isVisible(el)) continue;
    const role = roleOf(el);
    const name = accessibleName(el);
    if (!name && role !== "textbox" && role !== "combobox") continue;
    const rect = el.getBoundingClientRect();
    const masked = isMasked(el);
    const input = el as HTMLInputElement;
    const inputType = el instanceof HTMLInputElement ? (input.autocomplete?.startsWith("cc-") ? input.autocomplete : input.type) : undefined;
    const e: InteractiveElement = {
      ref: { id: refFor(el), path: `${role}/${name.slice(0, 40)}/${interactive.length}` },
      role,
      name,
      state: {
        disabled: (el as HTMLButtonElement).disabled || el.getAttribute("aria-disabled") === "true" || undefined,
        checked: el instanceof HTMLInputElement && (el.type === "checkbox" || el.type === "radio") ? el.checked : undefined,
        expanded: el.hasAttribute("aria-expanded") ? el.getAttribute("aria-expanded") === "true" : undefined,
        focused: document.activeElement === el || undefined,
        required: (el as HTMLInputElement).required || el.getAttribute("aria-required") === "true" || undefined,
        masked: masked || undefined,
      },
      value: !masked && (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) && input.value ? input.value.slice(0, 200) : undefined,
      href: el instanceof HTMLAnchorElement ? el.href : undefined,
      input_type: inputType,
      bbox: { x: Math.round(rect.x), y: Math.round(rect.y), w: Math.round(rect.width), h: Math.round(rect.height) },
      in_viewport: inViewport(rect),
      landmark: landmarkOf(el),
    };
    if (consequentialHint(el, role, name)) e.consequential_hint = true;
    interactive.push(e);
  }

  const hidden: string[] = [];
  const root = document.querySelector("main,[role=main],article") ?? document.body;
  const blocks = collectContent(root, hidden, deadline, o.maxContentChars);
  const content = packChunks(blocks);
  const tools = collectSiteTools();

  const serialized = JSON.stringify({ u: location.href, i: interactive.map((e) => [e.ref.id, e.role, e.name, e.value ?? ""]), c: content.map((c) => c.text) });
  const approx_tokens = Math.ceil(serialized.length / 4);

  const kind = pageKind(interactive, blocks);
  const obs: Observation = {
    page: {
      url: location.href,
      origin: location.origin,
      title: document.title,
      lang: document.documentElement.lang || undefined,
      page_kind: kind,
      untrusted: true,
      frame_id: o.frameId,
      snapshot_hash: fnv1a(serialized),
      // The sensor never decides sensitivity by content; the host computes it
      // from origin + session (ADR-007). This is only the "has_session" input.
      sensitivity: "public",
      has_session: hasSession(),
    },
    interactive,
    content,
    tools,
    hidden_text_signals: hidden,
    approx_tokens,
    captured_at: Date.now(),
  };
  return obs;
}

// ---------------------------------------------------------------------------
// Idle-time mutation watcher: debounced, never more than one snapshot per
// 500 ms, only when the document is visible. Emits via the host binding.
// ---------------------------------------------------------------------------

type Emit = (obs: Observation) => void;

export function watch(emit: Emit, opts: Partial<SensorOptions> = {}): () => void {
  let dirty = true;
  let scheduled = false;
  let last = 0;
  const run = () => {
    scheduled = false;
    if (!dirty || document.visibilityState !== "visible") return;
    const now = performance.now();
    if (now - last < 500) {
      schedule();
      return;
    }
    dirty = false;
    last = now;
    emit(snapshot(opts));
  };
  const schedule = () => {
    if (scheduled) return;
    scheduled = true;
    const ric = (globalThis as unknown as { requestIdleCallback?: (cb: () => void, o?: { timeout: number }) => number }).requestIdleCallback;
    if (ric) ric(run, { timeout: 1500 });
    else setTimeout(run, 300);
  };
  const mo = new MutationObserver(() => {
    dirty = true;
    schedule();
  });
  mo.observe(document.documentElement, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ["disabled", "aria-expanded", "value", "hidden", "class"] });
  document.addEventListener("visibilitychange", schedule);
  schedule();
  return () => {
    mo.disconnect();
    document.removeEventListener("visibilitychange", schedule);
  };
}

// Host binding: CEF exposes `__browseEmit` via CDP `Runtime.addBinding` in the
// isolated world; WKWebView via `webkit.messageHandlers.browse` in the content world.
declare const __browseEmit: ((json: string) => void) | undefined;

export function installDefaultHost(): void {
  const g = globalThis as unknown as { webkit?: { messageHandlers?: { browse?: { postMessage: (m: string) => void } } } };
  const emit: Emit = (obs) => {
    const json = JSON.stringify(obs);
    if (typeof __browseEmit === "function") __browseEmit(json);
    else g.webkit?.messageHandlers?.browse?.postMessage(json);
  };
  watch(emit);
}
