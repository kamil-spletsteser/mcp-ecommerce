---
name: obsluga-zamowienia
description: Bezpieczna zmiana danych zamówienia w BaseLinkerze przez E-commerce MCP — zmiana statusu i dopisanie notatki sprzedawcy. Użyj, gdy użytkownik chce coś zmienić w zamówieniu, np. "przenieś zamówienie 12345 do Wysłane", "oznacz jako spakowane", "dodaj notatkę do zamówienia", "update order status".
---

# Obsługa zamówienia (operacje zapisu)

Narzędzia `update_order_status` i `add_order_note` **zmieniają prawdziwe dane sklepu**. Zmiana statusu może
uruchomić automatyzacje sprzedawcy (e-maile do klienta, nadanie przesyłki, faktura). Nie da się jej cofnąć
inaczej niż kolejną zmianą statusu.

Nazwy narzędzi: `<provider>__<source_id>__<narzędzie>`; źródła i prefiksy podaje `ecommerce_mcp_list_sources`.

## Procedura

1. **Zidentyfikuj zamówienie.** Wywołaj `get_order` i sprawdź, że to właściwe zamówienie (klient, kwota, obecny status).
   Gdy użytkownik nie podał numeru, znajdź je przez `list_orders` i potwierdź wybór.
2. **Ustal wartość docelową.** Status: `get_order_statuses` → dopasuj `id` po nazwie. Przy kilku pasujących
   nazwach pokaż je i dopytaj. Nigdy nie zgaduj `status_id`.
3. **Poproś o potwierdzenie** jednym zdaniem z konkretami, np.:
   „Zamówienie 12345 (Jan Kowalski, 249,00 PLN): status *Nowe* → *Do wysłania*. Potwierdzasz?”
   Wykonaj zapis dopiero po wyraźnej zgodzie. Wyjątek: użytkownik w tej samej wiadomości podał jednoznacznie
   numer zamówienia i docelowy status — wtedy wykonaj, ale i tak najpierw sprawdź zamówienie przez `get_order`.
4. **Wykonaj** `update_order_status` (`order_id`, `status_id`) albo `add_order_note` (`order_id`, `note`).
5. **Potwierdź wynik** na podstawie odpowiedzi narzędzia (`ok`, `action`, `order_id`). Przy błędzie powiedz wprost,
   że zmiana **nie** została wykonana.

## Notatki — `add_order_note`

- Notatka jest **dopisywana** do komentarza sprzedawcy (`admin_comments`), po separatorze ` | `.
- BaseLinker ogranicza całe pole do **200 znaków**. Jeśli narzędzie zwróci `VALIDATION_ERROR` o długości,
  zaproponuj krótszą treść — nie usuwaj samodzielnie istniejących notatek.
- Pisz notatki zwięźle i rzeczowo; bez danych wrażliwych, których tam wcześniej nie było.

## Wiele zamówień naraz

Pokaż pełną listę zmian (numer, klient, status z → na) i poproś o jedno zbiorcze potwierdzenie. Wykonuj po kolei;
po pierwszym błędzie zatrzymaj się i zgłoś, co zostało zmienione, a co nie. Zapisy nie są ponawiane automatycznie —
po błędzie sieci sprawdź stan przez `get_order`, zanim spróbujesz ponownie. Limit API to 100 zapytań na minutę.
