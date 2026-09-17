// Test najważniejszego przepływu GUI na udawanym backendzie Tauri: dodanie źródła → test połączenia → wyłączenie → usunięcie.
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import App from "./App";
import { backend } from "./test/fakeBackend";

const TOKEN = "4005-10023-UITOKENUITOKENUITOKEN0123456789";

vi.mock("@tauri-apps/api/core", async () => {
  const { backend } = await import("./test/fakeBackend");
  return { invoke: backend.invoke };
});

beforeEach(() => backend.reset());

async function addSource(user: ReturnType<typeof userEvent.setup>) {
  await user.click((await screen.findAllByRole("button", { name: "+ Dodaj źródło" }))[0]);
  await user.click(screen.getByRole("button", { name: /BaseLinker/ }));
  await user.type(screen.getByLabelText("Nazwa źródła"), "Główny sklep");
  await user.type(screen.getByLabelText("Token API"), TOKEN);
  await user.click(screen.getByRole("button", { name: "Zapisz i przetestuj" }));
}

test("add source → test connection → disable → delete", async () => {
  const user = userEvent.setup();
  render(<App />);

  // stan pusty prowadzi użytkownika
  expect(await screen.findByText(/Dodaj pierwsze źródło/)).toBeInTheDocument();
  expect(await screen.findByText("Brak aktywnych źródeł")).toBeInTheDocument();

  await addSource(user);
  expect(backend.calls.find((c) => c.cmd === "add_source")?.args).toEqual({ provider: "baselinker", name: "Główny sklep", fields: { api_token: TOKEN } });

  // karta źródła: status, możliwości językiem użytkownika, nazwy narzędzi dopiero w szczegółach
  const card = await screen.findByRole("article", { name: "Główny sklep" });
  expect(within(card).getByText("Połączono")).toBeInTheDocument();
  expect(within(card).getByText("przeglądać zamówienia i ich szczegóły")).toBeInTheDocument();
  expect(within(card).getAllByText("zmienia dane")).toHaveLength(2); // zmiana statusu + notatka
  expect(within(card).getByText("baselinker__glowny_sklep__list_orders")).toBeInTheDocument();
  expect(await screen.findByText("MCP gotowe")).toBeInTheDocument();
  expect(document.body.innerHTML).not.toContain("UITOKEN");

  // test połączenia, który zawodzi → „wymaga uwagi” + zrozumiały komunikat, bez surowego błędu
  backend.testOk = false;
  await user.click(within(card).getByRole("button", { name: "Testuj połączenie" }));
  expect(await within(card).findByText(/BaseLinker odrzucił token/)).toBeInTheDocument();
  expect((await screen.findAllByText("Wymaga uwagi")).length).toBeGreaterThan(0);

  // wyłączenie: AI traci narzędzia
  await user.click(within(card).getByRole("switch", { name: "Źródło aktywne" }));
  expect(await within(card).findByText("Wyłączone")).toBeInTheDocument();
  expect(within(card).getByText(/AI nie widzi jego narzędzi/)).toBeInTheDocument();
  expect(await screen.findByText("Brak aktywnych źródeł")).toBeInTheDocument();

  // usunięcie wymaga potwierdzenia z informacją o tokenie
  await user.click(within(card).getByRole("button", { name: "Usuń" }));
  expect(screen.getByText(/token z systemowego magazynu haseł/)).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Usuń źródło i token" }));
  expect(await screen.findByText(/Dodaj pierwsze źródło/)).toBeInTheDocument();
  expect(backend.calls.some((c) => c.cmd === "delete_source")).toBe(true);
});

test("Allegro: form → browser authorization with a code → connected, read-only", async () => {
  const user = userEvent.setup();
  const SECRET = "AllegroSecretAllegroSecret0123456789";
  render(<App />);
  await user.click((await screen.findAllByRole("button", { name: "+ Dodaj źródło" }))[0]);
  await user.click(screen.getByRole("button", { name: /Allegro/ }));
  expect(screen.getByText(/zarejestruj nową aplikację/)).toBeInTheDocument();
  await user.type(screen.getByLabelText("Nazwa źródła"), "Moje Allegro");
  await user.type(screen.getByLabelText("Client ID"), "0123456789abcdef");
  await user.type(screen.getByLabelText("Client Secret"), SECRET);
  await user.type(screen.getByLabelText(/^Nagłówek User-Agent/), "MojSklep/1.0.0 (+https://mojsklep.pl/info)");
  // środowisko: domyślnie produkcja, do wyboru sandbox
  expect(screen.getByLabelText(/^Środowisko Allegro/)).toHaveValue("production");
  await user.selectOptions(screen.getByLabelText(/^Środowisko Allegro/), "sandbox");
  await user.click(screen.getByRole("button", { name: "Zapisz i połącz konto" }));
  expect(backend.calls.find((c) => c.cmd === "add_source")?.args).toEqual({
    provider: "allegro",
    name: "Moje Allegro",
    fields: { environment: "sandbox", client_id: "0123456789abcdef", client_secret: SECRET, user_agent: "MojSklep/1.0.0 (+https://mojsklep.pl/info)" },
  });

  // krok autoryzacji: nic nie startuje samo; po kliknięciu widać kod do porównania z przeglądarką
  expect(backend.calls.some((c) => c.cmd === "start_authorization")).toBe(false);
  await user.click(await screen.findByRole("button", { name: "Połącz z Allegro" }));
  expect(await screen.findByLabelText("Kod potwierdzenia")).toHaveTextContent("abc-123-def");
  expect(screen.getByText("Czekam na potwierdzenie w Allegro…")).toBeInTheDocument();

  backend.authorization!.resolve(); // użytkownik potwierdził dostęp w przeglądarce
  const card = await screen.findByRole("article", { name: "Moje Allegro" });
  expect(await within(card).findByText("Połączono")).toBeInTheDocument();
  expect(within(card).getByText("przeglądać oferty, ceny i stany")).toBeInTheDocument();
  expect(within(card).queryByText("zmienia dane")).not.toBeInTheDocument();
  expect(within(card).queryByRole("button", { name: "Połącz ponownie" })).not.toBeInTheDocument();
  expect(within(card).getByText("Allegro · sandbox")).toBeInTheDocument();
  expect(document.body.innerHTML).not.toContain(SECRET);
});

test("Allegro: abandoning authorization leaves a source that can be reconnected", async () => {
  const user = userEvent.setup();
  render(<App />);
  await user.click((await screen.findAllByRole("button", { name: "+ Dodaj źródło" }))[0]);
  await user.click(screen.getByRole("button", { name: /Allegro/ }));
  await user.type(screen.getByLabelText("Nazwa źródła"), "Moje Allegro");
  await user.type(screen.getByLabelText("Client ID"), "0123456789abcdef");
  await user.type(screen.getByLabelText("Client Secret"), "AllegroSecretAllegroSecret0123456789");
  await user.type(screen.getByLabelText(/^Nagłówek User-Agent/), "MojSklep/1.0.0 (+https://mojsklep.pl/info)");
  await user.click(screen.getByRole("button", { name: "Zapisz i połącz konto" }));
  await user.click(await screen.findByRole("button", { name: "Połącz z Allegro" }));
  await user.click(await screen.findByRole("button", { name: "Anuluj" }));
  expect(backend.calls.some((c) => c.cmd === "cancel_authorization")).toBe(true);

  const card = await screen.findByRole("article", { name: "Moje Allegro" });
  expect(within(card).getByText(/Konto Allegro nie jest jeszcze połączone/)).toBeInTheDocument();
  await user.click(within(card).getByRole("button", { name: "Połącz ponownie" }));
  await user.click(await screen.findByRole("button", { name: "Połącz z Allegro" }));
  await screen.findByLabelText("Kod potwierdzenia");
  backend.authorization!.resolve();
  expect(await within(await screen.findByRole("article", { name: "Moje Allegro" })).findByText("Połączono")).toBeInTheDocument();
});

test("Claude Desktop tab offers a one-file plugin download with import steps", async () => {
  const user = userEvent.setup();
  render(<App />);
  await user.click(await screen.findByRole("button", { name: "Połącz klienta AI" }));
  await user.click(await screen.findByRole("button", { name: "Pobierz plugin" }));
  expect(await screen.findByText(/Zapisano: .*ecommerce-mcp-plugin\.zip/)).toBeInTheDocument();
  expect(screen.getByText(/Customize → Plugins/)).toBeInTheDocument();
  expect(backend.calls.some((c) => c.cmd === "export_plugin")).toBe(true);

  // Codex: zakładka widoczna, ale zablokowana — z tooltipem „wkrótce”; kliknięcie niczego nie zmienia
  const codex = screen.getByRole("tab", { name: "Codex" });
  expect(codex).toHaveAttribute("aria-disabled", "true");
  expect(codex).toHaveAttribute("title", "Pojawi się wkrótce");
  await user.click(codex);
  expect(screen.getByRole("tab", { name: "Claude Desktop" })).toHaveAttribute("aria-selected", "true");
  expect(screen.getByRole("button", { name: "Pobierz plugin" })).toBeInTheDocument();
});
