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
| S3 Model Gateway: HTTP-транспорт, стриминг, кэш, облачный провайдер, lifecycle llama-server | не начато | — | — |
| S4 Page Intelligence: извлечение, модель страницы, суммаризация / Q&A / перевод | не начато | — | — |
| S5 Memory: индексация с эмбеддингами, семантический поиск, управление памятью | не начато | — | — |
| S6 Agent Runtime: сценарии (≥10), подтверждения, защита от инъекций | не начато | — | — |
| S7 Оболочка: окна, вкладки, омнибокс, закладки, история, загрузки | не начато | — | — |
| S8 Вкладки по задачам, безопасность (фишинг, тёмные паттерны, трекеры) | не начато | — | — |
| S9 Полировка: хоткеи, темы, настройки ИИ, онбординг | не начато | — | — |

Итог сборки на момент записи: `cargo test --workspace --all-features` — **77 тестов, 0 падений**
(agent-runtime 15, core-types 6, engine-adapter 1, engine-cdp E2E 7, fixtures 5, memory 9,
model-gateway 14, page-intelligence 6, policy 14); `cargo clippy --workspace --all-targets
--all-features` без предупреждений; `npm test` в `sensor/` — smoke-тест бандла.

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

## Следующие шаги

S3 (Model Gateway): реальный HTTP-транспорт (`reqwest`), SSE-стриминг, кэш ответов в SQLite,
OpenAI-совместимый облачный провайдер по ключу, менеджер жизненного цикла `llama-server`;
проверка на локальном llama.cpp (Qwen2.5-1.5B чат + EmbeddingGemma-300M эмбеддинги — модели,
которые помещаются в среду без GPU; пресет для RTX 4060 из ADR-003 остаётся целевым).
