# E-commerce MCP — architektura i decyzje

Dokument dla osób rozwijających projekt. Przygotowanie maszyny i budowanie paczki: [README](../README.md).
Zasady pracy nad kodem (także dla Claude): [CLAUDE.md](../CLAUDE.md). Wymagania produktowe: [specyfikacja.md](specyfikacja.md).

## Struktura kodu

```text
src/                     React + TypeScript + Tailwind (GUI, po polsku, i18n w src/i18n)
src-tauri/src/
  main.rs                jedno binarium: bez argumentów → GUI, z argumentem `mcp` → serwer MCP (stdio)
  app/                   komendy Tauri (mod.rs), logika GUI bez Tauri (service.rs), self-check MCP (selfcheck.rs),
                         generator pluginu Claude (plugin.rs)
  config/                bezsekretna konfiguracja: wersjonowany JSON, migracje, atomowy zapis
  secrets/               SecretStore: Keychain / Windows Credential Manager (keyring) + fake do testów
  mcp/                   serwer MCP na oficjalnym SDK `rmcp`, dynamiczna lista narzędzi
  integrations/          kontrakt Provider + rejestr; http.rs (wspólne zasady HTTP); baselinker/ (token API),
                         allegro/ (OAuth Device Flow, tylko odczyt)
  diagnostics/           redakcja sekretów, log na stderr, raport diagnostyczny
src-tauri/plugin/        treść pluginu: skille (SKILL.md) i README — wkompilowane w binarium
src-tauri/tests/e2e.rs   E2E: prawdziwe binarium w trybie `mcp` + mockowane API BaseLinkera i Allegro
src/test/                udawany backend Tauri dla testów UI i podglądu w przeglądarce (preview.html)
docs/                    specyfikacja produktu, instrukcja użytkownika, pakowanie
```

### Kluczowe decyzje

**Jedno binarium zamiast sidecara.** Klient AI uruchamia `ecommerce-mcp mcp`; to ten sam plik wykonywalny co GUI.
Dlaczego: (1) przy transporcie stdio to *klient AI* jest rodzicem procesu MCP, więc aplikacja i tak nie „hostuje” serwera —
osobny sidecar nic nie daje; (2) Keychain wiąże wpis z tożsamością binarium, więc GUI (zapis tokenu) i serwer MCP (odczyt)
jako jeden plik nie wywołują systemowych promptów o dostęp; (3) brak skryptów kopiujących sidecar i `externalBin`.
Status „MCP gotowe” w GUI to wynik self-checku: aplikacja uruchamia samą siebie w trybie `mcp`, robi handshake
i `tools/list`, po czym zamyka proces (kontrolowane wyjście po zamknięciu stdin, z twardym limitem 3 s).
Ścieżka awaryjna, gdyby stdio w binarium GUI sprawiało kłopoty na Windows: drugi `[[bin]]` z `mcp::run_stdio()` dołączony przez `bundle.externalBin`.

**Tylko stdio.** Serwer nie otwiera żadnego portu. Transport HTTP jest poza MVP.

**Nazwy narzędzi:** `<provider>__<source_id>__<tool>`, np. `baselinker__glowny_sklep__list_orders`.
`source_id` to slug nadawany przy tworzeniu źródła (`[a-z0-9_]`, bez `__`) — nie zmienia się przy zmianie nazwy.
Zawsze dostępne jest `ecommerce_mcp_list_sources` (mapa nazw źródeł → prefiksy narzędzi, bez sekretów).

**Dynamiczna lista narzędzi.** Konfiguracja jest czytana przy każdym `tools/list` i `tools/call`: wyłączone źródło
znika z listy po odświeżeniu, a jego wywołanie zwraca `SOURCE_DISABLED`. Serwer jest bezstanowy; token jest pobierany
z credential store dopiero wtedy, gdy narzędzie go potrzebuje.

**Konfiguracja:** `<data_local_dir>/com.lampartoms.ecommerce-mcp/config.json` — macOS `~/Library/Application Support/…`,
Windows `%LOCALAPPDATA%\…` (Local, nie Roaming: wpisy Credential Managera też nie roamują). Katalog powstaje przy pierwszym
zapisie. Zapis atomowy (plik tymczasowy + fsync + rename), pole `schema_version` i migracje krok po kroku; plik z nowszej
wersji aplikacji nie jest nadpisywany.

**Sekrety:** wyłącznie systemowy credential store (`keyring`): usługa `com.lampartoms.ecommerce-mcp`,
konto `<provider>/<source_id>/<rodzaj>`. Brak dostępu = błąd i wycofanie źródła; nie ma fallbacku do pliku.
Typ `Secret` ma zredagowany `Debug`, a każda załadowana wartość jest rejestrowana w redaktorze, przez który przechodzą
wszystkie logi, komunikaty błędów i raport diagnostyczny (plus heurystyka na ciągi wyglądające jak tokeny).

**Błędy narzędzi:** `{"error":{"code","message"}}` z kodami `SOURCE_NOT_FOUND`, `SOURCE_DISABLED`, `CREDENTIAL_UNAVAILABLE`,
`AUTH_FAILED`, `RATE_LIMITED`, `UPSTREAM_ERROR`, `VALIDATION_ERROR`, `NOT_FOUND`.

### Narzędzia BaseLinkera

| Narzędzie | Metoda API | Zapis |
|---|---|---|
| `get_order_statuses` | `getOrderStatusList` | |
| `list_orders` (data od/do, status, e-mail, limit ≤ 100, paginacja `next_date_from`) | `getOrders` | |
| `get_order` | `getOrders` (`order_id`) | |
| `list_inventories` | `getInventories` | |
| `list_warehouses` | `getInventoryWarehouses` | |
| `list_products` (katalog, nazwa/SKU/EAN, strona, limit ≤ 1000) | `getInventoryProductsList` | |
| `update_order_status` | `setOrderStatus` | ✔ |
| `add_order_note` (dopisuje do `admin_comments`, limit 200 znaków) | `getOrders` + `setOrderFields` | ✔ |

Zasady klienta HTTP: stały endpoint `https://api.baselinker.com/connector.php` (nie da się go podać z zewnątrz — brak SSRF),
token w nagłówku `X-BLToken`, timeout 30 s, limit odpowiedzi 8 MB, do 2 ponowień z backoffem tylko dla odczytów i tylko
dla błędów przejściowych (timeout, sieć, 5xx). Zapisy i limit API (100 req/min → `RATE_LIMITED`) nie są ponawiane.
Do modelu trafia whitelist pól, nie surowa odpowiedź. Test połączenia używa `getOrderStatusList`.
Dokumentacja API nie publikuje pełnej listy kodów błędów, więc mapowanie działa po fragmentach nazw (`*TOKEN*`/`*BLOCKED*` → `AUTH_FAILED`, `*LIMIT*` → `RATE_LIMITED`).

### Allegro (OAuth Device Flow, tylko odczyt)

Każdy użytkownik rejestruje własną, bezpłatną aplikację typu „device” w apps.developer.allegro.pl i wkleja w GUI
**Client ID** (niesekretny → `Source.settings`) oraz **Client Secret** (credential store). Potem „Połącz z Allegro”:
aplikacja pobiera kod urządzenia, otwiera stronę Allegro w przeglądarce (adres sprawdzany: musi prowadzić do `https://allegro.pl/`),
pokazuje kod do porównania i odpytuje o wynik (`authorization_pending`, `slow_down`/429 → rzadziej, limit czasu z `expires_in`).
`device_code` nigdy nie trafia do webview. Prosimy tylko o scope'y odczytu: `orders:read`, `sale:offers:read`, `profile:read`.

| Narzędzie | Endpoint |
|---|---|
| `get_account` (także test połączenia) | `GET /me` |
| `list_orders` (status, status realizacji, daty zakupu, login kupującego, limit ≤ 100, `offset`, limit+offset ≤ 10000) | `GET /order/checkout-forms` |
| `get_order` (UUID) | `GET /order/checkout-forms/{id}` |
| `list_offers` (tytuł, status publikacji, limit ≤ 200, `offset`) | `GET /sale/offers` |
| `get_offer` (bez opisu HTML) | `GET /sale/product-offers/{offerId}` |

Tokeny: `access_token` (12 h) i `refresh_token` (3 mies., **jednorazowy** — każda odpowiedź niesie nowy) to osobne wpisy
credential store, bo wpis Windows Credential Managera mieści 2560 bajtów (dlatego `KeyringStore` zapisuje surowe bajty
UTF-8, a nie UTF-16). Bez księgowania wygaśnięcia: access token jest używany do pierwszego `401`, wtedy jedno odświeżenie
i jedno ponowienie. Odświeżanie chroni plik-blokada `allegro-refresh.lock` w katalogu danych (`File::lock`) — wspólna dla GUI
i wszystkich sesji MCP, więc dwa procesy nie zużyją tego samego refresh tokenu; pod blokadą najpierw sprawdzamy, czy ktoś
już nie odświeżył. Kolejność zapisu: refresh token, potem access token. Odrzucony refresh token → `AUTH_FAILED`
(„Połącz ponownie” w GUI). Zmiana Client ID/Secret kasuje tokeny; usunięcie źródła kasuje wszystkie trzy wpisy.
Proces MCP zapisuje więc tokeny w credential store, ale `config.json` nadal tylko czyta.

### Plugin dla Claude Desktop

„Połącz klienta AI → Pobierz plugin” zapisuje w Pobranych `ecommerce-mcp-plugin.zip` w formacie pluginów Claude:
`.claude-plugin/plugin.json`, `.mcp.json` (serwer `ecommerce-mcp` = bezwzględna ścieżka do zainstalowanego binarium + `mcp`)
oraz `skills/` (`przeglad-zamowien`, `obsluga-zamowienia`, `produkty-i-stany`, `raport-sprzedazy`, `allegro-sprzedaz`). Zip powstaje w chwili
kliknięcia, bo ścieżka zależy od miejsca instalacji; nie zawiera sekretów. Użytkownik importuje go sam
(Claude Desktop → Customize → Plugins → dodaj z pliku) — aplikacja nie modyfikuje konfiguracji klientów AI.
Test `plugin_zip_has_manifest_mcp_server_and_valid_skills` pilnuje m.in., żeby skille nie odsyłały do nieistniejących narzędzi.

### Dodawanie providera

1. Nowy moduł w `src-tauri/src/integrations/` implementujący trait `Provider` (metadane, pola formularza — sekretne i
   niesekretne, walidacja, `test_connection`, `tools`, `call_tool`, mapowanie błędów; dla OAuth dodatkowo `AuthKind::OauthDevice`,
   `token_kinds`, `begin_authorization`, `poll_authorization`). HTTP przez `integrations/http.rs`.
2. Jedna linia w `Registry::default()`.
3. Teksty w `src/i18n/pl.ts`: `cap.*`, `provider.<id>.desc`, `provider.<id>.help`, `form.field.<klucz>`, opcjonalnie
   `errors.<KOD>.<id>` i `auth.<id>.*`.

Rdzeń MCP, warstwa sekretów i GUI nie wymagają zmian; test kontraktu `provider_contract_holds_for_every_provider`
sprawdzi nowy provider automatycznie.

## Co pokrywają testy

- konfiguracja: brak pliku, roundtrip, atomowość (brak plików tymczasowych), migracja v0→v1, odrzucenie nowszego schematu, uszkodzony plik, slugi `source_id`;
- sekrety: namespacing kluczy, niedostępny store, zredagowany `Debug`; brak tokenu w konfiguracji, logach, raporcie, błędach i odpowiedziach dla GUI;
- BaseLinker (mock HTTP): sukces, zły token, timeout + ponowienia, rate limit, 5xx/nie-JSON, powrót po błędzie przejściowym, zapisy bez ponowień, limit notatki, walidacja wejścia przed siecią;
- Allegro (mock HTTP): start device flow (Basic auth, tylko scope'y odczytu, obcy adres weryfikacji odrzucony), stany pollingu,
  odświeżenie po 401 z zapisem rotacji, dokładnie jedno odświeżenie przy równoległych wywołaniach, `invalid_grant`, mapowanie
  filtrów, walidacja wejścia przed siecią (w tym próby path traversal w identyfikatorach), whitelist pól;
- Service: formularz Allegro → autoryzacja → test, anulowanie/odmowa, kasowanie tokenów przy zmianie danych i usunięciu źródła;
- rejestr i kontrakt providerów; MCP: dynamiczne `tools/list`, znormalizowane błędy źródeł;
- E2E: handshake, `tools/list`, `tools/call` (statusy, szczegóły zamówienia), walidacja, wyłączenie źródła w trakcie sesji, czyste wyjście;
- UI: dodanie źródła → test połączenia → wyłączenie → usunięcie; Allegro: formularz → kod → „Połączono”, rezygnacja →
  „Połącz ponownie”; pobranie pluginu; zablokowana zakładka Codex.


## Bezpieczeństwo i prywatność

- Brak telemetrii i jakiejkolwiek komunikacji poza API podłączonych providerów.
- Webview: CSP `default-src 'self'`, brak zdalnego kodu; capability tylko `core:default` (GUI nie ma dostępu do plików, shella ani sieci — wyłącznie komendy aplikacji).
- GUI nigdy nie dostaje tokenu z powrotem; pola hasła są czyszczone po wysłaniu.
- Aplikacja nie edytuje plików konfiguracyjnych klientów AI — pokazuje fragment do skopiowania.
- Logi tylko na stderr (w trybie MCP trafiają do logów klienta), po redakcji; nie logujemy danych zamówień.

## Świadomie pominięte w MVP

- **Auto-aktualizacje:** brak `tauri-plugin-updater`, dopóki nie ma serwera aktualizacji i kluczy podpisu. Fundament: wersja z `Cargo.toml`/`tauri.conf.json` jest widoczna w GUI, raporcie i `serverInfo` MCP; włączenie opisuje [pakowanie.md](pakowanie.md).
- **Podpisywanie/notaryzacja:** konfiguracja w CI jest przygotowana, artefakty są na razie niepodpisane.
- **Allegro — zapis:** wersja 1 tylko czyta (bez zmiany statusu realizacji, numerów przesyłek i wiadomości); brak też trybu sandbox.
- **Codex / Claude Code:** zakładka Codex jest widoczna jako „wkrótce”; serwer MCP jest zwykłym serwerem stdio, więc ręczna
  konfiguracja (`command` = ścieżka binarium, `args` = `["mcp"]`) działa w każdym kliencie MCP.
