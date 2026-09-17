---
name: allegro-sprzedaz
description: Przeglądanie zamówień i ofert z konta Allegro użytkownika przez E-commerce MCP. Użyj, gdy użytkownik pyta o Allegro — "pokaż dzisiejsze zamówienia z Allegro", "co kupił login kupujacy123", "zamówienia do wysłania", "ile sztuk zostało w ofercie", "które oferty są nieaktywne", "Allegro orders", "Allegro offers".
---

# Allegro — zamówienia i oferty (tylko odczyt)

Narzędzia pochodzą z serwera MCP `ecommerce-mcp` i mają nazwy `allegro__<source_id>__<narzędzie>`.
Źródła i ich prefiksy podaje `ecommerce_mcp_list_sources` — przy kilku kontach dopytaj, o które chodzi.
Ta wersja niczego w Allegro nie zmienia: nie ma narzędzi do zmiany statusu, cen ani stanów.

## Zamówienia — `list_orders`

- Wyniki są **od najnowszych**. Filtry: `status`, `fulfillment_status`, `bought_from` / `bought_to`
  (`YYYY-MM-DD` albo RFC 3339, UTC), `buyer_login` (dokładny login), `limit` 1–100 (domyślnie 25), `offset`.
- `status` (stan zakupu): `BOUGHT` — kupione, formularz dostawy niewypełniony; `FILLED_IN` — wypełnione, nieopłacone;
  `READY_FOR_PROCESSING` — opłacone/potwierdzone, **do realizacji**; `CANCELLED` — anulowane.
  Pytanie „co mam do wysłania” = `status: READY_FOR_PROCESSING` + `fulfillment_status: NEW` lub `PROCESSING`.
- `fulfillment_status` (stan realizacji po stronie sprzedawcy): `NEW`, `PROCESSING`, `READY_FOR_SHIPMENT`,
  `READY_FOR_PICKUP`, `SENT`, `PICKED_UP`, `CANCELLED`, `SUSPENDED`, `RETURNED`.
- Paginacja: gdy w wyniku jest `next_offset`, wywołaj ponownie z `offset = next_offset`. `total_count` mówi, ile
  zamówień pasuje do filtra — przy dużych liczbach zawęź daty zamiast stronicować (limit + offset ≤ 10000).
- Podsumowanie zawiera: `id`, statusy, `buyer_login`, `bought_at`, `total_to_pay` (kwota + waluta), `payment_type`,
  `paid_at` (brak = nieopłacone), `delivery_method`, `items_count`.

## Szczegóły zamówienia — `get_order`

`order_id` to UUID z listy. Zwraca kupującego, płatność, dostawę (adres, metoda, punkt odbioru), dane do faktury,
`messageToSeller` i pozycje (`lineItems`: oferta, ilość, cena). Dane osobowe pokazuj tylko w zakresie potrzebnym do odpowiedzi.

## Oferty — `list_offers`, `get_offer`

- `list_offers`: `name` (fragment tytułu), `status` (`ACTIVE`, `INACTIVE`, `ACTIVATING`, `ENDED`), `limit` 1–200, `offset`.
  W wyniku: cena (`sellingMode.price`), `stock.available` / `stock.sold`, `stats` (odwiedziny, obserwujący), `publication.status`.
- `get_offer`: `offer_id` to numer oferty z listy. Zwraca kategorię, cenę, stan, publikację, dostawę i powiązane produkty
  z katalogu. Opis HTML oferty jest pomijany.
- „Czego brakuje” = oferty `ACTIVE` z `stock.available` równym 0 lub niskim — stronicuj `list_offers` i zestaw wynik w tabeli.

## Konto — `get_account`

Login, e-mail, firma i rynek bazowy połączonego konta. Przydaje się, gdy użytkownik ma kilka kont.

## Błędy

`CREDENTIAL_UNAVAILABLE` → konto nie zostało jeszcze połączone: poproś o dokończenie łączenia w aplikacji E-commerce MCP.
`AUTH_FAILED` → autoryzacja wygasła (po 3 miesiącach bezczynności) albo aplikacji Allegro brakuje uprawnień do odczytu
zamówień/ofert: poproś o „Połącz ponownie” w aplikacji. `RATE_LIMITED` → odczekaj minutę, nie ponawiaj w pętli.
`VALIDATION_ERROR` → popraw argumenty według komunikatu. `NOT_FOUND` → nie ma takiego zamówienia lub oferty.
