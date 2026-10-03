# Page sensor

TypeScript, injected into an **isolated world** on every document (CDP
`Page.addScriptToEvaluateOnNewDocument { worldName }` on CEF, `WKContentWorld`
on iOS). Produces JSON that deserializes into `core_types::Observation` (ADR-006).

What it does:

- assigns stable, unforgeable `ref` ids to interactive elements (WeakMap in the
  isolated world; the page cannot read or spoof them);
- extracts roles/accessible names/state; marks masked fields (`password`,
  `cc-*`, OTP) and never reports their values;
- extracts main content as heading-aware blocks, packed into `obs_id` chunks;
- keeps hidden/off-screen text that looks like model instructions out of
  `content` and reports it as `hidden_text_signals`;
- collects WebMCP site tools (declarative `<form toolname>` and a host-provided
  snapshot of `navigator.modelContext`);
- runs only at idle (`requestIdleCallback`), debounced to ≤ 1 snapshot / 500 ms,
  time-boxed to 40 ms per snapshot (budget: Page Intelligence idle < 5% CPU).

What it never does: dispatch input, read cookies/storage, traverse cross-origin
frames, decide sensitivity (the host derives it from origin + session).

```sh
cd sensor
npm install
npm run typecheck
npm run build      # dist/sensor.js, bundled into the CEF resources / iOS app
```

Host bindings: CEF exposes `__browseEmit` via `Runtime.addBinding` in the same
world; WKWebView exposes `webkit.messageHandlers.browse`. `installDefaultHost()`
picks whichever exists.
