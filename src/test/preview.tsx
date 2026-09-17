// Podgląd GUI w zwykłej przeglądarce: `pnpm dev` → http://localhost:1420/src/test/preview.html. Nie wchodzi do builda produkcyjnego.
import { mockIPC } from "@tauri-apps/api/mocks";
import { createRoot } from "react-dom/client";
import App from "../App";
import "../index.css";
import { backend } from "./fakeBackend";

backend.autoApproveMs = 4000; // w podglądzie „użytkownik” potwierdza dostęp w Allegro po 4 s
mockIPC((cmd, args) => backend.invoke(cmd, args as Record<string, unknown>));
createRoot(document.getElementById("root")!).render(<App />);
