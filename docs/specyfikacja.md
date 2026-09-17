# Prompt wdrożeniowy: lokalna aplikacja desktopowa E-commerce MCP

## Nazwa produktu i kierunek interfejsu

Nazwa aplikacji w całym produkcie, instalatorach, tytułach okien i dokumentacji użytkownika: **E-commerce MCP**. Unikaj roboczych nazw typu „Gateway”, „Hub” lub technicznego języka w głównym interfejsie.

Interfejs ma być bardzo prosty i produktowy, a nie panelem administracyjnym dla programisty. Nie buduj rozbudowanego bocznego menu tylko dlatego, że jest to popularny wzorzec. W MVP preferuj jeden główny widok typu dashboard oraz lekką nawigację kontekstową (np. ustawienia i diagnostyka dostępne z nagłówka). Najważniejsze działania muszą być dostępne bez szukania ich w menu: **Dodaj źródło**, zarządzanie istniejącymi źródłami i zobaczenie, co każde z nich umożliwia agentowi AI.

## Rola i cel

Zaprojektuj i zaimplementuj produkcyjne MVP **E-commerce MCP Desktop**: lokalną, wieloplatformową aplikację desktopową dla właściciela e-commerce. Aplikacja umożliwia bezpieczne podłączenie kont e-commerce (na początek BaseLinker), zarządzanie ich konfiguracją w GUI oraz udostępnienie agentom AI lokalnego serwera MCP (Model Context Protocol).

Najważniejszy efekt: użytkownik pobiera instalator na macOS albo Windows, instaluje aplikację, uruchamia ją dwuklikiem, dodaje token BaseLinkera w GUI i może podłączyć Codex lub Claude do lokalnego serwera MCP — bez Dockera, ręcznego Node.js, serwera bazy danych ani konta w usłudze chmurowej.

Nie buduj wieloużytkownikowego SaaS-a ani rozwiązania z tenantami. To jest aplikacja lokalna dla jednego użytkownika na jednym komputerze. Projektuj jednak moduły integracyjne tak, aby w przyszłości łatwo dodać Allegro i kolejne systemy e-commerce.

## Założenia produktowe

- Aplikacja działa lokalnie, offline w zakresie konfiguracji; sieć jest potrzebna wyłącznie do komunikacji z API podłączonych providerów.
- Użytkownik nie instaluje Dockera, Node.js, PostgreSQL/MySQL ani osobnego serwera.
- Wszystkie binaria potrzebne w runtime są dostarczone w aplikacji/instalatorze.
- Aplikacja ma czytelne desktopowe GUI; lokalna strona administracyjna na `localhost` nie jest głównym produktem.
- Lokalny serwer MCP jest uruchamiany, zatrzymywany i monitorowany przez aplikację desktopową.
- Dane konfiguracji nie mogą zawierać tokenów API w plaintext.
- Pierwszą pełną integracją jest **BaseLinker**. Allegro implementuj jako gotowy kontrakt/moduł szkieletowy, bez udawania kompletnej implementacji, jeśli brakuje dostępu lub specyfikacji OAuth.

## Wymagany stack

Wybierz następującą architekturę, chyba że istnieje udokumentowana przeszkoda techniczna:

- **Tauri 2** jako aplikacja desktopowa i warstwa systemowa.
- **Rust** dla procesu głównego, lokalnego serwera MCP, trwałej konfiguracji, bezpiecznego magazynu sekretów i adapterów integracyjnych.
- **React + TypeScript + Vite** dla GUI.
- Stylowanie: prosty, dostępny system komponentów (np. Tailwind + shadcn/ui lub równoważny), bez zależności wymagających osobnego serwera.
- Preferuj SDK/protokół MCP dla Rusta. Jeżeli dojrzałość konkretnej biblioteki uniemożliwia solidny transport stdio, zaimplementuj minimalną, zgodną z MCP obsługę JSON-RPC po stdio w Rust i wyraźnie pokryj ją testami kontraktowymi.

Nie opieraj runtime na procesie Node.js. Node może być użyty wyłącznie podczas budowania frontendu przez zespół deweloperski; nie może być wymagany na komputerze użytkownika końcowego.

## Architektura docelowa

Zaimplementuj monorepo/workspace o przejrzystym podziale odpowiedzialności, przykładowo:

```text
ecommerce-mcp-desktop/
  src/                         # React/TypeScript UI
  src-tauri/
    src/
      app/                     # inicjalizacja, lifecycle, Tauri commands
      config/                  # modele i bezsekretna konfiguracja na dysku
      secrets/                 # adapter OS credential store
      mcp/                     # protokół, registry narzędzi, transport stdio
      integrations/            # kontrakty i providerzy
        baselinker/
        allegro/               # szkielet/kontrakt na przyszłość
      diagnostics/             # logi redagowane z sekretów, health/status
    tests/
  docs/
```

### Komponenty

1. **Aplikacja Tauri**
   - uruchamia GUI oraz usługę/lokalny proces MCP;
   - przy starcie odczytuje konfigurację, sprawdza dostęp do sekretów i uruchamia MCP;
   - raportuje status: działający, zatrzymany, błąd konfiguracji, błąd połączenia;
   - gwarantuje kontrolowane zatrzymanie serwera i zamykanie uchwytów/pipe’ów przy wyjściu.

2. **Local MCP Server**
   - działa wyłącznie lokalnie;
   - domyślnie korzysta z **transportu stdio**, aby był naturalnie kompatybilny z klientami MCP i nie wystawiał portu sieciowego;
   - jest uruchamiany jako subprocess/bundled sidecar albo proces potomny aplikacji Rust — wybierz prostszy i najstabilniejszy wariant dla Tauri, uzasadnij go w README;
   - nie wypisuje tokenów ani danych uwierzytelniających na stdout/stderr;
   - ładuje konfigurację przy starcie sesji i pobiera sekret dopiero, gdy narzędzie danego źródła go potrzebuje;
   - stan MCP pozostaje bezstanowy pomiędzy wywołaniami, poza bezpiecznym cache’em technicznym o krótkim TTL, jeśli jest potrzebny.

3. **GUI**
   - komunikuje się z Rustem przez Tauri commands/events, nie bezpośrednio z plikami konfiguracji;
   - nie ma dostępu do pełnych tokenów po zapisaniu; może wyświetlać jedynie status typu „token zapisany”.

4. **Warstwa integracji**
   - każdy provider implementuje wspólny trait/interfejs, np. metadane providera, schemat konfiguracji, walidację, test połączenia oraz rejestrację MCP tools;
   - rejestr providerów jest centralny i nie wymaga zmian w rdzeniu MCP przy dodawaniu nowej integracji;
   - każde skonfigurowane źródło ma stabilne `source_id`, nazwę użytkownika, provider, status i metadane niebędące sekretami.

## Trwałość danych i sekrety

### Konfiguracja

Zapisuj wyłącznie dane niebędące sekretami w katalogu danych aplikacji właściwym dla systemu operacyjnego (użyj biblioteki systemowej, nie hardcode’uj ścieżek):

- macOS: Application Support właściwe dla aplikacji;
- Windows: Local AppData/Roaming AppData — wybierz jednoznacznie i udokumentuj wybór.

Przykładowe dane w konfiguracji: wersja schematu, lista źródeł, `source_id`, provider, nazwa, data utworzenia, ustawienia niesekretne, flaga aktywności i ostatni zredagowany wynik testu połączenia. Użyj wersjonowanego schematu z migracjami oraz atomowego zapisu pliku (plik tymczasowy + rename). Nie zapisuj w nim tokenów, haseł, refresh tokenów ani pełnych odpowiedzi HTTP mogących zawierać wrażliwe dane.

### Sekrety

Użyj systemowego magazynu poświadczeń przez przenośną bibliotekę Rust (np. `keyring`/odpowiednik), z osobnymi wpisami per `source_id` i typ sekretu.

- macOS: Keychain.
- Windows: Windows Credential Manager lub DPAPI poprzez sprawdzoną bibliotekę.
- Klucz wpisu powinien być namespacowany nazwą aplikacji, providerem, `source_id` i rodzajem sekretu.
- Token trafia do credential store podczas zapisu formularza; UI otrzymuje tylko wynik powodzenia/porażki.
- Token nie może znaleźć się w logach, telemetryce, raportach błędów, pliku konfiguracyjnym, test fixtures ani komunikatach API zwracanych do frontendu.
- W przypadku braku dostępu do credential store pokaż jasny błąd i nie zapisuj tokenu w fallbacku plaintext. Nie implementuj cichego fallbacku do pliku.
- Umożliwiaj zmianę i usunięcie sekretu wraz z usunięciem źródła. Po usunięciu źródła usuń konfigurację oraz odpowiadające wpisy credential store.

Zaimplementuj redakcję wartości wrażliwych w loggerze. Błędy API powinny zawierać kod/status i bezpieczny opis, nigdy nagłówek autoryzacji ani body z tokenem.

## UX i ekrany

Interfejs ma być po polsku w MVP, z gotową strukturą i18n do przyszłego rozszerzenia.

### 1. Start / Pulpit

- Jest to domyślny ekran po uruchomieniu aplikacji. Ma odpowiadać na trzy pytania: czy E-commerce MCP jest gotowe do pracy, jakie źródła są podpięte oraz co agent AI może dzięki nim zrobić.
- W nagłówku pokaż nazwę **E-commerce MCP** oraz dyskretny status „MCP gotowe”, „Wymaga uwagi” albo „Brak aktywnych źródeł”.
- Pokaż prostą kartę stanu: status lokalnego MCP, liczba aktywnych źródeł oraz główny przycisk **+ Dodaj źródło**. Przycisk ma być widoczny od razu, również gdy lista źródeł jest pusta.
- Poniżej pokaż karty podpiętych źródeł. Każda karta zawiera logo/ikonę, nazwę nadaną przez użytkownika, typ źródła (Allegro/BaseLinker), status połączenia i krótki opis możliwości.
- Dla każdego aktywnego źródła pokaż sekcję **„Co może zrobić AI”**, czyli językową listę capabilities, a nie listę surowych nazw MCP methods. Przykład: „przeglądać zamówienia”, „sprawdzać statusy”, „wyszukiwać produkty”, „aktualizować status zamówienia”. Drobniejszym drukiem lub po kliknięciu „Zobacz szczegóły” można pokazać odpowiadające im konkretne narzędzia MCP.
- Skróty „Połącz klienta AI” i „Diagnostyka” mogą znajdować się w nagłówku, menu z trzema kropkami albo na dole dashboardu; nie wymagają stałego bocznego menu.
- Stan pusty powinien wprost prowadzić użytkownika: „Dodaj pierwsze źródło, aby AI mogło pracować z Twoim e-commerce” oraz przycisk **+ Dodaj źródło**.

### 2. Integracje / Sources

- Ten widok może być pełnoekranowym rozwinięciem dashboardu albo modalem — nie musi być osobną pozycją rozbudowanego menu.
- lista źródeł: ikona providera, własna nazwa, status, data ostatniego testu oraz podsumowanie capabilities;
- przyciski: dodaj, edytuj, testuj połączenie, włącz/wyłącz, usuń;
- potwierdzenie usunięcia, z jasną informacją o usunięciu tokenu z magazynu systemowego;
- źródła wyłączone nie rejestrują swoich MCP tools.

### 2a. Dodawanie źródła

- Kliknięcie **+ Dodaj źródło** otwiera prosty wybór dostawcy w postaci dwóch czytelnych kart: **BaseLinker** i **Allegro**.
- Karta BaseLinkera informuje, że pozwala połączyć dane zamówień, produktów, magazynów i statusów przez API BaseLinkera.
- Karta Allegro ma istnieć już w MVP. Jeżeli pełna integracja Allegro nie jest jeszcze gotowa, powinna być uczciwie oznaczona „Wkrótce”, a po kliknięciu pokazywać krótką informację, do czego będzie służyć i możliwość zapisania zainteresowania/wyświetlenia statusu — bez udawania, że da się ją połączyć.
- Gdy implementacja Allegro jest gotowa, jej konfiguracja powinna być spójna z BaseLinkerem, lecz przystosowana do OAuth: jasno opisz proces połączenia konta, wymagane zgody i status odświeżania dostępu.

### 3. Konfiguracja BaseLinkera

- nazwa źródła (np. „Główny sklep”);
- token API jako pole typu password;
- opis, skąd uzyskać token, i ostrzeżenie, aby go nie udostępniać;
- walidacja lokalna, zapis bez sekretu w konfiguracji, zapis tokenu w credential store, następnie test połączenia;
- jeśli test zawiedzie, pozostaw źródło jako „wymaga uwagi” i pokaż możliwość korekty; nie ujawniaj tokenu.

### 4. Połącz klienta AI

Pokaż instrukcje dla co najmniej Claude Desktop i Codex — w formie kopiowalnego fragmentu konfiguracji lub działania wspieranego przez dany klient, jeśli da się to wykonać niezawodnie lokalnie.

Instrukcja ma wskazywać komendę do uruchomienia **dołączonego binarium MCP** (oraz argumenty, jeśli są konieczne), a nie `npx` ani komendę zależną od globalnie zainstalowanego Node. Uwzględnij różnice macOS/Windows oraz objaśnij, jak zweryfikować połączenie. Jeżeli automatyczna konfiguracja klienta jest niestabilna lub wymaga uprawnień, zapewnij bezpieczny wariant ręczny; nie edytuj plików konfiguracyjnych klientów bez wyraźnego potwierdzenia użytkownika.

Wraz z instrukcją pokaż „Sprawdź konfigurację” — test uruchamiający bezpieczne sprawdzenie, czy MCP da się uruchomić, bez wypisywania sekretów.

### 5. Diagnostyka

- status credential store;
- status serwera MCP;
- status i ostatni bezpieczny wynik połączenia każdego źródła;
- możliwość skopiowania zredagowanego raportu diagnostycznego;
- możliwość otwarcia katalogu danych aplikacji, bez automatycznego pokazywania sekretów.

## Zachowanie MCP

Implementuj MCP zgodnie z aktualną specyfikacją wybranego SDK/protokołu. W szczególności:

- serwer opisuje swoje możliwości i wersję;
- lista narzędzi jest dynamiczna: zawiera wyłącznie narzędzia aktywnych, poprawnie skonfigurowanych źródeł;
- nazwy narzędzi są stabilne, z namespacem providera i identyfikatorem źródła, np. `baselinker__main_store__list_orders`; alternatywnie użyj jednego parametru `source_id`, ale wybierz jeden spójny model i udokumentuj go;
- opisy i schematy wejścia są konkretne, aby agent AI nie zgadywał parametrów;
- waliduj wszystkie parametry po stronie Rust; ogranicz zakresy dat, limity i identyfikatory;
- zwracaj strukturalne, czytelne wyniki oraz znormalizowane błędy (`SOURCE_NOT_FOUND`, `SOURCE_DISABLED`, `CREDENTIAL_UNAVAILABLE`, `AUTH_FAILED`, `RATE_LIMITED`, `UPSTREAM_ERROR`, `VALIDATION_ERROR`);
- bezpiecznie obsłuż timeouty, retry wyłącznie dla błędów przejściowych i kontrolowany backoff; nie wykonuj automatycznych retry dla potencjalnie nieidempotentnych zapisów;
- operacje modyfikujące dane muszą mieć wyraźne nazwy i wymagać precyzyjnych argumentów. Nie twórz „uniwersalnego execute API request”.

Nie otwieraj domyślnie publicznego endpointu HTTP. Jeśli opcjonalny transport HTTP będzie potrzebny w przyszłości, powinien być domyślnie wyłączony, nasłuchiwać tylko na loopback i mieć osobny mechanizm autoryzacji — poza zakresem MVP.

## BaseLinker: pełna integracja MVP

Zaimplementuj klienta BaseLinker na podstawie aktualnej oficjalnej dokumentacji API. Izoluj endpointy i modele w module `integrations/baselinker`, aby zmiany API nie rozlewały się po aplikacji.

### Minimalny zestaw MCP tools

Udostępnij dla każdego aktywnego źródła BaseLinker co najmniej:

- pobranie statusów zamówień;
- listę zamówień z filtrami (status, zakres dat, limit/paginacja zgodna z API);
- pobranie szczegółów jednego zamówienia;
- listę magazynów;
- listę produktów/inwentarza z bezpiecznymi filtrami i limitami;
- aktualizację statusu zamówienia;
- dodanie notatki do zamówienia, jeśli wspiera je API.

Przed dodaniem operacji zapisu potwierdź dokładną nazwę i semantykę endpointu w oficjalnej dokumentacji. Dla każdej operacji zapisu zwróć jednoznaczne potwierdzenie wykonanej akcji i identyfikator obiektu. Rozpoznaj błędy uwierzytelnienia i limity API. Przy `test connection` użyj taniego, niezmieniającego danych endpointu.

Nie wystawiaj tokenu BaseLinkera w definicjach narzędzi ani wynikach. Nie implementuj arbitralnego proxy BaseLinker API; każde narzędzie ma mieć wąski, opisany cel.

## Kontrakt dla przyszłych providerów

Przygotuj rozszerzalny interfejs providera obejmujący co najmniej:

- identyfikator i metadane providera;
- JSON Schema/model pól konfiguracji niesekretnej i sekretnej;
- zapis/usunięcie/odczyt sekretów przez wspólną warstwę;
- walidację konfiguracji i `test_connection`;
- rejestrację MCP tools;
- klasyfikację i mapowanie błędów upstream.

Dodaj provider `allegro` jako wyraźnie oznaczony „w przygotowaniu”: metadane, ekran/informację dla użytkownika i miejsce na OAuth/token refresh, ale bez fałszywie działających narzędzi. Struktura ma pozwolić dodać pełną obsługę Allegro bez przebudowy rdzenia, szczególnie dla cyklu życia OAuth i odświeżania tokenów w credential store.

## Bezpieczeństwo i prywatność

- Zasada minimalnych uprawnień: tylko lokalny proces, tylko niezbędne API providerów.
- Zablokuj możliwość podania dowolnego URL/endpointu przez MCP tool, aby uniknąć SSRF i niekontrolowanego proxy.
- Ustaw rozsądne limity żądań, czasu i rozmiaru odpowiedzi; normalizuj/ograniczaj payload zwracany do modelu.
- Traktuj dane zamówień jako potencjalnie wrażliwe. Logi diagnostyczne powinny redukować PII do minimum.
- Nie dodawaj analityki ani wysyłania danych do chmury w MVP. Jeśli w przyszłości powstanie telemetria, ma być opt-in i pozbawiona sekretów/PII.
- Dodaj politykę CSP/webview zgodną z Tauri i nie ładuj zdalnego kodu wykonywalnego.
- Podpisuj/notaryzuj artefakty produkcyjne zgodnie z wymaganiami platform, gdy dostępne są poświadczenia wydawcy.

## Testy i jakość

Zaimplementuj i uruchom:

- testy jednostkowe Rust dla konfiguracji, migracji, redakcji logów, walidacji parametrów i mapowania błędów;
- testy jednostkowe provider registry oraz kontraktu integracji;
- testy z mockowanym HTTP dla klienta BaseLinker: sukces, błędny token, timeout, rate limit, błąd odpowiedzi i operacje zapisu;
- testy, że sekret nigdy nie trafia do serializowanej konfiguracji, logów ani odpowiedzi Tauri;
- testy MCP: inicjalizacja, dynamiczne `tools/list`, walidacja wejścia, wywołanie narzędzia i błąd źródła;
- testy UI najważniejszego przepływu: dodanie źródła, test połączenia, wyłączenie i usunięcie;
- co najmniej jeden test end-to-end z testowym/mockowanym BaseLinkerem i dołączonym binarium MCP;
- lint, formatowanie i CI dla macOS i Windows.

Do testów credential store zastosuj abstrakcję `SecretStore` oraz bezpieczny fake/in-memory store; nie wymagaj prawdziwego Keychaina/Credential Managera w CI.

## Pakowanie i instalacja

Przygotuj powtarzalne buildy release dla:

- macOS: aplikacja `.app` oraz `.dmg` (Apple Silicon; rozważ Universal Binary, jeśli jest realistyczne);
- Windows: instalator `.msi` lub `.exe` poprzez standardowe mechanizmy Tauri/WiX/NSIS.

Instalatory muszą zawierać główne binarium, zasoby UI i binarium/sidecar MCP, aby na czystym komputerze użytkownika nie był potrzebny Node ani Docker. Pierwsze uruchomienie powinno utworzyć katalog danych aplikacji dopiero wtedy, gdy jest potrzebny. Dodaj czytelną wersję aplikacji, mechanizm sprawdzania aktualizacji jako opcjonalny fundament (bez wymogu serwera aktualizacji w MVP) oraz instrukcję odinstalowania.

W README przygotuj instrukcje deweloperskie i osobną krótką instrukcję użytkownika. Nie każ użytkownikowi końcowemu uruchamiać komend z terminala do codziennego korzystania.

## Plan implementacji

Pracuj iteracyjnie w następującej kolejności:

1. Zainicjalizuj Tauri + React/TypeScript + Rust i skonfiguruj podstawowy build dla obu platform.
2. Zaimplementuj modele konfiguracji, atomowy zapis, migracje oraz `SecretStore` oparte o OS credential store.
3. Zbuduj lifecycle lokalnego serwera MCP i minimalne narzędzia diagnostyczne.
4. Zdefiniuj kontrakt providerów, registry i dynamiczną rejestrację MCP tools.
5. Zaimplementuj BaseLinker wraz z testami mock HTTP.
6. Zbuduj GUI pulpit/integracje/formularz BaseLinkera/diagnostykę oraz komunikację Tauri.
7. Dodaj ekran instrukcji dla Claude i Codex oraz bezpieczne sprawdzenie konfiguracji.
8. Dodaj testy E2E, CI, buildy release i dokumentację.

Na każdym etapie zachowuj kompilowalny, przetestowany stan. Jeśli szczegół oficjalnego API lub konfiguracji klienta MCP jest niepewny, zweryfikuj aktualną dokumentację źródłową przed implementacją i udokumentuj przyjętą wersję/procedurę.

## Kryterium akceptacji end-to-end

Funkcja jest zaakceptowana, gdy na świeżym komputerze macOS i Windows można wykonać następujący scenariusz bez instalowania Node.js, Dockera lub bazy danych:

1. Użytkownik instaluje aplikację z `.dmg` albo instalatora Windows i uruchamia ją dwuklikiem.
2. Aplikacja pokazuje, że lokalny serwer MCP działa albo jasno wskazuje jedyny wymagany krok naprawczy.
3. Użytkownik dodaje źródło „Główny sklep” typu BaseLinker, wpisuje token i zapisuje je.
4. Token jest zapisany wyłącznie w Keychainie na macOS albo Windows Credential Manager/DPAPI na Windows, a plik konfiguracji zawiera wyłącznie metadane źródła.
5. Test połączenia potwierdza dostęp do BaseLinkera lub zwraca bezpieczny, zrozumiały błąd.
6. Użytkownik kopiuje instrukcję konfiguracji Claude Desktop albo Codex i dodaje lokalne dołączone binarium MCP jako serwer MCP.
7. Klient AI widzi narzędzia aktywnego źródła BaseLinker i skutecznie pobiera listę statusów oraz szczegóły wybranego zamówienia.
8. Po wyłączeniu źródła jego narzędzia znikają po odświeżeniu/nowej sesji MCP; po usunięciu źródła token również zostaje usunięty z magazynu poświadczeń.
9. Żaden token nie pojawia się w konfiguracji, konsoli MCP, logach, raporcie diagnostycznym ani UI po ponownym uruchomieniu aplikacji.

Na końcu dostarcz działające źródła, README, instrukcję pakowania oraz wyniki uruchomionych testów. Nie ograniczaj się do makiet UI — kluczowy przepływ BaseLinker → credential store → local MCP → klient AI ma działać naprawdę.
