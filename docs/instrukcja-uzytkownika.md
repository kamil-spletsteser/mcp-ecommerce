# E-commerce MCP — instrukcja użytkownika

E-commerce MCP pozwala asystentowi AI (Claude, Codex) pracować z danymi Twojego sklepu. Wszystko działa na Twoim
komputerze; tokeny są przechowywane w systemowym magazynie haseł (Pęk kluczy na macOS, Menedżer poświadczeń na Windows).

## 1. Instalacja

- **macOS:** otwórz pobrany plik `.dmg` i przeciągnij **E-commerce MCP** do folderu Aplikacje.
  Dopóki aplikacja nie jest podpisana przez Apple, przy pierwszym uruchomieniu kliknij ją prawym przyciskiem → **Otwórz** → **Otwórz**.
- **Windows:** uruchom instalator `.exe` (lub `.msi`). Jeśli pojawi się ostrzeżenie SmartScreen, wybierz **Więcej informacji → Uruchom mimo to**.

Nie potrzebujesz Node.js, Dockera ani bazy danych.

## 2. Dodanie sklepu (BaseLinker)

1. Uruchom aplikację i kliknij **+ Dodaj źródło** → **BaseLinker**.
2. Wpisz nazwę (np. „Główny sklep”) i token API. Token znajdziesz w panelu BaseLinker: **Moje konto → API**.
   Token daje pełny dostęp do konta — nie udostępniaj go nikomu.
3. Kliknij **Zapisz i przetestuj**. Status **Połączono** oznacza, że wszystko działa.
   Status **Wymaga uwagi** — kliknij **Edytuj** i popraw token.

Na karcie źródła widzisz, **co może zrobić AI**. Pozycje oznaczone „zmienia dane” (zmiana statusu zamówienia,
dodanie notatki) modyfikują dane w BaseLinkerze — klient AI poprosi Cię o zgodę przed ich użyciem.

## 2a. Dodanie konta Allegro

Allegro wymaga, aby każdy sprzedawca miał własną, bezpłatną „aplikację” — to tylko para kluczy.

1. Wejdź na **apps.developer.allegro.pl**, zaloguj się kontem sprzedawcy (wymagane dwustopniowe logowanie) i zarejestruj nową
   aplikację. Jako typ wybierz **„Aplikacja będzie działać w środowisku bez dostępu do przeglądarki albo klawiatury”** (device).
   Wygeneruj też dla niej nagłówek **User-Agent** (apps.developer.allegro.pl/user-agent) — Allegro rozpoznaje po nim aplikację.
2. W E-commerce MCP kliknij **+ Dodaj źródło → Allegro**, wpisz nazwę, zostaw środowisko **Produkcyjne**, wklej **Client ID**,
   **Client Secret** i **User-Agent**, a potem kliknij **Zapisz i połącz konto**.
3. Kliknij **Połącz z Allegro**. W przeglądarce otworzy się strona Allegro — sprawdź, czy kod na stronie zgadza się z kodem
   w aplikacji, i potwierdź dostęp. Po chwili status zmieni się na **Połączono**.

Środowisko **Sandbox** służy do testów bez ruszania prawdziwego konta: to osobne Allegro (allegro.pl.allegrosandbox.pl) z własnymi
kontami i własną rejestracją aplikacji (apps.developer.allegro.pl.allegrosandbox.pl). Klucze z produkcji tam nie działają.

E-commerce MCP ma wyłącznie prawo odczytu: AI może przeglądać zamówienia i oferty, ale niczego na Allegro nie zmieni.
Po 3 miesiącach bez używania Allegro wymaga ponownego połączenia — na karcie źródła pojawi się przycisk **Połącz ponownie**.

## 3. Połączenie z Claude

### Claude Desktop — plugin (zalecane)

1. Kliknij **Połącz klienta AI** → zakładka **Claude Desktop** → **Pobierz plugin**.
   Plik `ecommerce-mcp-plugin.zip` trafia do folderu Pobrane (aplikacja pokaże go w Finderze/Eksploratorze).
2. W Claude Desktop otwórz **Customize → Plugins**, wybierz dodanie pluginu z pliku i wskaż pobrany zip.
3. Gotowe. Plugin zawiera serwer MCP oraz skille: przegląd zamówień, obsługa zamówienia (zmiana statusu, notatki),
   produkty i stany, raport sprzedaży oraz sprzedaż na Allegro. Zapytaj np.: „Podsumuj sprzedaż z wczoraj”.

Plugin wskazuje zainstalowaną aplikację na tym komputerze — po jej przeniesieniu lub reinstalacji pobierz plugin ponownie.
Plik nie zawiera tokenów.

### Ręczna konfiguracja (bez pluginu)

1. W oknie **Połącz klienta AI** rozwiń **Bez pluginu: ręczna konfiguracja**, skopiuj pokazany fragment i wklej go w
   Claude Desktop: **Ustawienia → Developer → Edit Config**, potem całkowicie zamknij i uruchom Claude ponownie.
2. Przycisk **Sprawdź konfigurację** potwierdza, że serwer MCP uruchamia się poprawnie.

Obsługa **Codex** pojawi się wkrótce (zakładka jest na razie zablokowana).

Aplikacja E-commerce MCP nie musi być otwarta, gdy korzystasz z AI — klient sam uruchamia serwer, gdy go potrzebuje.

## 4. Zarządzanie źródłami

- **Przełącznik** na karcie wyłącza źródło: AI przestaje widzieć jego narzędzia (po odświeżeniu listy / nowej rozmowie).
- **Usuń** kasuje konfigurację źródła oraz jego dane dostępowe (token API, Client Secret, tokeny Allegro) z systemowego magazynu haseł.
- **Diagnostyka** pokazuje stan magazynu haseł, serwera MCP i źródeł. **Kopiuj raport diagnostyczny** tworzy raport
  bez tokenów i bez danych zamówień — możesz go bezpiecznie wysłać do pomocy technicznej.

## 5. Odinstalowanie

1. Najpierw usuń w aplikacji wszystkie źródła (to kasuje tokeny z magazynu haseł) i usuń wpis `ecommerce-mcp`
   z konfiguracji klienta AI.
2. **macOS:** przenieś aplikację do Kosza; opcjonalnie usuń `~/Library/Application Support/com.lampartoms.ecommerce-mcp`.
   **Windows:** Ustawienia → Aplikacje → E-commerce MCP → Odinstaluj; opcjonalnie usuń `%LOCALAPPDATA%\com.lampartoms.ecommerce-mcp`.

Jeśli odinstalowałeś aplikację bez usuwania źródeł, tokeny znajdziesz i usuniesz ręcznie: w aplikacji **Dostęp do pęku kluczy**
(macOS) lub **Menedżer poświadczeń → Poświadczenia systemu Windows** — wpisy zaczynają się od `com.lampartoms.ecommerce-mcp`.
