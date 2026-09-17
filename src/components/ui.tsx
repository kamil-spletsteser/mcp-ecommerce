import { useEffect, useRef, useState, type ButtonHTMLAttributes, type ReactNode } from "react";
import { t } from "../i18n";

type Variant = "primary" | "secondary" | "ghost" | "danger";
const variants: Record<Variant, string> = {
  primary: "bg-accent text-accent-ink hover:opacity-90",
  secondary: "border border-line bg-surface hover:bg-neutral-soft",
  ghost: "hover:bg-neutral-soft text-muted hover:text-ink",
  danger: "bg-danger text-white hover:opacity-90",
};

export function Button({ variant = "secondary", className = "", ...props }: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant }) {
  return (
    <button
      type="button"
      {...props}
      className={`inline-flex items-center justify-center gap-2 rounded-lg px-3.5 py-2 text-sm font-medium transition disabled:cursor-not-allowed disabled:opacity-50 ${variants[variant]} ${className}`}
    />
  );
}

export type Tone = "ok" | "warn" | "danger" | "neutral";
const tones: Record<Tone, string> = {
  ok: "bg-ok-soft text-ok",
  warn: "bg-warn-soft text-warn",
  danger: "bg-danger-soft text-danger",
  neutral: "bg-neutral-soft text-muted",
};

export function Badge({ tone, children }: { tone: Tone; children: ReactNode }) {
  return (
    <span className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-0.5 text-xs font-medium ${tones[tone]}`}>
      <span className="size-1.5 rounded-full bg-current" aria-hidden />
      {children}
    </span>
  );
}

export function Alert({ tone, children }: { tone: "warn" | "danger"; children: ReactNode }) {
  return (
    <p role="alert" className={`rounded-lg px-4 py-3 text-sm ${tones[tone]}`}>
      {children}
    </p>
  );
}

/** Natywny <dialog>: focus trap, Esc i aria-modal dostajemy od przeglądarki. */
export function Dialog({ title, onClose, children, wide = false }: { title: string; onClose: () => void; children: ReactNode; wide?: boolean }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (dialog && !dialog.open) dialog.showModal();
  }, []);
  return (
    <dialog
      ref={ref}
      aria-label={title}
      onClose={onClose}
      onClick={(e) => e.target === ref.current && onClose()}
      className={`m-auto w-[calc(100%-2rem)] ${wide ? "max-w-2xl" : "max-w-lg"} rounded-2xl border border-line bg-surface p-0 text-ink shadow-2xl`}
    >
      <div className="flex items-center justify-between border-b border-line px-6 py-4">
        <h2 className="text-base font-semibold">{title}</h2>
        <button type="button" onClick={onClose} aria-label={t("common.close")} className="rounded-md px-2 py-1 text-muted hover:bg-neutral-soft hover:text-ink">
          ✕
        </button>
      </div>
      <div className="px-6 py-5">{children}</div>
    </dialog>
  );
}

export function CopyBlock({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };
  return (
    <div className="relative">
      <pre aria-label={label} className="overflow-x-auto rounded-lg border border-line bg-page p-3 pb-4 text-xs leading-relaxed whitespace-pre">
        {text}
      </pre>
      <Button onClick={copy} className="absolute top-2 right-2 px-2.5 py-1 text-xs shadow-sm">
        {copied ? t("connect.copied") : t("connect.copy")}
      </Button>
    </div>
  );
}

export function ProviderIcon({ id }: { id: string }) {
  const style = id === "allegro" ? "bg-orange-500" : "bg-sky-600";
  const label = id === "allegro" ? "A" : "BL";
  return (
    <span aria-hidden className={`flex size-10 shrink-0 items-center justify-center rounded-xl text-sm font-bold text-white ${style}`}>
      {label}
    </span>
  );
}
