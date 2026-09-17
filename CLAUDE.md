# CLAUDE.md

E-commerce MCP: lokalna aplikacja desktopowa (Tauri 2 + Rust + React/TS), która udostępnia agentom AI konta e-commerce
użytkownika (BaseLinker, Allegro) jako serwer MCP po stdio. **Jedno binarium**: bez argumentów → GUI, `ecommerce-mcp mcp` → serwer MCP
uruchamiany przez klienta AI jako osobny proces. Szczegóły i uzasadnienia: [docs/architektura.md](docs/architektura.md).

## Komendy

```bash
pnpm install                 # zależności frontendu + Tauri CLI
pnpm tauri dev               # aplikacja z hot-reloadem
pnpm lint && pnpm test       # tsc + Vitest (UI na udawanym backendzie)
pnpm build                   # frontend → dist/ (WYMAGANE przed jakimkolwiek `cargo` — generate_context! czyta dist/)
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
CI=true pnpm tauri build     # .app + .dmg (CI=true: bez AppleScriptu Findera, który tu nie ma uprawnień)
pnpm dev                     # podgląd GUI w przeglądarce: http://localhost:1420/src/test/preview.html
```

Przed zakończeniem pracy przechodzą wszystkie: `fmt`, `clippy -D warnings`, `cargo test`, `pnpm lint`, `pnpm test`.

## Mapa kodu

| Gdzie | Co |
|---|---|
| `src-tauri/src/main.rs` | wybór trybu: GUI albo `mcp` |
| `src-tauri/src/app/` | `mod.rs` komendy Tauri (cienkie) · `service.rs` cała logika GUI, testowalna bez Tauri · `selfcheck.rs` klient MCP po stdio (self-check + E2E) · `plugin.rs` generator zipa pluginu Claude |
| `src-tauri/src/integrations/` | `mod.rs` trait `Provider`, `Registry`, `ToolError`, walidatory argumentów, `pick` · `http.rs` wspólne zasady HTTP · `baselinker/` · `allegro/` (`auth.rs` = OAuth Device Flow + odświeżanie) |
| `src-tauri/src/mcp/` | serwer `rmcp`; narzędzia `<provider>__<source_id>__<tool>` budowane dynamicznie z konfiguracji |
| `src-tauri/src/config/` | `config.json` bez sekretów: wersja schematu, migracje, atomowy zapis |
| `src-tauri/src/secrets/` | `SecretStore`: `KeyringStore` (Keychain / Credential Manager) i `MemoryStore` (testy) |
| `src-tauri/src/diagnostics/` | redakcja sekretów, log na stderr, raport diagnostyczny |
| `src-tauri/plugin/` | skille pluginu Claude (`SKILL.md`) — wkompilowane przez `include_str!` |
| `src/` | GUI; `api.ts` = jedyny styk z Rustem; `i18n/pl.ts` = wszystkie teksty; `test/fakeBackend.ts` = udawany backend |

## Zasady niełamalne

- **Sekrety tylko w systemowym credential store.** Nigdy w `config.json`, logach, raporcie, komunikatach błędów ani w odpowiedziach
  dla GUI (typy zwracane do webview są bezsekretne z konstrukcji). Brak dostępu do store = błąd; **żadnego fallbacku do pliku**.
- Klucze w `Source.settings` nie mogą zawierać słów `secret` ani `token` (pilnuje tego test konfiguracji).
- Każdą wartość sekretną opakuj w `Secret` — rejestruje ją w redaktorze. Błędy twórz przez `ToolError::new` / `CommandError::new` (redagują treść).
- Jeden wpis credential store ≤ 2560 bajtów (limit Windows; `MemoryStore` go wymusza) → długie sekrety jako osobne wpisy, zapis przez surowe bajty.
- **Stałe hosty API.** Żaden argument narzędzia ani ustawienie nie może wskazać adresu (SSRF); co najwyżej wybór z zamkniętej listy
  stałych (np. Allegro produkcja/sandbox przez `FieldSpec::options`). Identyfikatory wstawiane do ścieżek waliduj ściśle.
- **Żadnego uniwersalnego „wykonaj zapytanie API”.** Każde narzędzie ma wąski cel, schemat z `additionalProperties: false` i walidację w Ruście **przed** siecią.
- Do modelu trafia whitelist pól (`pick`), nie surowa odpowiedź upstream.
- **Zapisy nigdy nie są ponawiane automatycznie**; ponowienia tylko dla odczytów i błędów przejściowych (`http::with_retries`). Limit API → `RATE_LIMITED` bez ponowień.
- `config.json` zapisuje **wyłącznie proces GUI** (pod blokadą w `Service`); proces MCP tylko czyta, a do credential store pisze jedynie odświeżone tokeny OAuth.
- W trybie `mcp` stdout należy do protokołu: żadnych `println!`. Logi tylko przez `diagnostics::log` (stderr, po redakcji).
- Override'y przez zmienne środowiskowe wyłącznie pod `#[cfg(debug_assertions)]`.
- Serwer MCP nie otwiera portów. Aplikacja nie edytuje plików konfiguracyjnych klientów AI — generuje plugin / fragment do skopiowania.
- Refresh token Allegro jest jednorazowy: odświeżanie tylko przez `Allegro::refresh` (blokada plikowa między procesami, zapis refresh → access).

## Jak dodać

- **Provider:** moduł w `integrations/` z `impl Provider` (+ dla OAuth: `AuthKind::OauthDevice`, `token_kinds`, `begin_authorization`,
  `poll_authorization`), jedna linia w `Registry::default()`, teksty w `pl.ts` (`cap.*`, `provider.<id>.desc|help`, `form.field.<klucz>`,
  opcjonalnie `errors.<KOD>.<id>`, `auth.<id>.*`). Rdzeń MCP i GUI nie wymagają zmian; test kontraktu sprawdzi go automatycznie.
- **Narzędzie MCP:** `ToolDef` w `tools()` + gałąź w `call_tool` + wpis w `capabilities` + test na wiremock + aktualizacja skilla w
  `src-tauri/plugin/skills/` (test pluginu wyłapie odwołania do nieistniejących narzędzi). Nazwa krótka — test kontraktu pilnuje limitu
  64 znaków dla pełnej nazwy `<provider>__<source_id>__<tool>`.
- **Komenda Tauri:** metoda w `Service` (z testem) → cienki wrapper w `app/mod.rs` + `generate_handler!` → `src/api.ts` → `src/test/fakeBackend.ts`.
- **Pole formularza źródła:** `FieldSpec::new(...)` w `meta()` providera (`.options(&[..])` = lista wyboru, `.keeps_auth()` = zmiana nie
  zrywa połączenia) + `validate_field` + teksty `form.field.<klucz>`, opcjonalnie `form.hint.<klucz>` i `form.option.<klucz>.<wartość>`.
  Formularz w GUI renderuje się sam z metadanych. Wartości nagłówków HTTP waliduj do drukowalnego ASCII (wstrzyknięcie nagłówków).
- **Tekst w GUI:** wyłącznie przez `t()` / `tDynamic()` / `tOptional()` i `src/i18n/pl.ts`.

## Testy

- HTTP mockuj `wiremock`, sekrety `MemoryStore`; żadnej prawdziwej sieci ani Keychaina (wyjątek: test `#[ignore]` `real_credential_store`).
- Bez prawdziwych `sleep`: interwały, backoff i krok `slow_down` są wstrzykiwane. Nie używaj `tokio::time::pause()` — z prawdziwymi gniazdami psuje timeouty reqwest.
- Każda nowa logika zostawia test; dla ścieżek bezpieczeństwa asercja, że sekret nie pojawia się w wyniku, błędzie ani pliku.
- Zmiany w GUI sprawdź wizualnie w podglądzie (`src/test/preview.html`) — natywnego okna nie da się obejrzeć bez zrzutu ekranu użytkownika.

## Konwencje

- Dokumentacja, komentarze w kodzie i GUI po polsku. Opisy narzędzi MCP i komunikaty błędów z Rusta po angielsku (czyta je model);
  GUI tłumaczy błędy po kodzie: `errors.<KOD>.<provider>` → `errors.<KOD>` → `errors.UNKNOWN`.
- Minimalizm: najpierw stdlib i to, co już jest w repo; bez spekulatywnych abstrakcji i konfiguracji „na zapas”; martwy kod usuwamy.
  Świadome uproszczenie ze znanym sufitem oznacz komentarzem `ponytail:` (co jest sufitem i jaka jest ścieżka rozbudowy).
- Formatowanie Rusta wg `src-tauri/rustfmt.toml` (szerokość 160).

## Pułapki

- Przebudowa niepodpisanej aplikacji zmienia tożsamość binarium → macOS jednorazowo pyta o dostęp do pęku kluczy („Zawsze pozwalaj”).
- Działająca instancja aplikacji nie podmienia się po buildzie — trzeba ją zamknąć i uruchomić ponownie.
- Plugin Claude i ręczna konfiguracja zawierają bezwzględną ścieżkę binarium — po przeniesieniu aplikacji trzeba je wygenerować ponownie.
- `samples/` to cudze repozytoria (ignorowane przez git) — tylko inspiracja; generyczne `baselinker_call` stamtąd jest sprzeczne z zasadami wyżej.
- Wersję podbijaj w trzech plikach: `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`.
- CI patrzy na gałąź `main`. Windows weryfikuje właściciel projektu i CI — lokalnie budujemy tylko macOS.
- Commit i push wyłącznie na wyraźną prośbę użytkownika.
