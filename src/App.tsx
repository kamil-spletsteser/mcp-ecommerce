import { useCallback, useEffect, useState } from "react";
import { api, type AppState, type McpCheck, type Source } from "./api";
import { SourceCard } from "./components/SourceCard";
import { AddSourceDialog, AuthorizeDialog, DeleteSourceDialog, EditSourceDialog } from "./components/SourceDialogs";
import { ConnectDialog, DiagnosticsDialog } from "./components/ToolDialogs";
import { Alert, Badge, Button, type Tone } from "./components/ui";
import { t } from "./i18n";

type Modal = { kind: "add" } | { kind: "connect" } | { kind: "diagnostics" } | { kind: "edit" | "delete" | "authorize"; source: Source };

function overallStatus(state: AppState, mcp: McpCheck | null): { tone: Tone; label: string } {
  const active = state.sources.filter((s) => s.enabled);
  if (!mcp) return { tone: "neutral", label: t("status.checking") };
  const broken = !mcp.ok || !state.credential_store.ok || state.config_error || active.some((s) => s.last_test && !s.last_test.ok);
  if (broken) return { tone: "warn", label: t("status.attention") };
  if (active.length === 0) return { tone: "neutral", label: t("status.no_sources") };
  return { tone: "ok", label: t("status.ready") };
}

export default function App() {
  const [state, setState] = useState<AppState | null>(null);
  const [mcp, setMcp] = useState<McpCheck | null>(null);
  const [modal, setModal] = useState<Modal | null>(null);

  const refresh = useCallback(async () => {
    setState(await api.getState());
    setMcp(await api.checkMcp().catch((): McpCheck => ({ ok: false, server_version: null, protocol_version: null, tools: [], error: "check failed" })));
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (!state) return null;

  const status = overallStatus(state, mcp);
  const active = state.sources.filter((s) => s.enabled).length;
  const closeAndRefresh = async () => {
    setModal(null);
    await refresh();
  };

  return (
    <div className="mx-auto flex min-h-screen max-w-3xl flex-col gap-5 px-6 py-6">
      <header className="flex flex-wrap items-center gap-3">
        <h1 className="text-xl font-semibold">{t("app.name")}</h1>
        <Badge tone={status.tone}>{status.label}</Badge>
        <nav className="ml-auto flex gap-1">
          <Button variant="ghost" onClick={() => setModal({ kind: "connect" })}>
            {t("header.connect")}
          </Button>
          <Button variant="ghost" onClick={() => setModal({ kind: "diagnostics" })}>
            {t("header.diagnostics")}
          </Button>
        </nav>
      </header>

      {!state.credential_store.ok && <Alert tone="danger">{t("alert.credential_store")}</Alert>}
      {state.config_error && <Alert tone="danger">{t("alert.config")}</Alert>}
      {mcp && !mcp.ok && <Alert tone="danger">{t("alert.mcp")}</Alert>}

      <section aria-label={t("summary.mcp")} className="flex flex-wrap items-center gap-x-10 gap-y-4 rounded-2xl border border-line bg-surface p-5">
        <div>
          <p className="text-xs font-semibold tracking-wide text-muted uppercase">{t("summary.mcp")}</p>
          <p className="mt-1 text-lg font-semibold">{!mcp ? t("status.checking") : mcp.ok ? t("summary.mcp.ok") : t("summary.mcp.error")}</p>
        </div>
        <div>
          <p className="text-xs font-semibold tracking-wide text-muted uppercase">{t("summary.sources")}</p>
          <p className="mt-1 text-lg font-semibold">{active}</p>
        </div>
        <Button variant="primary" className="ml-auto px-5 py-2.5" onClick={() => setModal({ kind: "add" })}>
          {t("summary.add")}
        </Button>
        <p className="basis-full text-xs text-muted">{t("summary.hint")}</p>
      </section>

      {state.sources.length === 0 ? (
        <section className="rounded-2xl border border-dashed border-line px-6 py-12 text-center">
          <h2 className="mx-auto max-w-md text-lg font-semibold">{t("empty.title")}</h2>
          <p className="mx-auto mt-2 max-w-md text-sm text-muted">{t("empty.body")}</p>
          <Button variant="primary" className="mt-6 px-5 py-2.5" onClick={() => setModal({ kind: "add" })}>
            {t("summary.add")}
          </Button>
        </section>
      ) : (
        <section className="flex flex-col gap-4">
          <h2 className="text-sm font-semibold text-muted">{t("sources.title")}</h2>
          {state.sources.map((source) => (
            <SourceCard
              key={source.source_id}
              source={source}
              provider={state.providers.find((p) => p.id === source.provider)}
              onChanged={refresh}
              onEdit={() => setModal({ kind: "edit", source })}
              onDelete={() => setModal({ kind: "delete", source })}
              onReconnect={() => setModal({ kind: "authorize", source })}
            />
          ))}
        </section>
      )}

      <footer className="mt-auto pt-2 text-center text-xs text-muted">
        {t("app.name")} {state.app_version}
      </footer>

      {modal?.kind === "add" && <AddSourceDialog providers={state.providers} onDone={closeAndRefresh} onClose={() => setModal(null)} />}
      {modal?.kind === "edit" && (
        <EditSourceDialog source={modal.source} provider={state.providers.find((p) => p.id === modal.source.provider)!} onDone={closeAndRefresh} onClose={() => setModal(null)} />
      )}
      {modal?.kind === "authorize" && <AuthorizeDialog source={modal.source} provider={state.providers.find((p) => p.id === modal.source.provider)!} onDone={closeAndRefresh} />}
      {modal?.kind === "delete" && <DeleteSourceDialog source={modal.source} onDone={closeAndRefresh} onClose={() => setModal(null)} />}
      {modal?.kind === "connect" && <ConnectDialog onClose={() => setModal(null)} />}
      {modal?.kind === "diagnostics" && <DiagnosticsDialog state={state} mcp={mcp} onRefresh={refresh} onClose={() => setModal(null)} />}
    </div>
  );
}
