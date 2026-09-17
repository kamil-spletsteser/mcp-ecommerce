import { useState } from "react";
import { api, toolName, type Provider, type Source } from "../api";
import { errorText, formatDate, t, tDynamic } from "../i18n";
import { needsAuthorization } from "./SourceDialogs";
import { Alert, Badge, Button, ProviderIcon, type Tone } from "./ui";

function status(source: Source): { tone: Tone; label: string } {
  if (!source.enabled) return { tone: "neutral", label: t("source.status.disabled") };
  if (!source.last_test) return { tone: "neutral", label: t("source.status.untested") };
  return source.last_test.ok ? { tone: "ok", label: t("source.status.ok") } : { tone: "warn", label: t("source.status.error") };
}

interface Props {
  source: Source;
  provider: Provider | undefined;
  onChanged: () => Promise<void>;
  onEdit: () => void;
  onDelete: () => void;
  /** Ponowna autoryzacja w przeglądarce — tylko dla źródeł OAuth, które wymagają uwagi. */
  onReconnect: () => void;
}

export function SourceCard({ source, provider, onChanged, onEdit, onDelete, onReconnect }: Props) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { tone, label } = status(source);

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await onChanged();
    } catch (e) {
      setError(errorText(e, source.provider));
    } finally {
      setBusy(false);
    }
  };

  const failedTest = source.enabled && source.last_test && !source.last_test.ok ? source.last_test : null;

  return (
    <article aria-label={source.name} className="rounded-2xl border border-line bg-surface p-5">
      <header className="flex items-start gap-4">
        <ProviderIcon id={source.provider} />
        <div className="min-w-0 flex-1">
          <h3 className="truncate text-base font-semibold">{source.name}</h3>
          <p className="text-sm text-muted">
            {provider?.name ?? source.provider}
            {source.settings.environment === "sandbox" && ` · ${t("source.sandbox")}`}
          </p>
        </div>
        <Badge tone={tone}>{label}</Badge>
        <label className="flex cursor-pointer items-center" title={t("source.enabled")}>
          <input
            type="checkbox"
            role="switch"
            aria-label={t("source.enabled")}
            checked={source.enabled}
            disabled={busy}
            onChange={(e) => run(() => api.setSourceEnabled(source.source_id, e.target.checked))}
            className="peer sr-only"
          />
          <span className="relative h-6 w-10 rounded-full bg-neutral-soft transition peer-checked:bg-accent peer-focus-visible:outline-2 peer-focus-visible:outline-accent after:absolute after:top-0.5 after:left-0.5 after:size-5 after:rounded-full after:bg-white after:shadow after:transition peer-checked:after:translate-x-4" />
        </label>
      </header>

      {failedTest && (
        <div className="mt-4">
          <Alert tone="warn">{errorText({ code: failedTest.code ?? "" }, source.provider)}</Alert>
        </div>
      )}
      {error && (
        <div className="mt-4">
          <Alert tone="danger">{error}</Alert>
        </div>
      )}

      <section className="mt-4">
        <h4 className="text-xs font-semibold tracking-wide text-muted uppercase">{t("source.capabilities")}</h4>
        {source.enabled ? (
          <>
            <ul className="mt-2 grid gap-x-6 gap-y-1 text-sm sm:grid-cols-2">
              {provider?.capabilities.map((capability) => (
                <li key={capability.label_key} className="flex items-baseline gap-2">
                  <span aria-hidden className="text-ok">
                    ✓
                  </span>
                  <span>
                    {tDynamic(capability.label_key)}
                    {capability.write && <span className="ml-1.5 rounded bg-warn-soft px-1.5 py-0.5 text-[11px] text-warn">{t("source.write_badge")}</span>}
                  </span>
                </li>
              ))}
            </ul>
            <details className="mt-3 text-sm">
              <summary className="cursor-pointer text-accent">{t("source.details")}</summary>
              <p className="mt-2 text-xs text-muted">{t("source.details.hint")}</p>
              <ul className="mt-1 space-y-0.5 font-mono text-xs text-muted">
                {provider?.tools.map((tool) => (
                  <li key={tool.name}>{toolName(source, tool.name)}</li>
                ))}
              </ul>
            </details>
          </>
        ) : (
          <p className="mt-2 text-sm text-muted">{t("source.capabilities.disabled")}</p>
        )}
      </section>

      <footer className="mt-4 flex flex-wrap items-center gap-2 border-t border-line pt-4">
        {needsAuthorization(provider, source) && (
          <Button variant="primary" disabled={busy} onClick={onReconnect}>
            {t("source.reconnect")}
          </Button>
        )}
        <Button disabled={busy} onClick={() => run(() => api.testSource(source.source_id))}>
          {busy ? t("source.testing") : t("source.test")}
        </Button>
        <Button variant="ghost" disabled={busy} onClick={onEdit}>
          {t("source.edit")}
        </Button>
        <Button variant="ghost" disabled={busy} onClick={onDelete}>
          {t("source.delete")}
        </Button>
        {source.last_test && <span className="ml-auto text-xs text-muted">{t("source.last_test", { date: formatDate(source.last_test.at) })}</span>}
      </footer>
    </article>
  );
}
