---
name: produkty-i-stany
description: Wyszukiwanie produktów, cen i stanów magazynowych w katalogach BaseLinkera przez E-commerce MCP. Użyj przy pytaniach typu "ile mamy sztuk SKU ABC-1", "znajdź produkt kubek", "jaka jest cena EAN 590...", "czego brakuje na magazynie", "stock", "inventory".
---

# Produkty i stany magazynowe

Nazwy narzędzi: `<provider>__<source_id>__<narzędzie>`; źródła i prefiksy podaje `ecommerce_mcp_list_sources`.

## Kolejność wywołań

1. `list_inventories` — katalogi produktów. `list_products` wymaga `inventory_id`. Gdy katalogów jest kilka,
   użyj tego z `is_default: true`, chyba że użytkownik wskazał inny. W wyniku są też `price_groups` i `warehouses` katalogu.
2. `list_products` — filtry: `name` (fragment nazwy), `sku` (dokładny), `ean` (dokładny). **Zawsze filtruj**, gdy
   użytkownik pyta o konkretny produkt — nie pobieraj całego katalogu, żeby szukać „ręcznie”.
3. `list_warehouses` — nazwy magazynów, gdy trzeba objaśnić klucze stanów.

## Czytanie wyniku

- `prices`: obiekt `{ "<id grupy cenowej>": cena brutto }`. Domyślną grupę wskazuje `default_price_group` katalogu.
- `stock`: obiekt `{ "<typ>_<id magazynu>": ilość }`, np. `bl_205`. Nazwę magazynu weź z `list_warehouses`
  (`warehouse_type` + `_` + `warehouse_id`). Stan łączny = suma po magazynach.
- `parent_id` różne od 0 oznacza wariant produktu głównego.

## Duże katalogi

BaseLinker zwraca strony po 1000 produktów. `limit` (domyślnie 100, maks. 1000) przycina stronę:
- `truncated: true` → na tej stronie jest więcej pozycji, niż pokazano: zawęź filtr albo zwiększ `limit`;
- `has_next_page: true` → pobierz `page + 1`.

Przegląd całego katalogu (np. „czego brakuje na magazynie”) rób strona po stronie z `limit: 1000`, licz wyniki
po drodze i pokaż zwięzłe zestawienie (SKU, nazwa, stan) zamiast surowych danych. Przy bardzo dużych katalogach
uprzedź, że to kilka–kilkanaście zapytań (limit API: 100 na minutę).

## Ograniczenia

To narzędzia tylko do odczytu — nie zmieniają cen ani stanów. Jeśli użytkownik chce coś zmienić, powiedz, że
obecna wersja E-commerce MCP tego nie obsługuje i trzeba to zrobić w panelu BaseLinkera.
