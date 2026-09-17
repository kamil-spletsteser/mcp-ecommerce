# E-commerce MCP

Lokalna aplikacja desktopowa (macOS, Windows), która łączy konta e-commerce z agentami AI przez
[Model Context Protocol](https://modelcontextprotocol.io). Użytkownik dodaje w GUI konto BaseLinker (token API) lub
Allegro (OAuth), a Claude dostaje lokalny serwer MCP z wąskimi, bezpiecznymi narzędziami — bez Dockera, Node.js, bazy danych
i konta w chmurze. Stack: Tauri 2, Rust, React + TypeScript.

| Dokument | Dla kogo |
|---|---|
| [docs/instrukcja-uzytkownika.md](docs/instrukcja-uzytkownika.md) | użytkownik końcowy: instalacja, dodawanie źródeł, plugin dla Claude |
| [docs/architektura.md](docs/architektura.md) | deweloper: decyzje, providerzy, narzędzia MCP, bezpieczeństwo, zakres testów |
| [docs/pakowanie.md](docs/pakowanie.md) | wydania: CI, wersjonowanie, podpisywanie, aktualizacje |
| [docs/specyfikacja.md](docs/specyfikacja.md) | pierwotne wymagania produktowe |
| [CLAUDE.md](CLAUDE.md) | zasady rozwijania projektu (także dla Claude Code) |

## 1. Co zainstalować na maszynie

Użytkownik końcowy nie instaluje niczego poza samą aplikacją. Poniższe dotyczy **budowania**.

### Wszystkie systemy

| Narzędzie | Wersja | Instalacja |
|---|---|---|
| Rust (`rustup`) | stable ≥ 1.89 | macOS: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` · Windows: [rustup-init.exe](https://rustup.rs) |
| Node.js | ≥ 20 | [nodejs.org](https://nodejs.org) lub menedżer wersji (fnm, nvm, volta) |
| pnpm | 10 | `corepack enable` — wersję bierze z pola `packageManager` w `package.json` |

`rust-toolchain.toml` w repo sprawia, że `rustup` sam doinstaluje `rustfmt` i `clippy`. Node jest potrzebny wyłącznie do
budowania frontendu. Nie trzeba OpenSSL, CMake ani NASM — TLS korzysta z bibliotek systemowych.

### macOS

```bash
xcode-select --install
```

Do Universal Binary (Apple Silicon + Intel) dodatkowo:

```bash
rustup target add x86_64-apple-darwin
```

### Windows

1. **Visual Studio Build Tools** z pakietem roboczym **„Desktop development with C++”** (MSVC + Windows SDK) —
   [visualstudio.microsoft.com/visual-cpp-build-tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/).
   `rustup` ma używać toolchaina `stable-msvc` (domyślny).
2. **WebView2** — jest wbudowany w Windows 10 (1803+) i 11; nic nie trzeba instalować.
3. Tylko dla instalatora `.msi`: włączona funkcja systemowa **VBSCRIPT** (Ustawienia → Funkcje opcjonalne). Instalator `.exe` (NSIS)
   jej nie wymaga. NSIS i WiX Tauri pobiera sam przy pierwszym buildzie.

Linux nie jest platformą docelową projektu (nietestowany).

## 2. Pierwsze uruchomienie

```bash
pnpm install
```

```bash
pnpm tauri dev
```

Pierwsza kompilacja Rusta trwa kilka minut; kolejne są przyrostowe. Zmiany w `src/` przeładowują się na żywo, zmiany w `src-tauri/`
przebudowują i restartują aplikację.

## 3. Testy i jakość

Frontend (TypeScript + testy UI na udawanym backendzie Tauri):

```bash
pnpm lint && pnpm test
```

Rust wymaga zbudowanego frontendu, bo `tauri::generate_context!` czyta `dist/` już podczas kompilacji:

```bash
pnpm build
```

```bash
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

`cargo test` obejmuje testy jednostkowe, mockowane HTTP (wiremock) i E2E na prawdziwym binarium w trybie `mcp`; nie dotyka sieci
ani systemowego magazynu haseł. Test prawdziwego Keychaina / Credential Managera uruchamia się osobno (tworzy i kasuje wpis-atrapę):

```bash
cd src-tauri && cargo test --lib -- --ignored real_credential_store
```

To samo robi CI na macOS i Windows: [.github/workflows/ci.yml](.github/workflows/ci.yml).

## 4. Budowanie paczki

```bash
pnpm tauri build
```

Na macOS przy budowaniu z terminala bez uprawnienia „Automatyzacja → Finder” (np. z poziomu Claude Code) użyj:

```bash
CI=true pnpm tauri build
```

`CI=true` pomija wyłącznie kosmetyczne układanie okna obrazu dysku przez AppleScript — tak samo buduje GitHub Actions.

| System | Artefakty w `src-tauri/target/release/bundle/` |
|---|---|
| macOS | `macos/E-commerce MCP.app`, `dmg/E-commerce MCP_<wersja>_aarch64.dmg` |
| Windows | `nsis/E-commerce MCP_<wersja>_x64-setup.exe` (instalacja per użytkownik, bez administratora), `msi/E-commerce MCP_<wersja>_x64_en-US.msi` |

Universal Binary na macOS:

```bash
pnpm tauri build --target universal-apple-darwin
```

Paczka zawiera jedno binarium `ecommerce-mcp` (GUI; z argumentem `mcp` — serwer MCP po stdio) i zasoby UI. Wydania z tagu `v*`
buduje [.github/workflows/release.yml](.github/workflows/release.yml). Wersjonowanie, podpisywanie i aktualizacje:
[docs/pakowanie.md](docs/pakowanie.md).

## 5. Typowe problemy

| Objaw | Przyczyna i rozwiązanie |
|---|---|
| `bundle_dmg.sh` kończy się błędem, w logu `-1743` | Terminal nie może sterować Finderem. Buduj z `CI=true` albo nadaj uprawnienie w Ustawieniach systemowych → Prywatność → Automatyzacja. |
| Błąd kompilacji Rusta o brakującym `../dist` / `frontendDist` | Najpierw `pnpm build` (robi to też `pnpm tauri build` i `pnpm tauri dev`). |
| macOS pyta o dostęp do pęku kluczy po każdym przebudowaniu | Niepodpisane binarium zmienia tożsamość przy każdym buildzie. Wybierz „Zawsze pozwalaj”; znika po podpisaniu Developer ID. |
| Po buildzie aplikacja wygląda jak stara wersja | Działa poprzednia instancja — zamknij ją (Cmd+Q) i uruchom ponownie. |
| Claude nie widzi serwera po przeniesieniu aplikacji | Plugin i ręczna konfiguracja wskazują bezwzględną ścieżkę binarium. Po instalacji do `/Applications` pobierz plugin ponownie. |
| `Port 1420 is already in use` | Działa inny `pnpm dev` / `pnpm tauri dev` — zamknij go. |
| `.msi` nie buduje się na Windows 11 | Włącz funkcję opcjonalną VBSCRIPT albo buduj tylko NSIS: `pnpm tauri build --bundles nsis`. |

## 6. Struktura repo

```text
src/                     GUI: React + TypeScript + Tailwind; teksty w src/i18n/pl.ts
src/test/                udawany backend Tauri (testy UI + podgląd w przeglądarce)
src-tauri/src/           Rust: app/ (komendy, logika GUI, self-check, plugin), config/, secrets/, mcp/,
                         integrations/ (baselinker/, allegro/, http.rs), diagnostics/
src-tauri/plugin/        skille i README pluginu Claude (wkompilowane w binarium)
src-tauri/tests/e2e.rs   E2E na prawdziwym binarium w trybie `mcp`
docs/                    dokumentacja
.github/workflows/       CI (macOS + Windows) i budowanie wydań
samples/                 (ignorowane przez git) cudze repozytoria trzymane lokalnie jako inspiracja
```

Przydatne przy pracy nad kodem:

- **Podgląd GUI w przeglądarce** na udawanym backendzie: `pnpm dev`, potem `http://localhost:1420/src/test/preview.html`.
- **Ręczny test serwera MCP**: `npx @modelcontextprotocol/inspector src-tauri/target/debug/ecommerce-mcp mcp`.
- **Zmienne środowiskowe tylko w buildach debug** (`#[cfg(debug_assertions)]` — w release tego kodu nie ma):
  `ECOMMERCE_MCP_DATA_DIR`, `ECOMMERCE_MCP_BASELINKER_URL`, `ECOMMERCE_MCP_ALLEGRO_URL`,
  `ECOMMERCE_MCP_TEST_SECRETS` (JSON `{"<provider>/<source_id>/<rodzaj>": "…"}` → magazyn w pamięci zamiast Keychaina).
