// Dev-only IPC mock so the UI can run in a plain browser (`npm run dev`) with dummy data.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

const inMs = (ms: number) => new Date(Date.now() + ms).toISOString();
const H = 3_600_000;

const usage = {
  claude_code: {
    is_available: true,
    status_line: "Pro · Sesión activa",
    five_hour: { utilization: 42, resets_at: inMs(2.6 * H) },
    seven_day: { utilization: 71, resets_at: inMs(3.2 * 24 * H) },
    today: { tokens: 17_190_306, cost: 7.41, unpriced_tokens: 0, unpriced_models: [] },
    error: null,
  },
  codex: {
    is_available: true,
    status_line: "Plus · Sesión activa",
    plan_type: "plus",
    periods: [
      { label_key: "codexSession", utilization: 18, resets_at: inMs(3.9 * H) },
      { label_key: "codexWeek", utilization: 88, resets_at: inMs(1.4 * 24 * H) },
    ],
    today: { tokens: 27_146_758, cost: 6.28, unpriced_tokens: 7_454_908, unpriced_models: ["gpt-daybreak-blue-latest", "codex-auto-review"] },
    error: null,
  },
  antigravity: {
    is_available: true,
    plan_name: "Pro",
    status_line: "Pro · 3 modelos disponibles",
    today: { tokens: 48_306_975, cost: 6.76, unpriced_tokens: 0, unpriced_models: [] },
    models: [
      { label: "Gemini 3 Pro", remaining_fraction: 0.65, percent_used: 35, reset_time: inMs(4.1 * H) },
      { label: "Claude Sonnet", remaining_fraction: 0.1, percent_used: 90, reset_time: inMs(1.2 * H) },
      { label: "GPT-OSS", remaining_fraction: 1, percent_used: 0, reset_time: "" },
    ],
    error: null,
  },
};

let settings = {
  enable_claude: true,
  enable_codex: true,
  enable_antigravity: true,
  enable_notifications: true,
  open_on_startup: false,
};
let pinned = false;

export function installDevMock() {
  // Emulate the Acrylic window in the browser: a wallpaper behind a window-sized body.
  document.documentElement.classList.add("dev-preview");
  mockWindows("main");
  mockIPC(
    (cmd, args) => {
      switch (cmd) {
        case "plugin:app|version":
          return "2.0.0-dev";
        case "get_all_usage_data":
          return usage;
        case "get_settings":
          return settings;
        case "save_settings":
          settings = (args as { settings: typeof settings }).settings;
          return null;
        case "toggle_pin":
          pinned = !pinned;
          return pinned;
        case "set_content_height":
          document.body.style.height = `${Math.min(650, Math.ceil((args as { height: number }).height))}px`;
          return null;
        case "get_accent_colors":
          return null; // keep the default Windows blue
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
