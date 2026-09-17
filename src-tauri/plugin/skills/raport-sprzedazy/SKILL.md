---
name: raport-sprzedazy
description: Raport sprzedaży ze sklepu użytkownika na podstawie zamówień z BaseLinkera (E-commerce MCP) — liczba zamówień, obrót, średnia wartość koszyka, podział na statusy i źródła, zamówienia nieopłacone. Użyj przy prośbach "podsumuj sprzedaż z wczoraj", "raport tygodniowy", "ile zarobiliśmy w tym miesiącu", "sales report".
---

# Raport sprzedaży

Nazwy narzędzi: `<provider>__<source_id>__<narzędzie>`; źródła i prefiksy podaje `ecommerce_mcp_list_sources`.

## Zbieranie danych

1. Ustal okres w datach kalendarzowych. „Wczoraj” = jeden dzień, „ten tydzień” = od poniedzielnika, „ten miesiąc” =
   od 1. dnia miesiąca. Daty w narzędziu są w UTC i dotyczą **potwierdzenia** zamówienia — napisz to w raporcie.
2. Wywołuj `list_orders` z `date_from`, `date_to`, `limit: 100`. Dopóki `has_more` jest `true`, powtarzaj z
   `date_from = next_date_from`. Zbierz wszystkie strony przed liczeniem.
3. Raz wywołaj `get_order_statuses`, żeby zamienić `order_status_id` na nazwy.
4. Powyżej ok. 1500 zamówień (15 stron) zaproponuj krótszy okres albo uprzedź, że to potrwa (limit API: 100 zapytań/min).

## Liczenie

- Licz **osobno dla każdej waluty** (`currency`) — nie sumuj PLN z EUR.
- Obrót = suma `total_gross` (produkty + dostawa, brutto). Średnia wartość zamówienia = obrót / liczba zamówień.
- Nieopłacone: `payment_done` < `total_gross` — podaj liczbę i kwotę.
- Podziały: według nazwy statusu i według `order_source`.
- Statusy wyglądające na anulowane lub zwroty pokaż osobno i zapytaj, czy wyłączyć je z obrotu —
  nie zakładaj tego samodzielnie.
- Przy większej liczbie zamówień licz kodem lub arkuszem, jeśli masz takie narzędzie; nie sumuj „w pamięci”.

## Format raportu

1. Jedno zdanie: okres, sklep, liczba zamówień, obrót.
2. Tabela kluczowych liczb (zamówienia, obrót, średnia, nieopłacone).
3. Tabela według statusów, potem według źródeł.
4. Krótkie obserwacje (maks. 3) — tylko takie, które wynikają z danych.

Najlepiej sprzedające się produkty wymagają `get_order` dla każdego zamówienia (jedno zapytanie na zamówienie).
Rób to tylko na wyraźną prośbę i dla niewielkich okresów; uprzedź o liczbie zapytań.
