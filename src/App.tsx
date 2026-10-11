import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { getVersion } from "@tauri-apps/api/app";
import { listen } from "@tauri-apps/api/event";
import "./App.css";
import claudeIcon from "./assets/providers/claude.svg?raw";
import codexIcon from "./assets/providers/codex.svg?raw";
import antigravityIcon from "./assets/providers/antigravity.svg?raw";

// ── Types ─────────────────────────────────────────────────────────────────────

interface UsagePeriod {
  utilization: number;
  resets_at: string;
}

interface DailyUsage {
  tokens: number;
  cost: number;
  unpriced_tokens: number;
  unpriced_models: string[];
}

interface ClaudeCodeData {
  is_available: boolean;
  status_line: string;
  five_hour: UsagePeriod | null;
  seven_day: UsagePeriod | null;
  today: DailyUsage | null;
  error: string | null;
}

interface ModelQuota {
  label: string;
  remaining_fraction: number;
  percent_used: number;
  reset_time: string;
}

interface AntigravityData {
  is_available: boolean;
  plan_name: string;
  models: ModelQuota[];
  status_line: string;
  today: DailyUsage | null;
  error: string | null;
}

interface CodexUsagePeriod {
  label_key: "codexSession" | "codexWeek";
  utilization: number;
  resets_at: string;
}

interface CodexData {
  is_available: boolean;
  status_line: string;
  plan_type: string;
  periods: CodexUsagePeriod[];
  today: DailyUsage | null;
  error: string | null;
}

interface AppSettings {
  enable_claude: boolean;
  enable_codex: boolean;
  enable_antigravity: boolean;
  enable_notifications: boolean;
  open_on_startup: boolean;
}

const defaultSettings: AppSettings = {
  enable_claude: true,
  enable_codex: true,
  enable_antigravity: true,
  enable_notifications: true,
  open_on_startup: false,
};

interface AllUsageData {
  claude_code: ClaudeCodeData;
  antigravity: AntigravityData;
  codex: CodexData;
}

// ── I18n ──────────────────────────────────────────────────────────────────────

type Lang = "es" | "en";
let currentLang: Lang = "en";

const DICT = {
  es: {
    loading: "Cargando...",
    refreshNow: "Actualizar ahora",
    developedBy: "Desarrollado por",
    visitWebsite: "Visitar sitio web",
    resetsIn: "Se reinicia en",
    soon: "pronto",
    justNow: "ahora mismo",
    ago: "hace",
    min: "min",
    noSession: "Sin sesión activa",
    notAvailable: "No disponible",
    updated: "Actualizado",
    session: "Sesión (5h)",
    week: "Semana",
    codexSession: "Sesión",
    codexWeek: "Semanal",
    pin: "Fijar ventana",
    settings: "Configuración",
    notifications: "Notificaciones",
    openOnStartup: "Abrir al iniciar sesión",
    save: "Guardar",
    aiProviders: "Proveedores de IA",
    appSettings: "Configuración de la app",
    cancel: "Cancelar",
    close: "Cerrar",
    about: "Acerca de",
    noProviders: "No hay proveedores activos. Actívalos en Configuración.",
    today: "hoy",
    costNote: "Costo equivalente a precios de API.",
    unpricedNote: "tokens sin precio publicado, no incluidos en el costo:",
  },
  en: {
    loading: "Loading...",
    refreshNow: "Refresh now",
    developedBy: "Developed by",
    visitWebsite: "Visit website",
    resetsIn: "Resets in",
    soon: "soon",
    justNow: "just now",
    ago: "",
    min: "mins ago",
    noSession: "No active session",
    notAvailable: "Not available",
    updated: "Updated",
    session: "Session (5h)",
    week: "Weekly",
    codexSession: "Session",
    codexWeek: "Weekly",
    pin: "Pin window",
    settings: "Settings",
    notifications: "Notifications",
    openOnStartup: "Open on sign-in",
    save: "Save",
    aiProviders: "AI providers",
    appSettings: "App settings",
    cancel: "Cancel",
    close: "Close",
    about: "About",
    noProviders: "No providers enabled. Turn them on in Settings.",
    today: "today",
    costNote: "Cost at API-equivalent prices.",
    unpricedNote: "tokens without a published price, not included in the cost:",
  }
};

function t(key: keyof typeof DICT.en): string {
  return DICT[currentLang][key];
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/** WinUI convention: accent fill while healthy, Fluent status colors near the limit. */
function utilizationLevel(pct: number): "" | "caution" | "critical" {
  if (pct >= 95) return "critical";
  if (pct >= 85) return "caution";
  return "";
}

function formatResetsAt(iso: string): string {
  if (!iso) return "";
  const diff = new Date(iso).getTime() - Date.now();
  if (diff <= 0) return t("soon");

  const d = Math.floor(diff / 86_400_000);
  const h = Math.floor((diff % 86_400_000) / 3_600_000);
  const m = Math.floor((diff % 3_600_000) / 60_000);

  if (d > 0) {
    if (h > 0) {
      return `${d}d ${h}h`;
    }
    return `${d}d ${m}m`;
  }
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

function formatLastUpdated(date: Date | null): string {
  if (!date) return "";
  const mins = Math.floor((Date.now() - date.getTime()) / 60_000);
  if (mins === 0) return t("justNow");
  const ago = t("ago");
  const min = t("min");
  return ago ? `${ago} ${mins} ${min}` : `${mins} ${min}`;
}

interface AccentColors {
  accent: string;
  light1: string;
  light2: string;
  light3: string;
  dark1: string;
  dark2: string;
  dark3: string;
}

/** Pushes the Windows accent palette into the CSS custom properties. */
function applyAccentColors() {
  invoke<AccentColors | null>("get_accent_colors")
    .then((c) => {
      if (!c) return;
      const root = document.documentElement.style;
      root.setProperty("--accent-light1", c.light1);
      root.setProperty("--accent-light2", c.light2);
      root.setProperty("--accent-light3", c.light3);
      root.setProperty("--accent-dark1", c.dark1);
      root.setProperty("--accent-dark2", c.dark2);
    })
    .catch(() => {});
}

// Segoe Fluent Icons glyphs
const Glyph = {
  pin: "",
  unpin: "",
  settings: "",
  refresh: "",
  close: "",
} as const;

function Icon({ glyph, className = "" }: { glyph: string; className?: string }) {
  return <span className={`icon ${className}`} aria-hidden="true">{glyph}</span>;
}

// ── Shared components ─────────────────────────────────────────────────────────

function ProgressBar({ pct, label, resetAt }: { pct: number; label: string; resetAt: string }) {
  const clamped = Math.min(100, Math.max(0, pct));
  const level = utilizationLevel(clamped);
  return (
    <div className="progress">
      <div className="progress-label-row">
        <span className="progress-label">{label}</span>
        {resetAt && <span className="progress-reset">{t("resetsIn")} {formatResetsAt(resetAt)}</span>}
        <span className={`progress-value ${level}`} style={resetAt ? undefined : { marginLeft: "auto" }}>
          {clamped.toFixed(0)}%
        </span>
      </div>
      <div
        className="progress-track"
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(clamped)}
      >
        <div className={`progress-fill ${level}`} style={{ width: `${clamped}%` }} />
      </div>
    </div>
  );
}

function ProgressRing() {
  return (
    <svg className="progress-ring" viewBox="0 0 16 16" aria-hidden="true">
      <circle cx="8" cy="8" r="7" />
    </svg>
  );
}

function ProviderIcon({ svg, className }: { svg: string; className: string }) {
  // Bundled SVG assets (no user content), inlined so they can use currentColor.
  return <span className={`provider-icon ${className}`} aria-hidden="true" dangerouslySetInnerHTML={{ __html: svg }} />;
}

function CardShell({
  name,
  icon,
  active,
  refreshing,
  children,
}: {
  name: string;
  icon: React.ReactNode;
  active: boolean;
  refreshing?: boolean;
  children: React.ReactNode;
}) {
  return (
    <section className="provider-card">
      <div className="card-header">
        {icon}
        <span className="provider-name">{name}</span>
        <span className="card-header-end">
          {refreshing
            ? <ProgressRing />
            : <span className={`status-dot ${active ? "active" : "inactive"}`} />}
        </span>
      </div>
      {children}
    </section>
  );
}

function CardStatus({ available, statusLine, error }: { available: boolean; statusLine: string; error: string | null }) {
  if (statusLine) return <div className="card-status">{statusLine}</div>;
  if (available) return null;
  return <div className={`card-status ${error ? "error" : ""}`}>{error || t("notAvailable")}</div>;
}

/** Today's tokens and API-equivalent cost; "≥" when some models have no published price. */
function UsageLine({ today }: { today: DailyUsage | null }) {
  if (!today) return null;
  const locale = currentLang === "es" ? "es-CL" : "en-US";
  const tokens = today.tokens.toLocaleString(locale);
  const cost = today.cost.toLocaleString(locale, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  const partial = today.unpriced_tokens > 0;
  const title = partial
    ? `${t("costNote")}\n${today.unpriced_tokens.toLocaleString(locale)} ${t("unpricedNote")} ${today.unpriced_models.join(", ")}`
    : t("costNote");
  return (
    <div className="card-status" title={title}>
      {tokens} tokens · {partial ? "≥ " : ""}${cost} {t("today")}
    </div>
  );
}

// ── Provider cards (presentational) ──────────────────────────────────────────

function ClaudeCard({ data, refreshing }: { data: ClaudeCodeData; refreshing: boolean }) {
  return (
    <CardShell
      name="Claude Code"
      icon={<ProviderIcon svg={claudeIcon} className="claude" />}
      active={data.is_available}
      refreshing={refreshing}
    >
      <CardStatus available={data.is_available} statusLine={data.status_line} error={data.error} />
      <UsageLine today={data.today} />
      {data.is_available && data.five_hour && (
        <ProgressBar pct={data.five_hour.utilization} label={t("session")} resetAt={data.five_hour.resets_at} />
      )}
      {data.is_available && data.seven_day && (
        <ProgressBar pct={data.seven_day.utilization} label={t("week")} resetAt={data.seven_day.resets_at} />
      )}
    </CardShell>
  );
}

function AntigravityCard({ data, refreshing }: { data: AntigravityData; refreshing: boolean }) {
  return (
    <CardShell name="Antigravity" icon={<ProviderIcon svg={antigravityIcon} className="antigravity" />} active={data.is_available} refreshing={refreshing}>
      <CardStatus available={data.is_available} statusLine={data.status_line} error={data.error} />
      <UsageLine today={data.today} />
      {data.is_available && data.models.map((m) => (
        <ProgressBar key={m.label} pct={m.percent_used} label={m.label} resetAt={m.reset_time} />
      ))}
    </CardShell>
  );
}

function CodexCard({ data, refreshing }: { data: CodexData; refreshing: boolean }) {
  return (
    <CardShell name="Codex" icon={<ProviderIcon svg={codexIcon} className="codex" />} active={data.is_available} refreshing={refreshing}>
      <CardStatus available={data.is_available} statusLine={data.status_line} error={data.error} />
      <UsageLine today={data.today} />
      {data.is_available && data.periods && data.periods.map((p, i) => (
        <ProgressBar key={i} pct={p.utilization} label={t(p.label_key)} resetAt={p.resets_at} />
      ))}
    </CardShell>
  );
}

// ── Dialogs (WinUI ContentDialog) ─────────────────────────────────────────────

function Dialog({ onClose, children }: { onClose: () => void; children: React.ReactNode }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="dialog-smoke" onClick={onClose}>
      <div className="dialog" role="dialog" aria-modal="true" onClick={(e) => e.stopPropagation()}>
        {children}
      </div>
    </div>
  );
}

function AboutModal({ onClose, version }: { onClose: () => void, version: string }) {
  return (
    <Dialog onClose={onClose}>
      <div className="dialog-content">
        <div className="dialog-title">WinAIUsage</div>
        <div className="dialog-caption">{version}</div>
        <div className="dialog-text" style={{ marginTop: 12 }}>{t("developedBy")} @AxelDreemurr</div>
      </div>
      <div className="dialog-footer">
        <button className="btn accent" onClick={() => invoke("open_url", { url: "https://axeldreemurr.cl" })}>
          {t("visitWebsite")}
        </button>
        <button className="btn" onClick={onClose}>{t("close")}</button>
      </div>
    </Dialog>
  );
}

function ToggleRow({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <label className="settings-row">
      <span>{label}</span>
      <span className="toggle">
        <input type="checkbox" role="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
        <span className="toggle-track" />
      </span>
    </label>
  );
}

function SettingsModal({ settings, onClose, onSave }: { settings: AppSettings, onClose: () => void, onSave: (s: AppSettings) => void }) {
  const [s, setS] = useState(settings);

  return (
    <Dialog onClose={onClose}>
      <div className="dialog-content">
        <div className="dialog-title">{t("settings")}</div>

        <div className="dialog-section-title">{t("aiProviders")}</div>
        <ToggleRow label="Claude Code" checked={s.enable_claude} onChange={(v) => setS({ ...s, enable_claude: v })} />
        <ToggleRow label="Codex" checked={s.enable_codex} onChange={(v) => setS({ ...s, enable_codex: v })} />
        <ToggleRow label="Antigravity" checked={s.enable_antigravity} onChange={(v) => setS({ ...s, enable_antigravity: v })} />

        <div className="dialog-section-title">{t("appSettings")}</div>
        <ToggleRow label={t("notifications")} checked={s.enable_notifications} onChange={(v) => setS({ ...s, enable_notifications: v })} />
        <ToggleRow label={t("openOnStartup")} checked={s.open_on_startup} onChange={(v) => setS({ ...s, open_on_startup: v })} />
      </div>
      <div className="dialog-footer">
        <button className="btn accent" onClick={() => { onSave(s); onClose(); }}>{t("save")}</button>
        <button className="btn" onClick={onClose}>{t("cancel")}</button>
      </div>
    </Dialog>
  );
}

// ── App ───────────────────────────────────────────────────────────────────────

function App() {
  const [data, setData] = useState<AllUsageData | null>(null);
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [appVersion, setAppVersion] = useState<string>("...");
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [, setTick] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const [showAbout, setShowAbout] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [isPinned, setIsPinned] = useState(false);
  const unlistenRef = useRef<(() => void) | null>(null);

  const headerRef = useRef<HTMLDivElement>(null);
  const bodyRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const footerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(console.error);
    invoke<AppSettings>("get_settings").then(setSettings).catch(console.error);

    const lang = navigator.language.toLowerCase().startsWith("es") ? "es" : "en";
    currentLang = lang;
    setTick((t) => t + 1);
    invoke("set_lang", { lang }).catch(console.error);

    applyAccentColors();
    window.addEventListener("focus", applyAccentColors);

    // Load cached data immediately
    invoke<AllUsageData>("get_all_usage_data").then((d) => {
      setData(d);
      if (d.claude_code.is_available || d.antigravity.is_available) {
        setLastUpdated(new Date());
      }
    });

    // Subscribe to background updates
    listen<AllUsageData>("usage-updated", (event) => {
      setData(event.payload);
      setLastUpdated(new Date());
    }).then((unlisten) => {
      unlistenRef.current = unlisten;
    });

    // Tick every 60s to refresh "Hace X min" text
    const timer = setInterval(() => setTick((t) => t + 1), 60_000);

    return () => {
      unlistenRef.current?.();
      clearInterval(timer);
      window.removeEventListener("focus", applyAccentColors);
    };
  }, []);

  // Size the window to its content, like a Windows flyout (no empty space below the cards).
  const fitWindow = useCallback(() => {
    const header = headerRef.current;
    const body = bodyRef.current;
    const footer = footerRef.current;
    if (!header || !body || !footer) return;
    let height = header.offsetHeight + body.scrollHeight + footer.offsetHeight;
    const dialog = document.querySelector<HTMLElement>(".dialog");
    if (dialog) height = Math.max(height, dialog.offsetHeight + 24);
    invoke("set_content_height", { height }).catch(() => {});
  }, []);

  useLayoutEffect(() => {
    const observer = new ResizeObserver(fitWindow);
    if (contentRef.current) observer.observe(contentRef.current);
    const dialog = document.querySelector<HTMLElement>(".dialog");
    if (dialog) observer.observe(dialog);
    fitWindow();
    return () => observer.disconnect();
  }, [fitWindow, showAbout, showSettings]);

  function handleClose() {
    invoke("hide_window").catch(() => {});
  }

  function handleRefresh() {
    if (refreshing) return;
    setRefreshing(true);
    invoke("refresh_now").finally(() => setRefreshing(false));
  }

  function handlePin() {
    invoke<boolean>("toggle_pin").then(setIsPinned);
  }

  function handleSaveSettings(newSettings: AppSettings) {
    invoke("save_settings", { settings: newSettings }).then(() => {
      setSettings(newSettings);
      handleRefresh(); // Refresh data with new settings
    }).catch(console.error);
  }

  const noProviders = !settings.enable_claude && !settings.enable_codex && !settings.enable_antigravity;

  return (
    <div className="popup">
      <div className="popup-header" ref={headerRef}>
        <span className="popup-title">AI Usage</span>
        <div className="header-actions">
          <button
            className={`btn-subtle ${isPinned ? "checked" : ""}`}
            onClick={handlePin}
            title={t("pin")}
            aria-label={t("pin")}
            aria-pressed={isPinned}
          >
            <Icon glyph={isPinned ? Glyph.unpin : Glyph.pin} />
          </button>
          <button className="btn-subtle" onClick={() => setShowSettings(true)} title={t("settings")} aria-label={t("settings")}>
            <Icon glyph={Glyph.settings} />
          </button>
          <button className="btn-subtle" onClick={handleRefresh} disabled={refreshing} title={t("refreshNow")} aria-label={t("refreshNow")}>
            <Icon glyph={Glyph.refresh} className={refreshing ? "spinning" : ""} />
          </button>
          <button className="btn-subtle" onClick={handleClose} title={t("close")} aria-label={t("close")}>
            <Icon glyph={Glyph.close} />
          </button>
        </div>
      </div>

      <div className="popup-body" ref={bodyRef}>
        <div className="popup-content" ref={contentRef}>
          {data ? (
            noProviders ? (
              <div className="card-empty">{t("noProviders")}</div>
            ) : (
              <>
                {settings.enable_claude && <ClaudeCard data={data.claude_code} refreshing={refreshing} />}
                {settings.enable_codex && <CodexCard data={data.codex} refreshing={refreshing} />}
                {settings.enable_antigravity && <AntigravityCard data={data.antigravity} refreshing={refreshing} />}
              </>
            )
          ) : (
            <div className="card-empty">{t("loading")}</div>
          )}
        </div>
      </div>

      <div className="popup-footer" ref={footerRef}>
        <button className="footer-link" onClick={() => setShowAbout(true)} title={t("about")}>
          WinAIUsage {appVersion}
        </button>
        {lastUpdated && (
          <span className="footer-updated">{t("updated")} {formatLastUpdated(lastUpdated)}</span>
        )}
      </div>

      {showAbout && <AboutModal onClose={() => setShowAbout(false)} version={appVersion} />}
      {showSettings && <SettingsModal settings={settings} onClose={() => setShowSettings(false)} onSave={handleSaveSettings} />}
    </div>
  );
}

export default App;
