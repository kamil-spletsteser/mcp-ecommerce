---
name: przeglad-zamowien
description: Przeglądanie i wyszukiwanie zamówień ze sklepu użytkownika (BaseLinker) przez E-commerce MCP. Użyj, gdy użytkownik pyta o zamówienia — "pokaż dzisiejsze zamówienia", "co zamówił klient jan@example.com", "zamówienia w statusie Nowe", "szczegóły zamówienia 12345", "orders", "order details".
---

# Przegląd zamówień

Narzędzia pochodzą z serwera MCP `ecommerce-mcp`. Ich nazwy mają postać `<provider>__<source_id>__<narzędzie>`,
np. `baselinker__glowny_sklep__list_orders`.

## Krok 0 — ustal źródło

Jeśli nie wiesz, jakie źródła ma użytkownik, wywołaj `ecommerce_mcp_list_sources`. Zwraca `name` (nazwa nadana
przez użytkownika), `tool_prefix` i `enabled`. Przy kilku źródłach zapytaj, o który sklep chodzi — chyba że
użytkownik podał nazwę. Źródło z `enabled: false` nie ma narzędzi; powiedz, że trzeba je włączyć w aplikacji E-commerce MCP.

## Lista zamówień — `list_orders`

- `date_from` / `date_to`: `YYYY-MM-DD` albo RFC 3339, w UTC. Filtr dotyczy **daty potwierdzenia** zamówienia.
  Domyślnie ostatnie 30 dni; zakres maks. 366 dni.
- Wyniki są **od najstarszych**. Dla pytań „najnowsze / dzisiejsze” ustaw `date_from` na dziś lub wczoraj,
  zamiast pobierać cały miesiąc.
- `status_id`: liczba, nie nazwa. Gdy użytkownik mówi nazwą statusu („Nowe”, „Do wysłania”), najpierw wywołaj
  `get_order_statuses` i dopasuj `id` po nazwie. Przy niejednoznacznej nazwie pokaż kandydatów i dopytaj.
- `email`: dokładny adres kupującego. `limit`: 1–100 (domyślnie 25).
- Paginacja: gdy `has_more` jest `true`, wywołaj ponownie z `date_from = next_date_from`. Nie pobieraj kolejnych
  stron „na zapas” — tylko gdy pytanie tego wymaga (np. suma z całego okresu).

Wynik to podsumowania: `order_id`, `order_status_id`, daty, `delivery_fullname`, `email`, `currency`,
`total_gross` (produkty + dostawa), `payment_done`, `products_count`.

## Szczegóły — `get_order`

`order_id` z listy. Zwraca status, daty, dane dostawy i faktury, płatność, komentarze kupującego (`user_comments`)
i sprzedawcy (`admin_comments`) oraz `products` (nazwa, SKU, EAN, ilość, `price_brutto`, `tax_rate`).

## Prezentacja

- Zamieniaj `order_status_id` na nazwę statusu (z `get_order_statuses`; w jednej rozmowie wystarczy pobrać raz).
- Listy pokazuj w tabeli: numer, data, klient, kwota z walutą, status. Daty podawaj w czasie lokalnym użytkownika.
- `payment_done` mniejsze niż `total_gross` oznacza zamówienie nieopłacone lub opłacone częściowo — zaznacz to.
- Dane osobowe klientów pokazuj tylko w zakresie potrzebnym do odpowiedzi.

## Błędy

Wynik z `error.code`: `AUTH_FAILED` lub `CREDENTIAL_UNAVAILABLE` → poproś o poprawienie tokenu w aplikacji
E-commerce MCP (Edytuj źródło); `RATE_LIMITED` → odczekaj minutę, nie ponawiaj w pętli; `SOURCE_DISABLED` →
źródło wyłączone w aplikacji; `VALIDATION_ERROR` → popraw argumenty zgodnie z komunikatem; `NOT_FOUND` → nie ma takiego zamówienia.
