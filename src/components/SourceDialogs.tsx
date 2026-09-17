import { useRef, useState, type FormEvent } from "react";
import { api, type AuthorizationView, type Provider, type Source } from "../api";
import { errorText, t, tDynamic } from "../i18n";
import { Alert, Button, CopyBlock, Dialog, ProviderIcon } from "./ui";

const inputClass = "mt-1 w-full rounded-lg border border-line bg-page px-3 py-2 text-sm";

/** Źródło OAuth bez udanego testu = konto jeszcze niepołączone (albo autoryzacja wygasła). */
export const needsAuthorization = (provider: Provider | undefined, source: Source) => provider?.auth === "oauth_device" && !source.last_test?.ok;

/** Formularz dodawania i edycji. Pola pochodzą z metadanych providera; sekrety żyją tylko w stanie formularza do momentu wysłania. */
function SourceForm({ provider, source, onSaved, onCancel }: { provider: Provider; source?: Source; onSaved: (source: Source) => void; onCancel: () => void }) {
  const [name, setName] = useState(source?.name ?? "");
  const [fields, setFields] = useState<Record<string, string>>(() =>
    Object.fromEntries(provider.fields.filter((f) => !f.secret).map((f) => [f.key, source?.settings[f.key] ?? ""])),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const saved = source ? await api.updateSource(source.source_id, name, fields) : await api.addSource(provider.id, name, fields);
      setFields({});
      onSaved(saved);
    } catch (e) {
      setError(errorText(e, provider.id));
      setBusy(false);
    }
  };

  return (
    <form onSubmit={submit} className="space-y-4">
      <p className="text-xs leading-relaxed whitespace-pre-line text-muted">{tDynamic(`provider.${provider.id}.help`)}</p>
      <label className="block text-sm font-medium">
        {t("form.name")}
        <input className={inputClass} value={name} onChange={(e) => setName(e.target.value)} placeholder={t("form.name.placeholder")} maxLength={60} required autoFocus />
      </label>
      {provider.fields.map((field) => (
        <label key={field.key} className="block text-sm font-medium">
          {tDynamic(source && field.secret ? `form.field.${field.key}.keep` : `form.field.${field.key}`)}
          <input
            className={inputClass}
            type={field.secret ? "password" : "text"}
            autoComplete="off"
            spellCheck={false}
            maxLength={field.max_len}
            required={field.required && !(source && field.secret)}
            value={fields[field.key] ?? ""}
            onChange={(e) => setFields({ ...fields, [field.key]: e.target.value })}
          />
        </label>
      ))}
      <p className="text-xs leading-relaxed text-muted">{source ? t("form.secret.saved") : t("form.secret.note")}</p>
      {error && <Alert tone="danger">{error}</Alert>}
      <div className="flex justify-end gap-2 pt-1">
        <Button variant="ghost" onClick={onCancel} disabled={busy}>
          {t("form.cancel")}
        </Button>
        <Button type="submit" variant="primary" disabled={busy}>
          {busy ? t("form.saving") : provider.auth === "oauth_device" ? t("form.save_and_connect") : t("form.save")}
        </Button>
      </div>
    </form>
  );
}

/**
 * Krok autoryzacji w przeglądarce (OAuth Device Flow). Start dopiero po kliknięciu — nie w `useEffect`, bo StrictMode
 * uruchomiłby go dwa razy (dwa kody i dwie pętle odpytujące).
 */
export function AuthorizeDialog({ source, provider, onDone }: { source: Source; provider: Provider; onDone: () => Promise<void> }) {
  const [view, setView] = useState<AuthorizationView | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const closed = useRef(false);

  const connect = async () => {
    setBusy(true);
    setError(null);
    try {
      setView(await api.startAuthorization(source.source_id)); // Rust otwiera też przeglądarkę
      await api.finishAuthorization(source.source_id);
      if (!closed.current) await onDone();
    } catch (e) {
      if (closed.current) return;
      setView(null);
      setBusy(false);
      setError(errorText(e, provider.id));
    }
  };

  const cancel = async () => {
    closed.current = true;
    await api.cancelAuthorization(source.source_id).catch(() => undefined);
    await onDone(); // źródło już istnieje — lista musi się odświeżyć także po rezygnacji
  };

  // zamknięcie okna (✕ / Esc) = rezygnacja: pętla odpytująca w Ruście ma się zatrzymać
  return (
    <Dialog title={tDynamic(`auth.${provider.id}.title`)} onClose={cancel}>
      <div className="space-y-4 text-sm">
      {view ? (
        <>
          <p className="leading-relaxed text-muted">{tDynamic(`auth.${provider.id}.waiting`)}</p>
          <p aria-label={t("auth.code")} className="rounded-xl border border-line bg-page py-4 text-center font-mono text-2xl font-semibold tracking-widest">
            {view.user_code}
          </p>
          <p className="text-xs text-muted">{t("auth.manual")}</p>
          <CopyBlock label={t("auth.address")} text={view.verification_uri} />
          <p role="status" className="text-muted">
            {t("auth.pending")}
          </p>
        </>
      ) : (
        <p className="leading-relaxed whitespace-pre-line text-muted">{tDynamic(`auth.${provider.id}.intro`)}</p>
      )}
      {error && <Alert tone="danger">{error}</Alert>}
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={cancel}>
          {view ? t("form.cancel") : t("auth.later")}
        </Button>
        {!view && (
          <Button variant="primary" onClick={connect} disabled={busy}>
            {tDynamic(`auth.${provider.id}.connect`)}
          </Button>
        )}
      </div>
      </div>
    </Dialog>
  );
}

export function AddSourceDialog({ providers, onDone, onClose }: { providers: Provider[]; onDone: () => Promise<void>; onClose: () => void }) {
  const [selected, setSelected] = useState<Provider | null>(null);
  const [created, setCreated] = useState<Source | null>(null);

  if (selected && created) {
    return <AuthorizeDialog source={created} provider={selected} onDone={onDone} />;
  }

  if (!selected) {
    return (
      <Dialog title={t("add.title")} onClose={onClose}>
        <p className="mb-4 text-sm text-muted">{t("add.choose")}</p>
        <div className="grid gap-3 sm:grid-cols-2">
          {providers.map((provider) => (
            <button
              key={provider.id}
              type="button"
              onClick={() => setSelected(provider)}
              className="flex flex-col gap-3 rounded-xl border border-line p-4 text-left transition hover:border-accent"
            >
              <ProviderIcon id={provider.id} />
              <span className="font-semibold">{provider.name}</span>
              <span className="text-sm text-muted">{tDynamic(`provider.${provider.id}.desc`)}</span>
            </button>
          ))}
        </div>
      </Dialog>
    );
  }

  const saved = (source: Source) => (needsAuthorization(selected, source) ? setCreated(source) : void onDone());
  return (
    <Dialog title={`${t("add.title")} — ${selected.name}`} onClose={onClose}>
      <SourceForm provider={selected} onSaved={saved} onCancel={() => setSelected(null)} />
    </Dialog>
  );
}

export function EditSourceDialog({ source, provider, onDone, onClose }: { source: Source; provider: Provider; onDone: () => Promise<void>; onClose: () => void }) {
  const [reauthorize, setReauthorize] = useState<Source | null>(null);
  if (reauthorize) {
    return <AuthorizeDialog source={reauthorize} provider={provider} onDone={onDone} />;
  }
  // zmiana danych aplikacji OAuth kasuje tokeny → od razu prowadzimy do ponownego połączenia
  const saved = (updated: Source) => (needsAuthorization(provider, updated) ? setReauthorize(updated) : void onDone());
  return (
    <Dialog title={t("edit.title")} onClose={onClose}>
      <SourceForm provider={provider} source={source} onSaved={saved} onCancel={onClose} />
    </Dialog>
  );
}

export function DeleteSourceDialog({ source, onDone, onClose }: { source: Source; onDone: () => Promise<void>; onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const confirm = async () => {
    setBusy(true);
    try {
      await api.deleteSource(source.source_id);
      await onDone();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };
  return (
    <Dialog title={t("delete.title", { name: source.name })} onClose={onClose}>
      <p className="text-sm leading-relaxed text-muted">{t("delete.body")}</p>
      {error && (
        <div className="mt-4">
          <Alert tone="danger">{error}</Alert>
        </div>
      )}
      <div className="mt-5 flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose} disabled={busy}>
          {t("form.cancel")}
        </Button>
        <Button variant="danger" onClick={confirm} disabled={busy}>
          {t("delete.confirm")}
        </Button>
      </div>
    </Dialog>
  );
}
