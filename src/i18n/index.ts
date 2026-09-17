// Minimalne i18n: słownik + podstawianie {zmiennych}. Nowy język = nowy plik słownika i wpis w `dictionaries`.
import type { CommandError } from "../api";
import { pl } from "./pl";

export type MessageKey = keyof typeof pl;
const dictionaries = { pl } satisfies Record<string, Record<MessageKey, string>>;
const locale: keyof typeof dictionaries = "pl";

export function t(key: MessageKey, vars: Record<string, string | number> = {}): string {
  return dictionaries[locale][key].replace(/\{(\w+)\}/g, (_, name: string) => String(vars[name] ?? ""));
}

/** Klucze przychodzące z Rusta (np. `cap.orders.read`) — nieznany klucz pokazujemy wprost, zamiast wywracać UI. */
export function tDynamic(key: string): string {
  return key in dictionaries[locale] ? t(key as MessageKey) : key;
}

/** Tekst błędu dla kodu z Rusta; najpierw wariant dla providera (`errors.AUTH_FAILED.allegro`), potem ogólny. */
export function errorText(error: unknown, providerId?: string): string {
  const code = (error as Partial<CommandError> | null)?.code;
  const key = [`errors.${code}.${providerId}`, `errors.${code}`].find((k) => k in dictionaries[locale]);
  return key ? t(key as MessageKey) : t("errors.UNKNOWN");
}

export function formatDate(unixSeconds: number): string {
  return new Intl.DateTimeFormat("pl-PL", { dateStyle: "medium", timeStyle: "short" }).format(unixSeconds * 1000);
}
