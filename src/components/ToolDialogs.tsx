import { useEffect, useState } from "react";
import { api, type AppState, type ClientSetup, type McpCheck } from "../api";
import { errorText, formatDate, t } from "../i18n";
import { Alert, Badge, Button, CopyBlock, Dialog } from "./ui";

const LockIcon = () => (
  <svg aria-hidden viewBox="0 0 16 16" className="size-3.5" fill="currentColor">
    <path d="M8 1a3.5 3.5 0 0 0-3.5 3.5V7H4a1.5 1.5 0 0 0-1.5 1.5v5A1.5 1.5 0 0 0 4 15h8a1.5 1.5 0 0 0 1.5-1.5v-5A1.5 1.5 0 0 0 12 7h-.5V4.5A3.5 3.5 0 0 0 8 1Zm2 6H6V4.5a2 2 0 1 1 4 0V7Z" />
  </svg>
);

export function ConnectDialog({ onClose }: { onClose: () => void }) {
  const [setup, setSetup] = useState<ClientSetup | null>(null);
  const [check, setCheck] = useState<McpCheck | "running" | null>(null);

  useEffect(() => {
    api.clientSetup().then(setSetup, () => setSetup(null));
  }, []);

  const runCheck = async () => {
    setCheck("running");
    setCheck(await api.checkMcp().catch((e): McpCheck => ({ ok: false, server_version: null, protocol_version: null, tools: [], error: errorText(e) })));
  };

  return (
    <Dialog title={t("connect.title")} onClose={onClose} wide>
      <p className="text-sm leading-relaxed text-muted">{t("connect.intro")}</p>

      <div role="tablist" className="mt-4 flex gap-1 rounded-lg bg-neutral-soft p-1">
        <button role="tab" type="button" aria-selected className="flex-1 rounded-md bg-surface px-3 py-1.5 text-sm font-medium shadow-sm">
          {t("connect.tab.claude_desktop")}
        </button>
        {/* aria-disabled zamiast disabled: wyłączony <button> nie pokazuje tooltipa w WebKit */}
        <button
          role="tab"
          type="button"
          aria-selected={false}
          aria-disabled
          title={t("connect.tab.soon")}
          className="flex flex-1 cursor-not-allowed items-center justify-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium text-muted opacity-60"
        >
          <LockIcon />
          {t("connect.tab.codex")}
        </button>
      </div>

      {setup && (
        <div role="tabpanel" className="mt-4 space-y-3 text-sm">
          <PluginDownload />
          <details>
            <summary className="cursor-pointer text-accent">{t("connect.manual")}</summary>
            <div className="mt-3 space-y-3">
              <p className="leading-relaxed whitespace-pre-line">{t("connect.claude_desktop.steps")}</p>
              <p className="text-xs text-muted">{t("connect.file", { path: setup.claude_desktop_config_path })}</p>
              <CopyBlock label={t("connect.tab.claude_desktop")} text={setup.claude_desktop_json} />
            </div>
          </details>
          <p className="text-xs text-muted">{t("connect.note")}</p>
        </div>
      )}

      <div className="mt-5 border-t border-line pt-4">
        <Button onClick={runCheck} disabled={check === "running"}>
          {check === "running" ? t("connect.checking") : t("connect.check")}
        </Button>
        {check && check !== "running" && (
          <div className="mt-3">
            {check.ok ? (
              <p role="status" className="rounded-lg bg-ok-soft px-4 py-3 text-sm text-ok">
                {t("connect.check.ok", { version: check.server_version ?? "?", count: check.tools.length })}
              </p>
            ) : (
              <Alert tone="danger">{t("connect.check.error", { error: check.error ?? "" })}</Alert>
            )}
          </div>
        )}
      </div>
    </Dialog>
  );
}

function PluginDownload() {
  const [result, setResult] = useState<{ path: string } | { error: string } | "saving" | null>(null);
  const download = async () => {
    setResult("saving");
    setResult(await api.exportPlugin().then((path) => ({ path }), (e) => ({ error: errorText(e) })));
  };
  return (
    <section aria-label={t("connect.plugin.title")} className="rounded-xl border border-line p-4">
      <h3 className="font-semibold">{t("connect.plugin.title")}</h3>
      <p className="mt-1 leading-relaxed text-muted">{t("connect.plugin.body")}</p>
      <Button variant="primary" className="mt-3" onClick={download} disabled={result === "saving"}>
        {result === "saving" ? t("connect.plugin.saving") : t("connect.plugin.download")}
      </Button>
      {result && result !== "saving" && "error" in result && (
        <div className="mt-3">
          <Alert tone="danger">{result.error}</Alert>
        </div>
      )}
      {result && result !== "saving" && "path" in result && (
        <div className="mt-3 space-y-2">
          <p role="status" className="rounded-lg bg-ok-soft px-4 py-3 break-all text-ok">
            {t("connect.plugin.saved", { path: result.path })}
          </p>
          <p className="leading-relaxed whitespace-pre-line">{t("connect.plugin.steps")}</p>
        </div>
      )}
    </section>
  );
}

function Row({ label, ok, detail }: { label: string; ok: boolean; detail?: string | null }) {
  return (
    <div className="flex items-start justify-between gap-4 py-2.5">
      <div className="min-w-0">
        <p className="text-sm font-medium">{label}</p>
        {detail && <p className="mt-0.5 text-xs break-words text-muted">{detail}</p>}
      </div>
      <Badge tone={ok ? "ok" : "danger"}>{ok ? t("diag.ok") : t("diag.error")}</Badge>
    </div>
  );
}

export function DiagnosticsDialog({ state, mcp, onRefresh, onClose }: { state: AppState; mcp: McpCheck | null; onRefresh: () => Promise<void>; onClose: () => void }) {
  const [message, setMessage] = useState<{ tone: "ok" | "danger"; text: string } | null>(null);

  const copyReport = async () => {
    try {
      await navigator.clipboard.writeText(await api.diagnosticReport());
      setMessage({ tone: "ok", text: t("connect.copied") });
    } catch (e) {
      setMessage({ tone: "danger", text: errorText(e) });
    }
  };
  const openDir = () => api.openDataDir().then(() => setMessage(null), (e) => setMessage({ tone: "danger", text: errorText(e) }));

  return (
    <Dialog title={t("diag.title")} onClose={onClose} wide>
      <div className="divide-y divide-line">
        <div className="flex justify-between py-2.5 text-sm">
          <span className="font-medium">{t("diag.version")}</span>
          <span className="text-muted">{state.app_version}</span>
        </div>
        <Row label={t("diag.credential_store")} ok={state.credential_store.ok} detail={state.credential_store.message} />
        <Row label={t("diag.mcp")} ok={mcp?.ok ?? false} detail={mcp?.ok ? t("diag.mcp.detail", { version: mcp.server_version ?? "?", protocol: mcp.protocol_version ?? "?", count: mcp.tools.length }) : (mcp?.error ?? t("status.checking"))} />
        {state.config_error && <Row label="config.json" ok={false} detail={state.config_error} />}
      </div>

      <h3 className="mt-5 text-xs font-semibold tracking-wide text-muted uppercase">{t("diag.sources")}</h3>
      {state.sources.length === 0 ? (
        <p className="mt-2 text-sm text-muted">{t("diag.no_sources")}</p>
      ) : (
        <div className="divide-y divide-line">
          {state.sources.map((source) => (
            <Row
              key={source.source_id}
              label={`${source.name} (${source.source_id})`}
              ok={source.last_test?.ok ?? false}
              detail={
                source.last_test
                  ? `${source.enabled ? "" : `${t("source.status.disabled")} · `}${source.last_test.code ?? "OK"} — ${source.last_test.message} · ${formatDate(source.last_test.at)}`
                  : t("source.status.untested")
              }
            />
          ))}
        </div>
      )}

      <div className="mt-5 flex flex-wrap items-center gap-2 border-t border-line pt-4">
        <Button onClick={copyReport}>{t("diag.copy_report")}</Button>
        <Button onClick={openDir}>{t("diag.open_dir")}</Button>
        <Button variant="ghost" onClick={onRefresh}>
          {t("diag.refresh")}
        </Button>
      </div>
      <p className="mt-2 text-xs text-muted">{t("diag.report_hint")}</p>
      {message && (
        <p role="status" className={`mt-2 text-sm ${message.tone === "ok" ? "text-ok" : "text-danger"}`}>
          {message.text}
        </p>
      )}
    </Dialog>
  );
}
