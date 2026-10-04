# Browse — умный браузер

Browse — концепт и реализация браузера с ИИ-ядром, где модель встроена в ядро браузера (понимание страниц, память, агентные действия), а не является изолированным боковым чатом.

Принципы:
- **Безопасность архитектурная, а не модельная** (детерминированная защита от prompt-инъекций, разграничение прав, подтверждение значимых действий).
- **Локально по умолчанию** (инференс через `llama-server` GGUF на железе класса ~8 ГБ RAM / RTX 4060, опциональное облако BYOK только для публичных данных).
- **Нулевая телеметрия** и полная приватность пользовательских данных.

---

An AI-native browser: the model is part of the core (page understanding,
memory, an agent that acts on pages), not a chat button bolted onto Chrome.
Windows + iOS, local-first inference on ~8 GB RAM / RTX 4060-class hardware,
open-source licenses only, **zero telemetry**.

This repository contains the engineering documents and the code so far:
domain types, the deterministic policy engine, the SQLite memory schema and
store, the model gateway (routing + VRAM budget), the agent runtime, an
engine-adapter trait with a mock backend **and a real Chromium backend over
the DevTools Protocol**, the page sensor, local fixture sites for tests, and a
headless desktop binary that runs the whole pipeline end-to-end.

## Run it in 5 minutes

Prerequisites: Rust stable ≥ 1.88 ([rustup](https://rustup.rs)); a
Chromium-based browser (Chrome, Chromium or Edge) for the engine's end-to-end
tests (they skip themselves if none is found; point `BROWSE_CHROME` at a
binary to override detection); Node ≥ 20 only if you change the sensor.

```sh
git clone https://github.com/Yonawie/Browse && cd Browse
cargo test --workspace --all-features         # includes headless Chromium E2E; live-model tests need a server
cargo run -p browse-desktop -- demo           # red-team demo of the policy layers (mock engine)
cargo run -p fixtures                         # serve the fixture sites on http://*.localhost:8765
```

### Page Intelligence CLI

Configure a local or optional cloud model as described in [Local models](#local-models), then run one vertical page workflow:

```sh
cargo run -p browse-desktop -- page https://example.com summarize
cargo run -p browse-desktop -- page https://example.com ask "What changed?"
cargo run -p browse-desktop -- page https://example.com translate Russian
```

The command launches an isolated headless Chromium profile, extracts a locally budgeted observation, streams the
answer with source citations `[c0]`, and automatically verifies citation validity against the extracted page chunks.
Page content, title, and URL are encoded as untrusted JSON data in the model prompt; this is not a guarantee against prompt injection.
Private pages can only use a local provider. Public-page summaries may use a configured cloud endpoint.
Free-form questions and translation requests are classified at least Personal and do not opt into cloud routing,
so these commands require a local provider. Live inference requires a separately configured model server;
tests that skip without one do not validate model answer quality.

### Memory & Search CLI

Index web pages into local SQLite memory, run hybrid semantic searches (lexical FTS5 + vector embeddings) and manage history erasure:

```sh
cargo run -p browse-desktop -- memory stats
cargo run -p browse-desktop -- memory index https://example.com
cargo run -p browse-desktop -- memory search "search query" [--domain example.com]
cargo run -p browse-desktop -- memory forget example.com
```

### Agent Run CLI

Execute autonomous browser tasks with deterministic policy validation, blind critic, interactive or auto-reject confirmation gates, and full SQLite auditing:

```sh
cargo run -p browse-desktop -- run https://example.com "find pricing plans" --dry-run
cargo run -p browse-desktop -- run https://example.com "sign up for newsletter" --interactive
```

### WebUI Shell & Server (Browser UI)

Launch the complete AI-native browser graphical shell with tab strip, task groups, omnibox, and AI assistant panel:

```sh
cargo run -p browse-desktop -- shell                     # serves WebUI and launches local browser
cargo run -p browse-desktop -- shell --port 8080 --no-open
```

Features:
- **Task-Based Tab Strip:** Automatic tab clustering by topic (`[Shopping]`, `[Documentation]`, `[Development]`).
- **Omnibox & Security Tag:** Natural language query dispatcher and real-time security shield tag (`Safe 🛡️` / `Suspicious ⚠` / `Dangerous 🚨`).
- **AI Assistant Panel:** Page summarization, citation inspection, autonomous agent runner with action confirmations, and local SQLite memory search.
- **Shortcuts & Themes:** `Ctrl+T` (New tab), `Ctrl+W` (Close), `Ctrl+L` (Omnibox), `Ctrl+B` (AI sidebar), `Ctrl+1..9` (Tab switch), Light/Dark theme toggle.

### Safety & Phishing Inspection CLI

Evaluate any URL and page content for homoglyph attacks, brand spoofing, and deceptive design patterns (FTC / EU DSA compliance):

```sh
cargo run -p browse-desktop -- inspect-safety https://paypal-security-update.com/login --text "Only 2 items left! Renews automatically at $49/mo."
```

### Model Context Protocol (MCP) Server for External Agents

Browse provides a built-in MCP server (`browse-desktop mcp`) allowing external coding and thinking agents (Claude Desktop, Cursor, Claude Code) to browse the web through Browse's secure, policy-governed runtime:

```sh
cargo run -p browse-desktop -- mcp [--dev-mode] [--auth-token <secret>] [--origin <origin>]
```

Configure in Claude Desktop (`claude_desktop_config.json`) or Cursor (`mcp.json`):
```json
{
  "mcpServers": {
    "browse": {
      "command": "cargo",
      "args": ["run", "-p", "browse-desktop", "--", "mcp", "--origin", "https://*"]
    }
  }
}
```
Exposed tools: `browse_navigate`, `browse_observe`, `browse_click`, `browse_type`, `browse_extract`. Every external action is validated by Browse's deterministic `PolicyEngine` (ADR-005) against indirect prompt injections and unauthorized data exfiltration.


With the fixture server running, open `http://shop.localhost:8765`,
`http://forms.localhost:8765/contact`, `http://login.localhost:8765` (demo /
demo) or `http://hostile.localhost:8765` in any browser; `*.localhost` resolves
to loopback without configuration. Sensor changes: `cd sensor && npm ci && npm
test` (rebuilds `sensor/dist/sensor.iife.js`, which is committed and embedded
into `engine-cdp` at compile time so `cargo build` never needs Node).

Progress, measurements and known limitations per stage are tracked in
[`docs/03-status.md`](docs/03-status.md).

## Documents

| Document | Contents |
|---|---|
| [`docs/01-research.md`](docs/01-research.md) | Phase 1: market landscape (Arc/Dia, Comet, Brave Leo, Opera Neon, Chrome Gemini, Edge Copilot, Vivaldi, SigmaOS, Zen, Operator, Browser Use, Playwright agents), capability map by layer, technical constraints and risks, engine comparison and choice, AI stack, ADR summary |
| [`docs/02-architecture.md`](docs/02-architecture.md) | Phase 2: layers, agent security model, privacy model, performance budgets, extensibility, data schema, repo layout, increment roadmap |
| [`docs/03-status.md`](docs/03-status.md) | Phases 3–4: per-stage status, build/test results, performance measurements vs budgets, limitations, next steps |
| [`docs/adr/`](docs/adr/) | ADR-001 engine (CEF + WKWebView behind one adapter) · ADR-002 language/stack (Rust core, TS shell/sensor, SwiftUI iOS) · ADR-003 local inference (llama-server sidecar, model preset for 8 GB VRAM) · ADR-004 memory store (SQLite + sqlite-vec + FTS5) · ADR-005 agent security · ADR-006 observation format · ADR-007 routing/privacy · ADR-008 extensibility |

## Layout

```
Cargo.toml                 Rust workspace
crates/
  core-types/              Observation, Provenance, Sensitivity, ToolCall/Manifest, TaskScope, Origin
  policy/                  PolicyEngine: deterministic allow/confirm/deny, sensitivity classes, grants, consequential lexicon
  page-intelligence/       chunker, injection signals, page kind, observation budgets
  memory/                  schema/memory.sql migrations, MemoryStore, hybrid search (BM25 + vector, RRF), append-only journal
  engine-adapter/          EngineAdapter trait, action/event vocabulary, MockEngine
  engine-cdp/              Chromium over the DevTools Protocol: isolated-world sensor, ref-based trusted input,
                           per-profile browser contexts, cookies, AX tree, screenshots; headless E2E tests
  fixtures/                local fixture sites (shop, shop2, forms, news, login, hostile, evil) on *.localhost
  model-gateway/           ModelProvider (chat, streaming, tools, JSON schema, embeddings), sensitivity-aware Router,
                           OpenAI-compatible provider (local llama-server / cloud), reqwest+SSE transport,
                           Gateway (cache, limits, model_calls log), VRAM ModelManager, llama-server sidecar pool
  agent-runtime/           observe → plan → label refs → policy → blind critic → confirm/dry-run → act → journal
apps/
  desktop/                 browse-desktop binary (mock engine demo), SQLite journal bridge
  ios/                     SwiftUI + WKWebView + UniFFI notes
sensor/                    TypeScript page sensor for the isolated world; dist/ is the committed esbuild bundle
schema/memory.sql          SQLite DDL (WAL, FTS5, vec0, append-only triggers)
schemas/                   JSON Schema: tool-manifest, task-scope, policy
.github/workflows/ci.yml   fmt, clippy (-D warnings), build, test on Windows/Linux/macOS + sensor job
```

## Build and test

```sh
cargo test --workspace --all-features        # everything, including Chromium E2E when a browser is present
cargo test -p engine-cdp --test e2e          # only the engine E2E suite
cargo clippy --workspace --all-targets --all-features && cargo fmt --all -- --check
cargo run -p browse-desktop -- schema-check  # apply DDL to an in-memory DB
cargo run -p browse-desktop -- demo          # headless red-team demo (see below)
cargo run -p browse-desktop -- demo --dry-run
cargo run -p browse-desktop -- models        # model routing table + streamed smoke prompt
```

### Local models

Any [llama.cpp](https://github.com/ggml-org/llama.cpp) `llama-server` works; the
alias is read from `/v1/models`. Start one chat server and one embedding server
(the ADR-003 preset is Qwen3-4B / Qwen3-8B + EmbeddingGemma-300M; smaller
models run on CPU-only machines), then point the binary at them:

```sh
llama-server -m qwen3-4b-instruct-q4_k_m.gguf --alias fast --port 8081 --jinja -c 8192 -ngl 99
llama-server -m embeddinggemma-300m-Q8_0.gguf --alias embed --port 8082 --embeddings --pooling mean
BROWSE_LLAMA_CHAT_URL=http://127.0.0.1:8081 BROWSE_LLAMA_EMBED_URL=http://127.0.0.1:8082 \
  cargo run -p browse-desktop -- models
# same variables enable the live gateway tests:
BROWSE_LLAMA_CHAT_URL=http://127.0.0.1:8081 BROWSE_LLAMA_EMBED_URL=http://127.0.0.1:8082 \
  cargo test -p model-gateway --test live_llama -- --nocapture
```

Alternatively set `BROWSE_LLAMA_SERVER=/path/to/llama-server` and
`BROWSE_MODELS_DIR=/path/to/ggufs` and the binary spawns its own sidecars. An
OpenAI-compatible cloud endpoint is optional (`BROWSE_CLOUD_BASE_URL`,
`BROWSE_CLOUD_API_KEY`, `BROWSE_CLOUD_MODEL`) and is only ever used for
`Public` data, or `Personal` data with a per-request opt-in; `BROWSE_OFFLINE=1`
disables it entirely.

The demo drives the agent runtime with a scripted "planner" that behaves like a
prompt-injected model on a shop page and prints what the deterministic layers
did about it:

```
step  1: ACTED     click → ok=true
step  2: REJECTED  navigate by user               # out-of-scope origin → confirmation, auto-rejected
step  3: DENIED    type [args.free_text_not_allowed] …  # model-authored text into a field → deny, final
step  4: ACTED     type → ok=true                 # same field, text referenced as `$user` → allowed
step  5: REJECTED  click by user                  # "Place order" → consequential → confirmation
```

Every branch is written to the append-only journal in SQLite
(`agent_sessions`, `agent_steps`, `agent_actions`, `approvals`, `policy_decisions`).

## Security model in one paragraph

Page content is data with provenance, never instructions. The agent runs in an
isolated profile with one-origin session import by explicit grant. Tools take
references (`ref`, `$user`, `$obs:<id>`, `$result:<id>`) rather than free text,
so every argument carries the origin it was read from and PolicyEngine can
enforce a same-origin policy for the agent: cross-origin data flow → user
confirmation; model-authored free text → deny; payments without a money limit →
deny; irreversible actions (submit, pay, delete, publish — multilingual lexicon
+ WebMCP hints) → confirmation, with dry-run stopping before them. A blind
critic that never sees page content can only tighten a verdict. Nothing here
depends on a model behaving.

## License

Apache-2.0.
