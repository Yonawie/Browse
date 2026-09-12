# ADR-002 — Язык и стек: ядро на Rust, Shell на TypeScript (десктоп) и SwiftUI (iOS)

Статус: принято (2026-09).

## Контекст

Нужен один код для логики, которая не зависит от движка и платформы: Page Intelligence (после
сенсора), Memory, Agent Runtime, Model Gateway, Policy. Он должен работать на Windows (рядом с
CEF) и на iOS (внутри приложения SwiftUI), общаться с локальными рантаймами инференса и SQLite,
быть безопасным по памяти (это код, который парсит недоверенный контент страниц) и быстрым в
простое (бюджет < 5% CPU).

## Варианты

| Вариант | За | Против |
|---|---|---|
| C++ (как CEF/Chromium) | Нативная интеграция с CEF | Небезопасная работа с недоверенным вводом; медленные итерации; на iOS живёт, но неудобен |
| TypeScript/Node (Electron-путь) | Скорость разработки UI | Node в привилегированном процессе рядом с агентом; память; на iOS нет |
| Rust | Безопасность памяти, производительность, отличная экосистема для SQLite/HTTP/serde, llama.cpp-биндинги, UniFFI для Swift, `cef-rs`/`wry` для движков, Servo на Rust (будущее) | Порог входа; биндинги к CEF менее зрелые, чем C++ |
| Swift везде | Идеален для iOS | На Windows неприменим |

## Решение

- **Ядро — Rust** (workspace `crates/*`): `core-types`, `engine-adapter`, `page-intelligence`,
  `memory`, `agent-runtime`, `model-gateway`, `policy`. Async на `tokio`. Ошибки — `thiserror`.
  Сериализация — `serde`. БД — `rusqlite` (bundled SQLite + расширение `sqlite-vec`).
- **Десктоп Shell** — TypeScript в привилегированном WebUI-контексте CEF (схема `browser://`),
  общается с ядром по типизированному IPC (JSON-RPC поверх CEF process messages); UI-фреймворк
  без тяжёлого рантайма (Solid/Svelte-класс). Веб-контент пользователя никогда не живёт в
  одном renderer-процессе с WebUI.
- **iOS Shell** — SwiftUI + WKWebView; ядро подключается как XCFramework через **UniFFI**
  (Rust ↔ Swift). Реализация `EngineAdapter` для WKWebView живёт в Swift и передаётся в ядро как
  callback-интерфейс.
- **Сенсор страницы** — TypeScript, компилируется в один изолированный JS-бандл, инжектируется в
  изолированный мир (CDP `Page.addScriptToEvaluateOnNewDocument` с `worldName`; на WKWebView —
  `WKContentWorld`). Общий код для обоих движков.
- Лицензия нашего кода: Apache-2.0. Зависимости — только OSI-лицензии (проверка `cargo deny`).

## Последствия

- Один набор доменных типов (`core-types`) с `serde` — источник правды для IPC, UniFFI и схемы БД.
- Тесты ядра запускаются без движка (через `cdp-remote` бэкенд и фикстуры снимков страниц).
- Цена: два UI-стека (TS и SwiftUI). Принимаем: UI на iOS всё равно должен быть нативным.
