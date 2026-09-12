# iOS shell (increment I6)

Apple requires WebKit for browsers distributed worldwide (BrowserEngineKit is
EU-only and not used), so the iOS app is a SwiftUI shell over `WKWebView`
(ADR-001). It shares everything below the Shell with desktop:

| Layer | iOS implementation |
|---|---|
| Shell | SwiftUI: tabs, omnibox, side panel, confirmation sheets, memory inspector |
| Engine adapter | Swift `WKWebViewEngine` implementing the UniFFI callback interface generated from `engine-adapter::EngineAdapter`; capabilities = `Capabilities::WKWEBVIEW` (no accessibility tree/network events/trusted input; actions are dispatched by the sensor's sibling script in the same `WKContentWorld`) |
| Sensor | `sensor/dist/sensor.js` as a `WKUserScript` in a dedicated `WKContentWorld`; emits via `webkit.messageHandlers.browse` |
| Page Intelligence, Memory, Policy, Agent Runtime | Rust crates compiled to an XCFramework via `uniffi-bindgen` (`crates/*` → `BrowseCore.xcframework`) |
| Model Gateway | `FoundationModelsProvider` (Apple Foundation Models, ~3B on-device, `Locality::Local`, tiers `fast`) and `CoreMLEmbedProvider` (EmbeddingGemma-300M 256-dim). `smart`/`vision` fall back to a user-configured provider or are unavailable offline. |
| Storage | Same `schema/memory.sql` in the app group container; `sqlite-vec` linked statically; iCloud sync is **off** (privacy model §3) |

Isolated agent profile = a separate non-persistent `WKWebsiteDataStore` per
session; one-origin session import uses `WKHTTPCookieStore` after explicit
confirmation, exactly as on desktop.

Not started yet; this document pins the decisions so the Rust core is not
shaped in a way that would block it (no `Send`-hostile types across the FFI
boundary, all IPC types `serde`-serializable, no CDP-only assumptions outside
`engine-adapter` backends).
