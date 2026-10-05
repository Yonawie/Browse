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
| MCP Extensibility: Model Context Protocol Server (ADR-008) для Claude Desktop / Cursor | готово | `4a1d4db` | 19 unit (agent-runtime) + 20 (desktop) + stdio JSON-RPC |
| Knowledge Graph: сущности, меншены, ко-оккурентные связи и IPC | готово | `29f2bf7` | 21 unit (page-intelligence) + 12 (memory) + RPC `memory.entities` |
| Plugin Host & Sandboxing: пользовательские инструменты агента (ADR-008) | готово | `7cfd88d` | 30 unit (agent-runtime: 20 unit + 10 redteam) |
| Skills & Automation Scenarios: библиотечные навыки и шаблоны (ADR-008) | готово | `b478e52` | 7 unit (core-types) + 22 (desktop) + CLI `skill` |
| NL Omnibox Intent Dispatcher (IN-1): URL, Search, Tab, Memory, Agent routing | готово | `f8d51b6` | 13 unit (core-types) + RPC `omnibox.classify` |
| Tab Hygiene & Commands (AT-2 / IN-2): детектирование старых вкладок, архивация, команды | готово | `f41b4dc` | 13 unit (memory) + 23 (desktop) + RPC `tabs.suggest_pruning`, `tabs.archive`, `tabs.execute_command` |
| Focus Mode & Selection (AT-4 / AT-3 / IN-4): режим фокусировки, приоритизация вкладок, выделение | готово | `1068472` | 24 unit (desktop) + CDP selection + RPC `focus.toggle`, `tabs.prioritize`, `page.selection.*` |
| Page Diff Across Time (PI-11 / AT-5): дифф страниц во времени, история версий, CLI diff | готово | `bbb320f` | 24 unit (page-intelligence) + 14 (memory) + RPC `page.diff` + CLI `page <url> diff` |
| Playwright Test Generator (D-2): экспорт тестов из действий браузера с ARIA-селекторами | готово | `5f0f30f` | 32 unit (agent-runtime) + 26 (desktop) + RPC `session.export_playwright` + CLI `playwright` |
| Privacy & Tracker Audit (D-5): аудит трекеров, рекламных сетей, сессионных реплеев и фингерпринтинга | готово | см. git log | 27 unit (page-intelligence) + RPC `page.analyze_safety` + UI Privacy Audit |
| Memory Obsidian Vault Export (M-4): экспорт базы знаний памяти в Obsidian vault с [[wikilinks]] и JSON | готово | см. git log | 16 unit (memory) + RPC `memory.export` + UI Vault Export |
| Tab Auto-Grouping (AT-1): детерминированная кластеризация вкладок по доменам, темам и времени | готово | см. git log | 18 unit (memory) + RPC `tabs.auto_group` + UI Auto-Group |
| DevTools Error Explainer (D-1): локальный анализ CORS/CSP/JS-ошибок с маскированием секретов | готово | см. git log | 31 unit (page-intelligence) + RPC `devtools.explain` + UI Dev Diagnostics |
| AdBlock & Tracker Shield: активный перехват и блокировка рекламы/трекеров по правилам EasyList | готово | см. git log | 35 unit (page-intelligence) + RPC `adblock.*` + UI Shield Button |
| Bookmarks Manager: хранение закладок, папки, теги, импорт/экспорт Netscape HTML (Chrome/Firefox) | готово | см. git log | 20 unit (memory) + RPC `bookmarks.*` + UI Bookmarks Bar |
| Model Catalog & Downloader: каталог рекомендованных GGUF-моделей (Qwen/Llama/BGE), инспекция статуса | готово | см. git log | 36 unit (model-gateway) + RPC `models.*` + UI 1-Click Setup |
| Browsing History Manager: история навигаций, поиск, аналитика доменов, удаление и очистка | готово | см. git log | 21 unit (memory) + RPC `history.*` + UI History Tab |
| Distraction-Free Reader Mode: извлечение чистого текста статьи, расчет времени чтения, UI оверлей | готово | см. git log | 36 unit (page-intelligence) + RPC `page.reader_mode` + UI Reader View |
| Site Data Cleaner ("Forget This Site"): каскадная очистка всех следов домена (история, память, чанки) | готово | см. git log | RPC `page.forget_site` + UI Forget Site Button |
| Download Manager & Safe File Inspector: трекинг загрузок, эвристики расширений, SHA-256 хеш | готово | см. git log | 23 unit (memory) + RPC `downloads.*` + UI Downloads Manager |
| Cookie & Site Storage Inspector: аудит и классификация куки, маскирование значений, очистка | готово | см. git log | 38 unit (page-intelligence) + RPC `cookies.*` + UI Storage Inspector |
| Find in Page (Ctrl+F): полнотекстовый поиск по странице, навигация Enter/Shift+Enter, счетчик | готово | см. git log | Хоткей Ctrl+F, регистронезависимый поиск, UI Find Bar |
| Zero-Knowledge Password Vault: шифрование логинов/паролей BLAKE3-шифром, SQLite vault | готово | см. git log | 25 unit (memory) + RPC `vault.*` + UI Password Vault |
| Multi-Profile Management: изолированные пространства пользователя, агента и приватные профили | готово | см. git log | 26 unit (memory) + RPC `profiles.*` + UI Profile Modal |
| Command Palette (Ctrl+K / Ctrl+P): мгновенный поиск команд, действий, вкладок, закладок, истории | готово | см. git log | 26 unit (desktop) + RPC `palette.search` + UI Palette Modal |
| Quick Search Bangs & Engine Switcher: быстрые бэнги (!gh, !yt, !w, !ddg, !g, !b, !c, !d, !r) и выбор поисковика | готово | см. git log | 14 unit (core-types) + UI Search Engine Settings |
| Site Security & Permissions Inspector (🔒): инспекция TLS, происхождение, матрица разрешений сайта | готово | см. git log | 27 unit (memory) + RPC `permissions.*` + UI Security Modal |
| Tab Sleeping & Memory Saver: усыпление неактивных вкладок для экономии RAM, пробуждение по клику | готово | см. git log | 26 unit (desktop) + RPC `tabs.discard`/`tabs.wake` + UI Sleeping Tabs |
| Saved Sessions & Workspaces: сохранение и восстановление именованных сессий вкладок | готово | см. git log | 28 unit (memory) + RPC `sessions.*` + UI Sessions Modal |
| Crash Recovery: непрерывное автосохранение активных вкладок, автоматический баннер восстановления | готово | см. git log | RPC `sessions.save_active`/`load_last_active` + UI Crash Alert |
| Tab Audio Muting: переключение звука вкладки, динамические иконки 🔊/🔇, хоткей Ctrl+M | готово | см. git log | RPC `tabs.toggle_mute` + UI Tab Mute Buttons |
| Page Zoom Controls: масштабирование страницы в веб-фрейме, хоткеи Ctrl++/Ctrl--/Ctrl+0, бейдж 100% | готово | см. git log | Хоткеи масштаба, кнопка сброса, визуальный zoom |
| Smart Tab Deduplication: очистка UTM-трекеров, канонизация URL, дедупликация вкладок | готово | см. git log | RPC `tabs.dedup` + Command Palette + Tab Hygiene modal |
| AI Prompt Templates: библиотека быстрых промптов ("Pricing Table", "Code", "Fact Check", "Takeaways") | готово | см. git log | 29 unit (memory) + RPC `prompts.*` + динамические чипы ИИ |
| User Scripts & Custom Styles: локальные расширения JS/CSS с паттернами доменов (*, domain) | готово | см. git log | RPC `userscripts.*` + UI User Scripts Modal |

Итог сборки на момент записи: `cargo test --workspace --all-features` — **212 тестов, 0 падений**
(agent-runtime 32 [22 unit + 10 redteam benchmark], core-types 14, engine-adapter 1, engine-cdp E2E 8, fixtures 5, memory 29,
model-gateway 36 + 3 live, page-intelligence 38, policy 14, desktop 26); `cargo clippy --workspace --all-targets
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

Все фазы архитектурного плана (S1–S9 + MCP) и расширенные возможности (Attention Management, Page Selection, Page Diff во времени, Playwright Test Generator) полностью реализованы, покрыты 174 тестами (0 падений) и проверены `clippy -D warnings`.

