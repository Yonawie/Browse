# Browse

An AI-native browser: the model is part of the core (page understanding,
memory, an agent that acts on pages), not a chat button bolted onto Chrome.
Windows + iOS, local-first inference on ~8 GB RAM / RTX 4060-class hardware,
open-source licenses only, **zero telemetry**.

This repository contains the engineering documents and the increment-I0 code:
domain types, the deterministic policy engine, the SQLite memory schema and
store, the model gateway (routing + VRAM budget), the agent runtime, an
engine-adapter trait with a mock backend, the page sensor, and a headless
desktop binary that runs the whole pipeline end-to-end.

## Documents

| Document | Contents |
|---|---|
| [`docs/01-research.md`](docs/01-research.md) | Phase 1: market landscape (Arc/Dia, Comet, Brave Leo, Opera Neon, Chrome Gemini, Edge Copilot, Vivaldi, SigmaOS, Zen, Operator, Browser Use, Playwright agents), capability map by layer, technical constraints and risks, engine comparison and choice, AI stack, ADR summary |
| [`docs/02-architecture.md`](docs/02-architecture.md) | Phase 2: layers, agent security model, privacy model, performance budgets, extensibility, data schema, repo layout, increment roadmap |
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
  model-gateway/           ModelProvider, sensitivity-aware Router, llama-server adapter, VRAM ModelManager
  agent-runtime/           observe → plan → label refs → policy → blind critic → confirm/dry-run → act → journal
apps/
  desktop/                 browse-desktop binary (mock engine in I0; `cef` feature reserved), SQLite journal bridge
  ios/                     SwiftUI + WKWebView + UniFFI notes
sensor/                    TypeScript page sensor for the isolated world (shared by CEF and WKWebView)
schema/memory.sql          SQLite DDL (WAL, FTS5, vec0, append-only triggers)
schemas/                   JSON Schema: tool-manifest, task-scope, policy
```

## Build and test

Requires Rust stable ≥ 1.88 (`rust-version` in `Cargo.toml`) and, for the sensor, Node ≥ 20.

```sh
cargo test --workspace                       # 65 unit/integration tests
cargo run -p browse-desktop -- schema-check  # apply DDL to an in-memory DB
cargo run -p browse-desktop -- demo          # headless red-team demo (see below)
cargo run -p browse-desktop -- demo --dry-run
cd sensor && npm install && npm run typecheck
```

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
