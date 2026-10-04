# Статус реализации (Фаза 3–4)

Живой документ: обновляется после каждого этапа. Для каждого этапа — что сделано,
что не сделано и почему, известные ограничения, результаты сборки/тестов и
измерения бюджетов (§4 в [`02-architecture.md`](02-architecture.md)).

Среда измерений на этом этапе: Linux VM, 4 vCPU, 15 ГБ RAM, без GPU, Google
Chrome 148 (headless), Rust stable 1.98. Референсная машина (Windows 11, RTX 4060)
пока недоступна — цифры ниже это нижняя граница, не p95 на целевом железе.

## Сводка по этапам

| Этап | Состояние | Коммиты | Тесты |
|---|---|---|---|
| S1 Скелет: workspace, CI (Windows/Linux/macOS + сенсор), rustfmt/clippy, README | готово | `f19462d`, `7f7166c` | CI: fmt, clippy `-D warnings`, build, test |
| S2 Движок: `engine-cdp` (Chromium через CDP), фикстурные сайты, hostile-page E2E | готово | `b559628`…`ca9b5e0` | 7 E2E + 5 фикстур |
| S3 Model Gateway: HTTP-транспорт, SSE-стриминг, кэш, облачный провайдер по ключу, lifecycle llama-server | готово | см. git log `feat(model-gateway)` | 34 unit + 3 live (llama.cpp) + 1 memory |
| S4 Page Intelligence: извлечение, модель страницы, суммаризация / Q&A / перевод, верификация цитат | готово | `a1adfe8` | 10 unit + fixture/Chromium smoke |
| S5 Memory: индексация с эмбеддингами, семантический поиск, управление памятью | готово | `358b364` | 10 unit + CLI (stats, index, search, forget) |
| S6 Agent Runtime: сценарии (≥10), подтверждения, защита от инъекций | готово | `02680b7` | 25 тестов (15 unit + 10 redteam E2E) + CLI `run` |
| S7 Оболочка: окна, вкладки, омнибокс, WebUI Shell, JSON-RPC & SSE IPC | готово | `6c283c2` | 18 unit (desktop) + embedded asset tests |
| S8 Вкладки по задачам, безопасность (фишинг, тёмные паттерны, zero-telemetry) | готово | см. git log | 20 unit (page-intelligence) + 11 (memory) + 20 (desktop) |
| S9 Полировка: хоткеи, темы, настройки ИИ, онбординг | готово | см. git log | хоткеи (Ctrl+T/W/L/B/1..9), темы, мастер онбординга |
| MCP Extensibility: Model Context Protocol Server (ADR-008) для Claude Desktop / Cursor | готово | см. git log | 19 unit (agent-runtime) + 20 (desktop) + stdio JSON-RPC |

Итог сборки на момент записи: `cargo test --workspace --all-features` — **144 теста, 0 падений**
(agent-runtime 29 [19 unit + 10 redteam benchmark], core-types 6, engine-adapter 1, engine-cdp E2E 8, fixtures 5, memory 11,
model-gateway 34 + 3 live, page-intelligence 20, policy 14, desktop 20); `cargo clippy --workspace --all-targets
--all-features -- -D warnings` без единого предупреждения.

## S1 — Скелет

Сделано:

- Rust workspace (`rust-version = 1.88`), `rustfmt.toml` (120 колонок), `clippy.toml`.
- GitHub Actions: матрица `windows-latest` / `ubuntu-latest` / `macos-latest` — fmt-check,
  clippy с `RUSTFLAGS=-D warnings`, build всех таргетов, тесты (включая E2E движка на
  предустановленном Chrome раннера); отдельный job для сенсора (typecheck, сборка, smoke-тест,
  проверка, что закоммиченный `sensor/dist` совпадает с исходниками).
- README: запуск за 5 минут (см. корневой `README.md`).

Не сделано / ограничения:

- CI на Windows пока не гоняет E2E движка в «сандбоксе»: Chrome на раннере есть, тесты
  запускаются, но результат на Windows не подтверждён локально (нет Windows-машины в среде).
- Прогонов CI ещё не было — первый пройдёт с PR. Если матрица упадёт, правки войдут в S3.

## S2 — Движок: `engine-cdp` и фикстурные сайты

Решение по движку на Фазу 3 (не отступление от ADR-001, а порядок реализации): первым
реализован бэкенд `cdp-remote` — реальный Chromium, управляемый по DevTools Protocol.
Это тот же протокол, который отдают CEF (`SendDevToolsMessage`) и WebView2
(`CallDevToolsProtocolMethod`), поэтому код `engine-cdp` — референс для оболочек S7 и
единственный бэкенд, который можно полноценно тестировать headless в CI на трёх ОС.

Сделано:

- `crates/engine-cdp`: `Connection` (WebSocket, корреляция ответов по `id`, очереди событий
  на сессию с монотонными курсорами), `launcher` (поиск Chrome/Chromium/Edge, `--headless=new`,
  временный профиль, парсинг `DevTools listening on`), `CdpEngine: EngineAdapter`.
- Сенсор внедряется в **изолированный мир** `browse-sensor`
  (`Page.addScriptToEvaluateOnNewDocument{worldName}` + `Page.createIsolatedWorld`); страница
  не видит `BrowseSensor` (проверяется тестом: `typeof BrowseSensor === "undefined"` в main world).
- Все действия по `ref`: `BrowseSensor.resolveRef(id)` → `objectId` → `DOM.getBoxModel` →
  доверенные события `Input.dispatchMouseEvent` / `Input.insertText` / `Input.dispatchKeyEvent`.
  Никаких селекторов от модели, никакого `element.click()`.
- Отказ печатать в маскированные поля (`type=password`, `autocomplete=cc-*`,
  `one-time-code`, `*-password`) — `EngineError::Blocked` на уровне движка, независимо от policy.
- Профили: `User` — контекст по умолчанию; `Agent{session_id}` и `Private` — отдельные
  `Target.createBrowserContext` (свои cookie, storage, кэш). `import_session` копирует cookie
  только в агентский контекст и только для одного origin; `export_session` читает cookie
  пользователя (включая `HttpOnly`) через `Storage.getCookies`.
- `observe`: снимок сенсора → `Observation`; `has_session` дополняется фактом наличия
  `HttpOnly` cookie для origin в jar'е контекста; чувствительность — детерминированный
  классификатор из `policy` (ADR-007). Эвристика human-challenge → `EngineEvent::HumanChallenge`.
- Ожидания: навигация ждёт `load` конкретного `loaderId`, затем «тихое окно» сети 150 мс
  (кап 800 мс); восстановление из bfcache распознаётся по `frameNavigated{BackForwardCacheRestore}`.
- `crates/fixtures`: один axum-сервер, виртуальные хосты `shop`, `shop2`, `forms`, `news`,
  `login`, `hostile`, `evil` на `*.localhost:PORT` — разные origin без DNS и TLS; фиксирует
  заказы, отправки форм и всё, что дошло до `evil`. Запуск вручную: `cargo run -p fixtures`.
- `policy`: в dev-режиме перечисленные loopback-origin допускаются по `http://`
  (иначе фикстуры невозможно было бы использовать как scope агента).
- Сенсор: сборка esbuild в IIFE (`sensor/dist/sensor.iife.js`, закоммичен, встраивается через
  `include_str!`), `readMore(obsId)`, значения `page_kind` синхронизированы с `core_types::PageKind`,
  расширены эвристики инъекций (семейство «note to AI agents…», зеркально в
  `page_intelligence::injection`).

E2E-тесты (`crates/engine-cdp/tests/e2e.rs`, один браузер на всю сессию, самопропуск без
Chrome):

| Тест | Что доказывает |
|---|---|
| `observe_catalog_page` | заголовки → `heading_path`, ссылки с `href`, скриншот PNG, AX-дерево |
| `type_submit_and_navigate_history` | ввод + Enter → навигация с `?q=`; Back/Forward; события `Loaded`/`Request` |
| `fill_contact_form_with_trusted_input` | textbox/combobox/checkbox/textarea/submit; данные дошли до сервера без искажений |
| `masked_fields_are_never_typed_into` | пароль и `cc-number` → `Blocked`; `page_kind = checkout`; `consequential_hint` у «Place order» |
| `agent_profile_is_isolated_and_session_import_is_scoped` | агент не наследует сессию; импорт cookie только своего origin (лишний домен отброшен); импорт в User/Private → `Blocked` |
| `hostile_page_hidden_instructions_are_quarantined` | скрытый текст не попадает в `content`, а в `hidden_text_signals`; видимая инструкция помечена `suspect_injection`; при наблюдении к `evil` не ушло ни одного запроса; изолированный мир не виден странице |
| `read_more_returns_full_chunk_and_unknown_ids_are_null` | `read_more`, `NoSuchElement`, `NoSuchWebView` |

Не сделано / ограничения:

- `Action::CallSiteTool` (WebMCP) — бэкенд возвращает `Backend("not supported")`: в Chromium
  нет стабильного API `navigator.modelContext`; декларативные формы-инструменты сенсор уже
  описывает, вызов появится вместе с S6.
- `observe_iframe_origins` пока не используется: сенсор работает только в главном фрейме.
- `looks_like_human_challenge` — текстовая эвристика; событие информационное.
- Chromium запускается с `--no-sandbox` только при `CI`, `BROWSE_CHROME_NO_SANDBOX` или root.

Измерения (локальные фикстуры, headless, без GPU; средние по 4 страницам):

| Метрика | Бюджет §4 | Измерено | Комментарий |
|---|---|---|---|
| Снимок страницы сенсором (`observe`) | p95 < 50 мс | 7–11 мс | страницы фикстур маленькие (80–250 токенов); замер на реальных сайтах — в S4 |
| Навигация до готовности к наблюдению | — | ~170 мс | `load` + тихое окно 150 мс; было ~820 мс до замены ожидания `networkIdle` |
| Действие без навигации (`type`) | — | ~8 мс | |
| Действие с навигацией (`click` по ссылке) | — | ~175 мс | |
| Back из bfcache | — | ~6 мс | |
| AX-дерево (`Accessibility.getFullAXTree`) | — | ~6 мс | |
| Скриншот PNG | — | ~45 мс | |
| Запуск Chromium + подключение | часть «холодного старта < 2 с» | ~130 мс | без учёта оболочки |
| Размер наблюдения | ≤ 4K токенов на шаг агента | 79–254 токенов | |

## S3 — Model Gateway

Сделано (`crates/model-gateway`):

- `OpenAiCompatProvider` — один провайдер для локального `llama-server` (`Locality::Local`) и
  облачных OpenAI-совместимых эндпоинтов (`::cloud(name, transport, models)` → `Locality::Cloud`).
  `LlamaServerProvider` оставлен как алиас. Инструменты, `response_format: json_schema`
  (на llama.cpp → GBNF-грамматика, вывод гарантированно валиден), `tool_calls` из потока
  собираются по `index`.
- Стриминг: `ModelProvider::chat_stream` → `ChatStream` из `Delta(String)` и ровно одного
  `Done(ModelResponse)` с usage (`stream_options.include_usage`). Провайдеры без стриминга
  получают fallback по умолчанию.
- `HttpTransport` (`reqwest`, rustls, без default features): JSON POST, SSE-парсер с учётом
  частичных чанков и `\r\n`, bearer-ключ (не печатается в `Debug`), connect-timeout 5 с для
  мёртвого sidecar'а. `RoutingTransport` — диспетчер по алиасу модели на процесс.
- `Gateway` — фасад для остального браузера: маршрутизация по чувствительности (ADR-007),
  кэш (`MemoryCache` LRU; ключ blake3 от provider+model+messages+tools+schema+max_tokens+temperature;
  кэшируются только детерминированные запросы без инструментов, t ≤ 0.1), кэш эмбеддингов
  по тексту (в модель уходят только промахи), лимит параллелизма (`Semaphore`), тайм-аут,
  журнал `model_calls` через трейт `CallLog` (в приложении — `SqliteCallLog` →
  `MemoryStore::record_model_call`; агрегат `model_usage()` для будущего UI настроек).
- `sidecar`: `LlamaSidecar::start(SidecarConfig)` — процесс на `127.0.0.1:<свободный порт>`,
  `--log-disable` (промпты не попадают в логи), `--jinja`, ожидание `/health == ok`, хвост stderr
  в сообщении об ошибке, `kill_on_drop`; `SidecarPool::apply(LoadPlan)` исполняет планы
  `ModelManager` (unload → load), `reap()` убирает упавшие процессы.
- `browse-desktop models`: конфигурация из окружения (`BROWSE_LLAMA_CHAT_URL`,
  `BROWSE_LLAMA_EMBED_URL`, `BROWSE_LLAMA_SERVER`+`BROWSE_MODELS_DIR` для спауна,
  `BROWSE_CLOUD_BASE_URL/API_KEY/MODEL`, `BROWSE_OFFLINE`), таблица маршрутизации по классам,
  стриминговый smoke-промпт, строки `model_calls`.

Проверка на реальном llama.cpp (b10933, CPU, 4 vCPU; `tests/live_llama.rs`, включается
переменными окружения, в CI пропускается):

| Метрика | Бюджет §4 | Измерено | Комментарий |
|---|---|---|---|
| Первый токен стриминга (Qwen2.5-1.5B Q4_K_M, 28 токенов промпта) | — | 85 мс | CPU; на RTX 4060 с Qwen3-4B ожидаемо ниже |
| Полный ответ из 2 токенов (без стриминга) | — | 276 мс | |
| Structured output по JSON-schema | 100 % валидный JSON | `{"city":"Paris","country":"France"}` | грамматика llama.cpp |
| Эмбеддинги 3 текстов (EmbeddingGemma-300M Q8, 768-dim) | — | 72 мс | cos(похожие)=0.76, cos(разные)=0.45 |
| Повторный запрос из кэша | — | < 1 мс, модель не вызывается | и для `chat`, и для `chat_stream` |
| Холодный старт sidecar (1.5B, ctx 1024, CPU) | «первый ответ < 4 с» | 2.0 с до `/health == ok` | Qwen3-4B на NVMe+GPU по ADR-003 ~3 с |

Не сделано / ограничения:

- Кэш ответов персистентен только на уровне трейта: `MemoryCache` живёт в процессе. Постоянный
  кэш в SQLite появится вместе с индексацией в S5, когда будет понятно, как он инвалидируется
  при «забыть страницу».
- Провайдеры Anthropic/Gemini с собственными протоколами не реализованы — OpenAI-совместимые
  эндпоинты (включая OpenRouter и прокси) покрывают облачный сценарий; учёт `cost_usd` пока `NULL`.
- Ключ облака берётся из переменной окружения; хранение в системном keychain — в S9 вместе с
  настройками.
- `llama-server` в режиме роутера моделей (`/models/load`) не используется: один процесс на модель
  проще убивать при превышении бюджета VRAM и он изолирует падения.

## Следующие шаги

S4 (Page Intelligence) завершён: CLI-срез `browse-desktop page <url> summarize|ask|translate` соединяет реальный CDP-движок,
локально ограниченное наблюдение и `Gateway` со стримингом, кэшем и автоматической верификацией цитат (`[c0]`, обнаружение галлюцинаций).

S5 (Memory) завершён: индексация страниц в постоянную БД SQLite, пакетное сохранение чанков и векторов, гибридный поиск (BM25 FTS5 + векторный поиск с RRF), CLI-команды `browse-desktop memory stats/index/search/forget`.

S7 (Оболочка / Shell) завершён (`6c283c2`): WebUI Shell (`apps/desktop/ui/`), встроенные ассеты, JSON-RPC 2.0 сервер по `/api/rpc`, SSE поток событий по `/api/events`, омнибокс с распознаванием URL/поиска, сайдбар ИИ и модальные карты подтверждения.

S8 (Безопасность и вкладки по задачам) завершён (`6f3c82e`): детектор фишинга и IDN-омоглифов (`phishing.rs`), анализатор тёмных паттернов FTC/EU DSA (`dark_patterns.rs`), группировка вкладок по задачам и категориям в SQLite (`task_groups`, `auto_cluster_tab`), shield-бейдж безопасности в UI и CLI `browse-desktop inspect-safety`.

S9 (Полировка) завершён: глобальные сочетания клавиш (`Ctrl+T/W/L/B/Tab/1..9/Esc`), переключение тёмной/светлой темы с сохранением в `localStorage`, стартовый мастер онбординга с объяснением гарантий приватности и локального инференса.

MCP Extensibility (ADR-008) завершён: полнофункциональный сервер Model Context Protocol (`browse-mcp`) в `crates/agent-runtime/src/mcp.rs` и CLI-команда `browse-desktop mcp [--dev-mode] [--auth-token <t>] [--origin <o>]` в `apps/desktop/src/mcp_cmd.rs`. Предоставляет внешним агентам (Claude Desktop, Cursor) стандартные инструменты `browse_navigate`, `browse_observe`, `browse_click`, `browse_type`, `browse_extract` со строгим контролем `PolicyEngine` (TaskScope, защита от инъекций, запрет действий вне разрешённых origins, валидация auth токена).

Все фазы архитектурного плана (S1–S9 + MCP) полностью реализованы, покрыты 144 тестами (0 падений) и проверены `clippy -D warnings`.

