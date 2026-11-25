// Dev-only IPC mock so the UI can run in a plain browser (`npm run dev`) with dummy data.
//
// Screenshot mode (used by marketing/store-shots.html): add `?shot` to the URL, plus
// optional `lang=es|en`, `theme=dark|light`, `accent=blue|purple|teal|orange`,
// `data=default|alert`, `providers=claude,codex,antigravity` and `frame=<id>`. The page then renders without the preview
// wallpaper and reports its height to the parent instead of resizing itself.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

const inMs = (ms: number) => new Date(Date.now() + ms).toISOString();
const H = 3_600_000;

const params = new URLSearchParams(location.search);
const shot = params.has("shot");
const es = (params.get("lang") ?? (navigator.language.startsWith("es") ? "es" : "en")) === "es";
const alert = params.get("data") === "alert";

const usage = {
  claude_code: {
    is_available: true,
    status_line: es ? "Max 5x · Sesión activa" : "Max 5x · Active session",
    five_hour: { utilization: alert ? 86 : 42, resets_at: inMs(2.6 * H) },
    seven_day: { utilization: alert ? 64 : 37, resets_at: inMs(3.2 * 24 * H) },
    today: { tokens: 17_190_306, cost: 7.41, unpriced_tokens: 0, unpriced_models: [] },
    error: null,
  },
  codex: {
    is_available: true,
    status_line: es ? "Plus · Sesión activa" : "Plus · Active session",
    plan_type: "plus",
    periods: [
      { label_key: "codexSession", utilization: alert ? 97 : 18, resets_at: inMs(1.1 * H) },
      { label_key: "codexWeek", utilization: alert ? 88 : 54, resets_at: inMs(1.4 * 24 * H) },
    ],
    today: { tokens: 9_482_113, cost: 3.12, unpriced_tokens: 0, unpriced_models: [] },
    error: null,
  },
  antigravity: {
    is_available: true,
    plan_name: "Pro",
    status_line: es ? "Pro · 3 modelos disponibles" : "Pro · 3 models available",
    today: { tokens: 12_306_975, cost: 2.36, unpriced_tokens: 0, unpriced_models: [] },
    models: [
      { label: "Gemini 3.8 Flash", remaining_fraction: 0.65, percent_used: 35, reset_time: inMs(4.1 * H) },
      { label: "Gemini 3.1 Pro", remaining_fraction: 0.4, percent_used: 60, reset_time: inMs(2.3 * H) },
      { label: "Claude Sonnet 5.5", remaining_fraction: 0.9, percent_used: 10, reset_time: inMs(4.6 * H) },
    ],
    error: null,
  },
};

// Windows accent palettes (UISettings: accent, light1-3, dark1-3).
const ACCENTS: Record<string, Record<string, string>> = {
  purple: { accent: "#744DA9", light1: "#9A73CF", light2: "#B999E6", light3: "#D6C0F5", dark1: "#5B3A8C", dark2: "#46296F", dark3: "#311B52" },
  teal: { accent: "#00897B", light1: "#26A69A", light2: "#4DD0C1", light3: "#8EF0E4", dark1: "#00695C", dark2: "#004D40", dark3: "#00332B" },
  orange: { accent: "#CA5010", light1: "#E36E2C", light2: "#F7955A", light3: "#FFBD8E", dark1: "#A33F0B", dark2: "#7D2F07", dark3: "#5A2004" },
};

// `providers=claude,codex` limits the cards shown (screenshot close-ups).
const providers = params.get("providers")?.split(",");
const shown = (id: string) => !providers || providers.includes(id);

let settings = {
  enable_claude: shown("claude"),
  enable_codex: shown("codex"),
  enable_antigravity: shown("antigravity"),
  enable_notifications: true,
  open_on_startup: true,
};
let pinned = false;

/** Forces the light or dark token set regardless of the browser's color scheme. */
function forceTheme(theme: string) {
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      continue;
    }
    for (let i = rules.length - 1; i >= 0; i--) {
      const rule = rules[i];
      if (rule instanceof CSSMediaRule && rule.conditionText.includes("prefers-color-scheme: light")) {
        const inner = Array.from(rule.cssRules).map((r) => r.cssText);
        sheet.deleteRule(i);
        if (theme === "light") inner.forEach((css) => sheet.insertRule(css, sheet.cssRules.length));
      }
    }
  }
}

export function installDevMock() {
  if (shot) {
    Object.defineProperty(navigator, "language", { get: () => (es ? "es-CL" : "en-US") });
    document.documentElement.style.background = "transparent";
    const theme = params.get("theme");
    if (theme) forceTheme(theme);
  } else {
    // Emulate the Acrylic window in the browser: a wallpaper behind a window-sized body.
    document.documentElement.classList.add("dev-preview");
  }

  mockWindows("main");
  mockIPC(
    (cmd, args) => {
      switch (cmd) {
        case "plugin:app|version":
          return shot ? "2.0.0" : "2.0.0-dev";
        case "get_all_usage_data":
          return usage;
        case "get_settings":
          return settings;
        case "save_settings":
          settings = (args as { settings: typeof settings }).settings;
          return settings;
        case "toggle_pin":
          pinned = !pinned;
          return pinned;
        case "set_content_height": {
          const height = Math.min(650, Math.ceil((args as { height: number }).height));
          if (shot) {
            parent.postMessage({ type: "winaiusage-height", frame: params.get("frame"), height }, "*");
          } else {
            document.body.style.height = `${height}px`;
          }
          return null;
        }
        case "get_accent_colors":
          return ACCENTS[params.get("accent") ?? ""] ?? null; // null keeps the default Windows blue
        case "refresh_now":
          return new Promise((r) => setTimeout(() => r(null), 600));
        default:
          console.debug("[devMock] unhandled", cmd, args);
          return null;
      }
    },
    { shouldMockEvents: true },
  );
}
