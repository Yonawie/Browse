"use strict";
var BrowseSensor = (() => {
  var __defProp = Object.defineProperty;
  var __getOwnPropDesc = Object.getOwnPropertyDescriptor;
  var __getOwnPropNames = Object.getOwnPropertyNames;
  var __hasOwnProp = Object.prototype.hasOwnProperty;
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, { get: all[name], enumerable: true });
  };
  var __copyProps = (to, from, except, desc) => {
    if (from && typeof from === "object" || typeof from === "function") {
      for (let key of __getOwnPropNames(from))
        if (!__hasOwnProp.call(to, key) && key !== except)
          __defProp(to, key, { get: () => from[key], enumerable: !(desc = __getOwnPropDesc(from, key)) || desc.enumerable });
    }
    return to;
  };
  var __toCommonJS = (mod) => __copyProps(__defProp({}, "__esModule", { value: true }), mod);

  // src/sensor.ts
  var sensor_exports = {};
  __export(sensor_exports, {
    DEFAULT_OPTIONS: () => DEFAULT_OPTIONS,
    installDefaultHost: () => installDefaultHost,
    readMore: () => readMore,
    resolveRef: () => resolveRef,
    snapshot: () => snapshot,
    watch: () => watch
  });
  var DEFAULT_OPTIONS = {
    frameId: "main",
    maxInteractive: 150,
    maxContentChars: 24e3,
    // Cold Chromium processes (notably macOS CI runners) can spend more than
    // 40 ms computing accessibility metadata for even a small form. Keep a
    // bounded budget, but leave enough headroom to avoid partial observations.
    timeBudgetMs: 100
  };
  var refs = /* @__PURE__ */ new WeakMap();
  var byRef = /* @__PURE__ */ new Map();
  var nextRef = 1;
  function refFor(el) {
    let id = refs.get(el);
    if (id === void 0) {
      id = nextRef++;
      refs.set(el, id);
      byRef.set(id, new WeakRef(el));
    }
    return id;
  }
  function resolveRef(id) {
    return byRef.get(id)?.deref() ?? null;
  }
  var INTERACTIVE_SELECTOR = [
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
    "[tabindex]:not([tabindex='-1'])"
  ].join(",");
  var MASKED_AUTOCOMPLETE = /^(cc-|one-time-code|current-password|new-password)/;
  function roleOf(el) {
    const explicit = el.getAttribute("role");
    if (explicit) return explicit;
    const tag = el.tagName.toLowerCase();
    if (tag === "a") return "link";
    if (tag === "button" || tag === "summary") return "button";
    if (tag === "select") return "combobox";
    if (tag === "textarea") return "textbox";
    if (tag === "input") {
      const t = (el.type || "text").toLowerCase();
      if (t === "checkbox" || t === "radio") return t;
      if (t === "submit" || t === "button" || t === "image" || t === "reset") return "button";
      if (t === "range") return "slider";
      return "textbox";
    }
    if (el.isContentEditable) return "textbox";
    return "generic";
  }
  function accessibleName(el) {
    const aria = el.getAttribute("aria-label");
    if (aria) return aria.trim();
    const labelledBy = el.getAttribute("aria-labelledby");
    if (labelledBy) {
      const txt = labelledBy.split(/\s+/).map((id) => document.getElementById(id)?.textContent ?? "").join(" ").trim();
      if (txt) return txt;
    }
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) {
      const labels = el.labels;
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
  function isMasked(el) {
    if (el instanceof HTMLInputElement) {
      if (el.type === "password") return true;
      if (MASKED_AUTOCOMPLETE.test(el.autocomplete || "")) return true;
      if (/(card|cvv|cvc|iban|ssn|passport|otp|pin)/i.test(el.name + " " + (el.id ?? ""))) return true;
    }
    return false;
  }
  function isVisible(el) {
    const style = getComputedStyle(el);
    if (style.display === "none" || style.visibility === "hidden" || parseFloat(style.opacity) === 0) return false;
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) return false;
    if (el.getAttribute("aria-hidden") === "true") return false;
    return true;
  }
  function inViewport(r) {
    return r.bottom > 0 && r.right > 0 && r.top < innerHeight && r.left < innerWidth;
  }
  function landmarkOf(el) {
    const lm = el.closest("main,nav,header,footer,aside,form,[role=main],[role=navigation],[role=dialog],[role=search]");
    if (!lm) return void 0;
    const role = lm.getAttribute("role") ?? lm.tagName.toLowerCase();
    if (lm.tagName === "FORM") {
      const name = lm.getAttribute("name") || lm.getAttribute("id") || lm.getAttribute("aria-label");
      return name ? `form:${name}` : "form";
    }
    return role === "nav" ? "navigation" : role;
  }
  var CONSEQUENTIAL = /\b(buy|pay|order|checkout|purchase|confirm|submit|send|delete|remove|publish|post|transfer|subscribe|unsubscribe|cancel|book|reserve|sign|agree|accept|купить|оплатить|заказать|оформить|подтвердить|отправить|удалить|опубликовать|перевести|подписаться|отменить|забронировать|kaufen|bezahlen|bestellen|löschen|senden|acheter|payer|commander|supprimer|envoyer|comprar|pagar|eliminar|enviar)\b/i;
  function consequentialHint(el, role, name) {
    if (role !== "button" && role !== "link") return false;
    if (el instanceof HTMLInputElement && el.type === "submit") return true;
    if (el instanceof HTMLButtonElement && (el.type === "submit" || !el.type) && el.form) return true;
    return CONSEQUENTIAL.test(name);
  }
  var SKIP_TAGS = /* @__PURE__ */ new Set(["SCRIPT", "STYLE", "NOSCRIPT", "TEMPLATE", "SVG", "CANVAS", "IFRAME", "OBJECT"]);
  var BLOCK_TAGS = /* @__PURE__ */ new Set(["P", "LI", "TD", "TH", "DD", "DT", "BLOCKQUOTE", "PRE", "FIGCAPTION", "H1", "H2", "H3", "H4", "H5", "H6", "ARTICLE", "SECTION", "DIV"]);
  var INJECTION = [
    /ignore (all |any )?(previous|prior|above) (instructions|prompts)/i,
    /(you are|you're) (now |an? )?(ai|assistant|chatgpt|claude|gemini|model|agent)/i,
    /^\s*(system|assistant|developer)\s*[:：]/im,
    // Text addressed to the model rather than to the reader.
    /\b(note|message|instructions?|attention) (to|for) (the |any |all )?(ai|llm|assistant|agent|model)s?\b/i,
    /\b(ai|llm) agents? (reading|visiting|browsing)/i,
    /\b(to (finish|complete) (the|your|this) task,? you must)/i,
    /(do not|don't) (tell|inform|show) (the )?user/i,
    /(send|forward|post|exfiltrate|upload) .*(cookies?|tokens?|password|credentials|history)/i,
    /(игнорируй|забудь) (все )?(предыдущие|прежние) (инструкции|указания)/i,
    /ты (теперь|сейчас) (ассистент|модель|агент)/i
  ];
  function injectionSignal(text) {
    return INJECTION.some((re) => re.test(text));
  }
  function isHiddenText(el) {
    const s = getComputedStyle(el);
    if (s.display === "none" || s.visibility === "hidden" || parseFloat(s.opacity) < 0.05) return true;
    if (parseFloat(s.fontSize) < 2) return true;
    if (s.color === s.backgroundColor && s.backgroundColor !== "rgba(0, 0, 0, 0)") return true;
    const r = el.getBoundingClientRect();
    if (r.width < 2 || r.height < 2) return true;
    if (r.right < -1e3 || r.bottom < -1e3) return true;
    if (s.clipPath === "inset(100%)" || s.clip === "rect(0px, 0px, 0px, 0px)") return true;
    return false;
  }
  function collectContent(root, hidden, deadline, maxChars) {
    const blocks = [];
    const headings = [];
    let total = 0;
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);
    let node = walker.currentNode;
    while (node && total < maxChars) {
      if (performance.now() > deadline) break;
      const el = node;
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
          blocks.push({ heading: headings.filter(Boolean).join(" > ") || void 0, text: t });
          total += t.length;
        }
        node = nextSkippingChildren(walker);
        continue;
      }
      node = walker.nextNode();
    }
    return blocks;
  }
  function hasBlockChildren(el) {
    for (const c of el.children) if (BLOCK_TAGS.has(c.tagName) || /^H[1-6]$/.test(c.tagName)) return true;
    return false;
  }
  function nextSkippingChildren(walker) {
    let n = walker.nextSibling();
    while (!n) {
      if (!walker.parentNode()) return null;
      n = walker.nextSibling();
    }
    return n;
  }
  var lastChunks = /* @__PURE__ */ new Map();
  function readMore(obsId) {
    return lastChunks.get(obsId) ?? null;
  }
  function packChunks(blocks) {
    const chunks = [];
    let cur = [];
    let curHeading;
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
        suspect_injection: injectionSignal(text) || void 0
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
    lastChunks.clear();
    for (const c of chunks) lastChunks.set(c.obs_id, c.text);
    return chunks;
  }
  function collectSiteTools() {
    const tools = [];
    document.querySelectorAll("form[toolname]").forEach((f) => {
      tools.push({
        name: f.getAttribute("toolname") ?? "",
        description: f.getAttribute("tooldescription") ?? "",
        input_schema: formSchema(f),
        read_only_hint: (f.getAttribute("method") ?? "get").toLowerCase() === "get",
        consequential_hint: (f.getAttribute("method") ?? "get").toLowerCase() !== "get",
        untrusted_content_hint: true,
        source: "declarative"
      });
    });
    const snapshot2 = globalThis.__browseSiteTools;
    if (Array.isArray(snapshot2)) for (const t of snapshot2) tools.push({ ...t, source: "imperative", untrusted_content_hint: true });
    return tools;
  }
  function formSchema(f) {
    const props = {};
    for (const el of Array.from(f.elements)) {
      const name = el.name;
      if (!name) continue;
      props[name] = { type: "string", description: accessibleName(el) };
    }
    return { type: "object", properties: props };
  }
  function pageKind(interactive, blocks) {
    const url = location.href.toLowerCase();
    if (/\/(cart|checkout|basket|payment|order)/.test(url) || interactive.some((e) => e.input_type?.startsWith("cc-"))) return "checkout";
    if (interactive.some((e) => e.state.masked && e.input_type === "password")) return "auth";
    if (/\/(search|results)\b|[?&]q=/.test(url)) return "search";
    if (document.querySelector("article") && blocks.length >= 3) return "article";
    if (document.querySelectorAll("video").length && /watch|video/.test(url)) return "media";
    if (document.querySelector("[itemtype*='Product'], .product, #checkout") || /\/p\//.test(url)) return "product";
    if (interactive.filter((e) => e.role === "textbox").length >= 3) return "form";
    return "unknown";
  }
  function hasSession() {
    return Boolean(
      document.querySelector("[href*='logout'],[href*='signout'],[href*='sign-out'],[data-testid*='account'],[aria-label*='account' i]")
    );
  }
  function fnv1a(s) {
    let h = 2166136261;
    for (let i = 0; i < s.length; i++) {
      h ^= s.charCodeAt(i);
      h = Math.imul(h, 16777619) >>> 0;
    }
    return h.toString(16).padStart(8, "0");
  }
  function snapshot(opts = {}) {
    const o = { ...DEFAULT_OPTIONS, ...opts };
    const t0 = performance.now();
    const deadline = t0 + o.timeBudgetMs;
    const interactive = [];
    const candidates = document.querySelectorAll(INTERACTIVE_SELECTOR);
    for (const el of Array.from(candidates)) {
      if (interactive.length >= o.maxInteractive || performance.now() > deadline) break;
      if (!isVisible(el)) continue;
      const role = roleOf(el);
      const name = accessibleName(el);
      if (!name && role !== "textbox" && role !== "combobox") continue;
      const rect = el.getBoundingClientRect();
      const masked = isMasked(el);
      const input = el;
      const inputType = el instanceof HTMLInputElement ? input.autocomplete?.startsWith("cc-") ? input.autocomplete : input.type : void 0;
      const e = {
        ref: { id: refFor(el), path: `${role}/${name.slice(0, 40)}/${interactive.length}` },
        role,
        name,
        state: {
          disabled: el.disabled || el.getAttribute("aria-disabled") === "true" || void 0,
          checked: el instanceof HTMLInputElement && (el.type === "checkbox" || el.type === "radio") ? el.checked : void 0,
          expanded: el.hasAttribute("aria-expanded") ? el.getAttribute("aria-expanded") === "true" : void 0,
          focused: document.activeElement === el || void 0,
          required: el.required || el.getAttribute("aria-required") === "true" || void 0,
          masked: masked || void 0
        },
        value: !masked && (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) && input.value ? input.value.slice(0, 200) : void 0,
        href: el instanceof HTMLAnchorElement ? el.href : void 0,
        input_type: inputType,
        bbox: { x: Math.round(rect.x), y: Math.round(rect.y), w: Math.round(rect.width), h: Math.round(rect.height) },
        in_viewport: inViewport(rect),
        landmark: landmarkOf(el)
      };
      if (consequentialHint(el, role, name)) e.consequential_hint = true;
      interactive.push(e);
    }
    const hidden = [];
    const root = document.querySelector("main,[role=main],article") ?? document.body;
    const blocks = collectContent(root, hidden, deadline, o.maxContentChars);
    const content = packChunks(blocks);
    const tools = collectSiteTools();
    const serialized = JSON.stringify({ u: location.href, i: interactive.map((e) => [e.ref.id, e.role, e.name, e.value ?? ""]), c: content.map((c) => c.text) });
    const approx_tokens = Math.ceil(serialized.length / 4);
    const kind = pageKind(interactive, blocks);
    const obs = {
      page: {
        url: location.href,
        origin: location.origin,
        title: document.title,
        lang: document.documentElement.lang || void 0,
        page_kind: kind,
        untrusted: true,
        frame_id: o.frameId,
        snapshot_hash: fnv1a(serialized),
        // The sensor never decides sensitivity by content; the host computes it
        // from origin + session (ADR-007). This is only the "has_session" input.
        sensitivity: "public",
        has_session: hasSession()
      },
      interactive,
      content,
      tools,
      hidden_text_signals: hidden,
      approx_tokens,
      captured_at: Date.now()
    };
    return obs;
  }
  function watch(emit, opts = {}) {
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
      const ric = globalThis.requestIdleCallback;
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
  function installDefaultHost() {
    const g = globalThis;
    const emit = (obs) => {
      const json = JSON.stringify(obs);
      if (typeof __browseEmit === "function") __browseEmit(json);
      else g.webkit?.messageHandlers?.browse?.postMessage(json);
    };
    watch(emit);
  }
  return __toCommonJS(sensor_exports);
})();
