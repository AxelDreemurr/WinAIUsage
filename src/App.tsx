import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";

// ── Types ─────────────────────────────────────────────────────────────────────

interface UsagePeriod {
  utilization: number;
  resets_at: string;
}

interface ClaudeCodeData {
  is_available: boolean;
  status_line: string;
  five_hour: UsagePeriod | null;
  seven_day: UsagePeriod | null;
  daily_tokens: number;
  daily_cost: number;
  error: string | null;
  is_peak_hours: boolean;
  peak_status: string;
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
    resetsIn: "se reinicia en",
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
  },
  en: {
    loading: "Loading...",
    refreshNow: "Refresh now",
    developedBy: "Developed by",
    visitWebsite: "Visit website",
    resetsIn: "resets in",
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
    openOnStartup: "Open on startup",
    save: "Save",
    aiProviders: "AI Providers",
    appSettings: "App Settings",
  }
};

function t(key: keyof typeof DICT.en): string {
  return DICT[currentLang][key];
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function utilizationColor(pct: number): string {
  if (pct >= 85) return "#F44336";
  if (pct >= 60) return "#FF9800";
  return "#4CAF50";
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

// ── Shared components ─────────────────────────────────────────────────────────

function ProgressBar({ pct, label, resetAt }: { pct: number; label: string; resetAt: string }) {
  const clamped = Math.min(100, Math.max(0, pct));
  const color = utilizationColor(clamped);
  return (
    <div className="progress-container">
      <div className="progress-label-row">
        <span>{label}</span>
        <span style={{ color }}>
          {clamped.toFixed(0)}%{resetAt ? ` · ${t("resetsIn")} ${formatResetsAt(resetAt)}` : ""}
        </span>
      </div>
      <div className="progress-track">
        <div className="progress-fill" style={{ width: `${clamped}%`, background: color }} />
      </div>
    </div>
  );
}

function PeakBadge({ status }: { status: string }) {
  let bg: string;
  let color: string;
  let icon: string;

  if (status === "Peak") {
    bg = "#3D1F1F";
    color = "#FF6B6B";
    icon = "⚡";
  } else if (status === "Off-Peak (weekend)") {
    bg = "#1A2A3D";
    color = "#64B5F6";
    icon = "✓";
  } else {
    bg = "#1A3D1F";
    color = "#4CAF50";
    icon = "✓";
  }

  return (
    <span
      style={{
        background: bg,
        color,
        borderRadius: "4px",
        padding: "2px 7px",
        fontSize: "11px",
        fontWeight: 500,
        marginLeft: "6px",
        whiteSpace: "nowrap",
        flexShrink: 0,
      }}
    >
      {icon} {status}
    </span>
  );
}

function CardShell({
  name,
  active,
  badge,
  refreshing,
  children,
}: {
  name: string;
  active: boolean;
  badge?: React.ReactNode;
  refreshing?: boolean;
  children: React.ReactNode;
}) {
  return (
    <div className="provider-card">
      <div className="card-header">
        <span className="provider-name">{name}</span>
        {badge}
        {refreshing
          ? <span className="spinner" style={{ marginLeft: "auto", flexShrink: 0 }} />
          : <span className={`status-dot ${active ? "active" : "inactive"}`} style={{ marginLeft: "auto" }} />}
      </div>
      {children}
    </div>
  );
}

// ── Provider cards (presentational) ──────────────────────────────────────────

function ClaudeCard({ data, refreshing }: { data: ClaudeCodeData; refreshing: boolean }) {
  return (
    <CardShell
      name="Claude Code"
      active={data.is_available}
      badge={<PeakBadge status={data.peak_status} />}
      refreshing={refreshing}
    >
      <div className="card-status">{data.status_line || data.error || t("notAvailable")}</div>
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
    <CardShell name="Antigravity" active={data.is_available} refreshing={refreshing}>
      <div className="card-status">{data.status_line || data.error || t("notAvailable")}</div>
      {data.is_available && data.models.map((m) => (
        <ProgressBar key={m.label} pct={m.percent_used} label={m.label} resetAt={m.reset_time} />
      ))}
    </CardShell>
  );
}

function CodexCard({ data, refreshing }: { data: CodexData; refreshing: boolean }) {
  return (
    <CardShell name="Codex" active={data.is_available} refreshing={refreshing}>
      <div className="card-status">{data.status_line || data.error || t("notAvailable")}</div>
      {data.is_available && data.periods && data.periods.map((p, i) => (
        <ProgressBar key={i} pct={p.utilization} label={t(p.label_key)} resetAt={p.resets_at} />
      ))}
    </CardShell>
  );
}

// ── About modal ───────────────────────────────────────────────────────────────

function AboutModal({ onClose }: { onClose: () => void }) {
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-card" onClick={(e) => e.stopPropagation()}>
        <button className="modal-close" onClick={onClose}>✕</button>
        <div className="modal-title">WinAIUsage</div>
        <div className="modal-version">v1.0.0</div>
        <div className="modal-divider" />
        <div className="modal-author">{t("developedBy")} @AxelDreemurr</div>
        <button
          className="modal-btn-link"
          onClick={() => invoke("open_url", { url: "https://axeldreemurr.cl" })}
        >
          {t("visitWebsite")}
        </button>
      </div>
    </div>
  );
}

// ── Settings modal ────────────────────────────────────────────────────────────

function SettingsModal({ settings, onClose, onSave }: { settings: AppSettings, onClose: () => void, onSave: (s: AppSettings) => void }) {
  const [s, setS] = useState(settings);

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-card" onClick={(e) => e.stopPropagation()}>
        <button className="modal-close" onClick={onClose}>✕</button>
        <div className="modal-title">{t("settings")}</div>
        <div className="modal-divider" />
        
        <div className="modal-section-title">{t("aiProviders")}</div>
        <div className="settings-row">
          <label><input type="checkbox" checked={s.enable_claude} onChange={e => setS({...s, enable_claude: e.target.checked})} /> Claude Code</label>
        </div>
        <div className="settings-row">
          <label><input type="checkbox" checked={s.enable_codex} onChange={e => setS({...s, enable_codex: e.target.checked})} /> Codex</label>
        </div>
        <div className="settings-row">
          <label><input type="checkbox" checked={s.enable_antigravity} onChange={e => setS({...s, enable_antigravity: e.target.checked})} /> Antigravity</label>
        </div>

        <div className="modal-divider" />
        
        <div className="modal-section-title">{t("appSettings")}</div>
        <div className="settings-row">
          <label><input type="checkbox" checked={s.enable_notifications} onChange={e => setS({...s, enable_notifications: e.target.checked})} /> {t("notifications")}</label>
        </div>
        <div className="settings-row">
          <label><input type="checkbox" checked={s.open_on_startup} onChange={e => setS({...s, open_on_startup: e.target.checked})} /> {t("openOnStartup")}</label>
        </div>

        <button className="modal-btn" onClick={() => { onSave(s); onClose(); }}>{t("save")}</button>
      </div>
    </div>
  );
}

// ── App ───────────────────────────────────────────────────────────────────────

function App() {
  const [data, setData] = useState<AllUsageData | null>(null);
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [, setTick] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const [showAbout, setShowAbout] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [isPinned, setIsPinned] = useState(false);
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    invoke<AppSettings>("get_settings").then(setSettings).catch(console.error);

    const lang = navigator.language.toLowerCase().startsWith("es") ? "es" : "en";
    currentLang = lang;
    setTick((t) => t + 1);
    invoke("set_lang", { lang }).catch(console.error);

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
    };
  }, []);

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

  return (
    <div className="popup">
      <div className="popup-header">
        <span>AI Usage</span>
        <div className="header-actions">
          <button
            className="btn-icon"
            onClick={handlePin}
            title={t("pin")}
            style={{ opacity: isPinned ? 1 : 0.4, display: 'flex', alignItems: 'center' }}
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill={isPinned ? "currentColor" : "none"} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <line x1="12" y1="17" x2="12" y2="22"></line>
              <path d="M5 17h14v-1.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V6h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1v4.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24Z"></path>
            </svg>
          </button>
          <button
            className="btn-icon"
            onClick={() => setShowSettings(true)}
            title={t("settings")}
            style={{ display: 'flex', alignItems: 'center' }}
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <circle cx="12" cy="12" r="3"></circle>
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"></path>
            </svg>
          </button>
          <button
            className="btn-icon"
            onClick={handleRefresh}
            title={t("refreshNow")}
            style={{ opacity: refreshing ? 0.4 : 1 }}
          >
            ↻
          </button>
          <button className="btn-close" onClick={handleClose}>✕</button>
        </div>
      </div>

      <div className="popup-body">
        {data ? (
          <>
            {settings.enable_claude && <ClaudeCard data={data.claude_code} refreshing={refreshing} />}
            {settings.enable_codex && <CodexCard data={data.codex} refreshing={refreshing} />}
            {settings.enable_antigravity && <AntigravityCard data={data.antigravity} refreshing={refreshing} />}
          </>
        ) : (
          <div className="card-status" style={{ padding: "8px 0" }}>{t("loading")}</div>
        )}
      </div>

      {showAbout && <AboutModal onClose={() => setShowAbout(false)} />}
      {showSettings && <SettingsModal settings={settings} onClose={() => setShowSettings(false)} onSave={handleSaveSettings} />}

      <div className="popup-footer">
        <span className="footer-app-name" onClick={() => setShowAbout(true)}>
          WinAIUsage v1.0.0
        </span>
        {lastUpdated && (
          <>
            <span className="footer-sep"> · </span>
            <span>{t("updated")} {formatLastUpdated(lastUpdated)}</span>
          </>
        )}
      </div>
    </div>
  );
}

export default App;
