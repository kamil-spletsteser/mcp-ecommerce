# Pakowanie i wydania

## Build lokalny

```bash
pnpm install
pnpm tauri build
```

| System | Artefakty (`src-tauri/target/release/bundle/`) |
|---|---|
| macOS | `macos/E-commerce MCP.app`, `dmg/E-commerce MCP_<wersja>_aarch64.dmg` |
| Windows | `nsis/E-commerce MCP_<wersja>_x64-setup.exe` (instalacja per użytkownik, bez uprawnień administratora), `msi/…_x64_en-US.msi` |

Krok `.dmg` układa okno obrazu dysku przez AppleScript w Finderze. Jeśli terminal nie ma uprawnienia
„Automatyzacja → Finder” (błąd `-1743` w `bundle_dmg.sh`), nadaj je w Ustawieniach systemowych albo zbuduj z `CI=true`,
co pomija wyłącznie ten kosmetyczny krok (tak samo buduje GitHub Actions):

```bash
CI=true pnpm tauri build
```

Universal Binary (Apple Silicon + Intel):

```bash
rustup target add x86_64-apple-darwin
pnpm tauri build --target universal-apple-darwin
```

Instalator zawiera jedno binarium `ecommerce-mcp` (GUI + serwer MCP w trybie `ecommerce-mcp mcp`) i zasoby UI.
Na komputerze użytkownika nie jest potrzebny Node.js ani żaden runtime; na Windows wymagany jest WebView2
(wbudowany w Windows 10/11; instalator NSIS doinstaluje go w razie braku). TLS korzysta z bibliotek systemowych
(Security.framework / SChannel), więc build nie wymaga OpenSSL, CMake ani NASM.

Ścieżka do binarium, którą GUI podaje klientom AI:

- macOS: `/Applications/E-commerce MCP.app/Contents/MacOS/ecommerce-mcp`
- Windows: `%LOCALAPPDATA%\E-commerce MCP\ecommerce-mcp.exe`

## CI

- `.github/workflows/ci.yml` — macOS + Windows: `tsc`, Vitest, `cargo fmt --check`, `clippy -D warnings`, `cargo test`.
- `.github/workflows/release.yml` — po tagu `v*` (lub ręcznie) buduje `.dmg` (universal) oraz instalatory Windows i publikuje je jako artefakty.

## Wersjonowanie

Wersję podbijamy w trzech miejscach: `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`.
Jest widoczna w stopce GUI, w Diagnostyce, w raporcie i w `serverInfo` serwera MCP.

## Podpisywanie (jeszcze nieskonfigurowane)

Bez podpisu macOS pokaże ostrzeżenie Gatekeepera, a Windows — SmartScreen. Dodatkowo na macOS wpisy Pęku kluczy są
powiązane z tożsamością binarium: przy niepodpisanej aplikacji każda aktualizacja zmienia tę tożsamość i system jednorazowo
zapyta o dostęp do zapisanego tokenu („Zawsze pozwalaj”). Podpis Developer ID usuwa ten efekt.

- **macOS:** ustaw w sekretach CI `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
  `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` i przekaż je jako `env` kroku `pnpm tauri build` — Tauri podpisze i znotaryzuje bundle.
- **Windows:** certyfikat code-signing; `bundle.windows.certificateThumbprint` + `timestampUrl` w `tauri.conf.json` albo `signCommand` (np. Azure Trusted Signing).

## Aktualizacje (fundament)

MVP nie sprawdza aktualizacji (brak serwera i kluczy). Włączenie: `tauri-plugin-updater`, para kluczy
`pnpm tauri signer generate`, `plugins.updater.endpoints` wskazujące na `latest.json` (np. GitHub Releases) i
`bundle.createUpdaterArtifacts: true`. Sprawdzanie powinno być inicjowane przez użytkownika lub za jego zgodą — aplikacja
poza tym nie łączy się z niczym oprócz API providerów.
