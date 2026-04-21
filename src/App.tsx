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

interface AllUsageData {
  claude_code: ClaudeCodeData;
  antigravity: AntigravityData;
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
  if (diff <= 0) return "pronto";
  const h = Math.floor(diff / 3_600_000);
  const m = Math.floor((diff % 3_600_000) / 60_000);
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

function formatLastUpdated(date: Date | null): string {
  if (!date) return "";
  const mins = Math.floor((Date.now() - date.getTime()) / 60_000);
  if (mins === 0) return "ahora mismo";
  return `hace ${mins} min`;
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
          {clamped.toFixed(0)}% · se reinicia en {formatResetsAt(resetAt)}
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
      <div className="card-status">{data.status_line || data.error || "No disponible"}</div>
      {data.is_available && data.five_hour && (
        <ProgressBar pct={data.five_hour.utilization} label="Sesión (5h)" resetAt={data.five_hour.resets_at} />
      )}
      {data.is_available && data.seven_day && (
        <ProgressBar pct={data.seven_day.utilization} label="Semana" resetAt={data.seven_day.resets_at} />
      )}
    </CardShell>
  );
}

function AntigravityCard({ data, refreshing }: { data: AntigravityData; refreshing: boolean }) {
  return (
    <CardShell name="Antigravity" active={data.is_available} refreshing={refreshing}>
      <div className="card-status">{data.status_line || data.error || "No disponible"}</div>
      {data.is_available && data.models.map((m) => (
        <ProgressBar key={m.label} pct={m.percent_used} label={m.label} resetAt={m.reset_time} />
      ))}
    </CardShell>
  );
}

function CodexCard({ refreshing }: { refreshing: boolean }) {
  return (
    <CardShell name="Codex" active={false} refreshing={refreshing}>
      <div className="card-status">Sin sesión activa</div>
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
        <div className="modal-version">v0.1.1</div>
        <div className="modal-divider" />
        <div className="modal-author">Desarrollado por @AxelDreemurr</div>
        <button
          className="modal-btn-link"
          onClick={() => invoke("open_url", { url: "https://axeldreemurr.cl" })}
        >
          Visitar sitio web
        </button>
      </div>
    </div>
  );
}

// ── App ───────────────────────────────────────────────────────────────────────

function App() {
  const [data, setData] = useState<AllUsageData | null>(null);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [, setTick] = useState(0);
  const [refreshing, setRefreshing] = useState(false);
  const [showAbout, setShowAbout] = useState(false);
  const unlistenRef = useRef<(() => void) | null>(null);

  useEffect(() => {
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

  return (
    <div className="popup">
      <div className="popup-header">
        <span>AI Usage</span>
        <div className="header-actions">
          <button
            className="btn-icon"
            onClick={handleRefresh}
            title="Actualizar ahora"
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
            <ClaudeCard data={data.claude_code} refreshing={refreshing} />
            <CodexCard refreshing={refreshing} />
            <AntigravityCard data={data.antigravity} refreshing={refreshing} />
          </>
        ) : (
          <div className="card-status" style={{ padding: "8px 0" }}>Cargando...</div>
        )}
      </div>

      {showAbout && <AboutModal onClose={() => setShowAbout(false)} />}

      <div className="popup-footer">
        <span className="footer-app-name" onClick={() => setShowAbout(true)}>
          WinAIUsage v0.1.1
        </span>
        {lastUpdated && (
          <>
            <span className="footer-sep"> · </span>
            <span>Actualizado {formatLastUpdated(lastUpdated)}</span>
          </>
        )}
      </div>
    </div>
  );
}

export default App;
