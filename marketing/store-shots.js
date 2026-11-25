// Microsoft Store screenshot composer. Each slide places the real app UI (index.html in
// screenshot mode, see src/devMock.ts) over a styled Windows 11 desktop.

const params = new URLSearchParams(location.search);
const slide = Number(params.get("slide") ?? 1);
const lang = params.get("lang") === "en" ? "en" : "es";
const stage = document.getElementById("stage");
document.documentElement.lang = lang;

const ICON = "/src-tauri/icons/icon.png";
const PROVIDER_ICONS = {
  claude: "/src/assets/providers/claude.svg",
  codex: "/src/assets/providers/codex.svg",
  antigravity: "/src/assets/providers/antigravity.svg",
};

const T = {
  es: {
    s1: {
      title: "Todo tu uso de IA,\nde un vistazo.",
      sub: "Cuotas, tokens y costo del día de tus herramientas de IA, a un clic desde la bandeja de Windows.",
    },
    s2: {
      title: "Sabe cuánto gastas hoy.",
      sub: "Tokens reales leídos de tus sesiones locales y su costo equivalente a precios de API.",
      callouts: [
        ["Tu plan, detectado", "Sin configurar nada"],
        ["Tokens y costo de hoy", "Calculados con los precios oficiales"],
        ["Cuándo se reinicia", "Cada ventana de cuota"],
        ["Cuota usada", "Sesión de 5 h y semanal"],
      ],
    },
    s3: {
      title: "Anticípate a\ntus límites.",
      sub: "Las barras cambian de color cerca del límite y Windows te avisa al 80 % y al 99 %.",
      toasts: [
        ["⚠️ Codex · Sesión", "97% de cuota utilizada"],
        ["⚠️ Claude Code · Sesión (5h)", "86% de cuota utilizada"],
      ],
    },
    s4: {
      title: "Hecho para Windows 11.",
      sub: "Diseño Fluent con Acrylic, modo claro y oscuro, y el color de acento de tu sistema.",
    },
    s5: {
      title: "Siempre a un clic.",
      sub: "Vive en la bandeja del sistema, se actualiza en segundo plano y arranca con Windows.",
      clock: ["12:45", "11-10-2026"],
    },
    s6: {
      title: "Tú decides qué\nmonitorear.",
      sub: "Activa solo las herramientas que usas, las notificaciones y el inicio automático.",
    },
    poster: { tagline: "Todo tu uso de IA,\nde un vistazo." },
  },
  en: {
    s1: {
      title: "All your AI usage,\nat a glance.",
      sub: "Quotas, tokens and today's cost for your AI coding tools, one click away in the Windows tray.",
    },
    s2: {
      title: "Know what you spend today.",
      sub: "Real tokens read from your local sessions, priced at API-equivalent rates.",
      callouts: [
        ["Your plan, detected", "Nothing to set up"],
        ["Today's tokens & cost", "Using official model prices"],
        ["When it resets", "For every quota window"],
        ["Quota used", "5-hour session and weekly"],
      ],
    },
    s3: {
      title: "Stay ahead of\nyour limits.",
      sub: "Bars change color near the limit, and Windows notifies you at 80% and 99%.",
      toasts: [
        ["⚠️ Codex · Session", "97% quota used"],
        ["⚠️ Claude Code · Session (5h)", "86% quota used"],
      ],
    },
    s4: {
      title: "Made for Windows 11.",
      sub: "Fluent design with Acrylic, light and dark mode, and your system accent color.",
    },
    s5: {
      title: "Always one click away.",
      sub: "Lives in the system tray, refreshes in the background and starts with Windows.",
      clock: ["12:45 PM", "10/11/2026"],
    },
    s6: {
      title: "You choose\nwhat to track.",
      sub: "Turn on only the tools you use, plus notifications and launch at startup.",
    },
    poster: { tagline: "All your AI usage,\nat a glance." },
  },
}[lang];

// ── Helpers ───────────────────────────────────────────────────────────────────

function el(tag, attrs = {}, children = []) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "style") Object.assign(node.style, v);
    else if (k === "text") node.textContent = v;
    else node.setAttribute(k, v);
  }
  for (const c of [].concat(children)) node.append(c);
  return node;
}

async function inlineSvg(url) {
  const span = el("span", { class: "logo" });
  span.innerHTML = await (await fetch(url)).text();
  return span;
}

function background() {
  stage.append(el("div", { class: "wallpaper" }), el("div", { class: "grain" }));
}

function copy({ title, sub, center = false, brand = false, width }) {
  const box = el("div", { class: `copy${center ? " center" : ""}` });
  if (width) box.style.width = `${width}px`; // keep clear of the window beside it
  if (brand) {
    box.append(el("div", { class: "brand" }, [el("img", { src: ICON, alt: "" }), "WinAIUsage"]), el("br"));
  }
  box.append(el("h1", { text: title }), el("p", { class: "sub", text: sub }));
  stage.append(box);
  return box;
}

const pending = new Set();
const frames = new Map();

/**
 * Places the real app UI. `x`/`y` position the window's left edge and its top,
 * center or bottom (per `anchor`); `s` is the scale of the 380px-wide popup.
 */
function appWindow({ x, y, s, anchor = "top", theme = "dark", accent, data, providers, label, onReady, onLayout }) {
  const id = `f${frames.size}`;
  const query = new URLSearchParams({ shot: "", lang, theme, frame: id });
  if (accent) query.set("accent", accent);
  if (data) query.set("data", data);
  if (providers) query.set("providers", providers);

  const iframe = el("iframe", { src: `/index.html?${query}`, style: { colorScheme: theme } });
  const win = el("div", { class: `window ${theme}`, style: { "--s": s, left: `${x}px`, width: `${380 * s}px` } }, iframe);
  win.style.setProperty("--s", s);
  stage.append(win);

  let labelEl;
  if (label) {
    labelEl = el("div", { class: "window-label", text: label, style: { left: `${x}px`, width: `${380 * s}px` } });
    stage.append(labelEl);
  }

  pending.add(id);
  let settle;
  frames.set(id, (height) => {
    iframe.style.height = `${height}px`;
    const h = height * s;
    win.style.height = `${h}px`;
    const top = anchor === "bottom" ? y - h : anchor === "center" ? y - h / 2 : y;
    win.style.top = `${top}px`;
    if (labelEl) labelEl.style.top = `${top + h + 26}px`;
    clearTimeout(settle);
    settle = setTimeout(async () => {
      // Heights reported while "Loading..." is shown don't count; the cards trigger another.
      if (!iframe.contentDocument?.querySelector(".provider-card")) return;
      onLayout?.({ iframe, win, s, top });
      if (onReady) {
        const next = onReady;
        onReady = null; // run once; it may trigger another resize
        await next({ iframe, win, s, top });
        return;
      }
      pending.delete(id);
      if (pending.size === 0) document.body.dataset.ready = "1";
    }, 600);
  });
  return { iframe, win };
}

window.addEventListener("message", (e) => {
  if (e.data?.type === "winaiusage-height") frames.get(e.data.frame)?.(e.data.height);
});

/** Element rect inside the iframe, in stage coordinates. */
function rectIn({ iframe, win, s }, selector, index = 0) {
  const target = iframe.contentDocument.querySelectorAll(selector)[index];
  const r = target.getBoundingClientRect();
  const left = win.offsetLeft;
  const top = win.offsetTop;
  return { x: left + r.left * s, y: top + r.top * s, w: r.width * s, h: r.height * s };
}

function callouts(items) {
  document.querySelector(".callouts")?.remove();
  const layer = el("div", { class: "callouts" });
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  layer.append(svg);
  for (const { label, detail, side, target, at } of items) {
    // Left callouts point at the text's left edge; right ones at its top center.
    const ty = side === "left" ? target.y + target.h / 2 : target.y - 12;
    const tx = side === "left" ? target.x - 18 : target.x + target.w / 2;
    const cy = at ?? ty;
    const box = el("div", { class: "callout", style: { top: `${cy}px` } }, [el("b", { text: label }), el("span", { text: detail })]);
    const boxX = side === "left" ? 36 : 1920 - 36 - 440;
    box.style.left = `${boxX}px`;
    layer.append(box);
    const line = document.createElementNS(svg.namespaceURI, "line");
    line.setAttribute("x1", side === "left" ? boxX + 440 + 10 : boxX - 10);
    line.setAttribute("y1", cy);
    line.setAttribute("x2", tx);
    line.setAttribute("y2", ty);
    const dot = document.createElementNS(svg.namespaceURI, "circle");
    dot.setAttribute("cx", tx);
    dot.setAttribute("cy", ty);
    dot.setAttribute("r", 7);
    svg.append(line, dot);
  }
  stage.append(layer);
}

// ── Slides ────────────────────────────────────────────────────────────────────

const slides = {
  // Hero: headline + the full popup.
  async 1() {
    background();
    const box = copy({ ...T.s1, brand: true });
    const pills = el("div", { class: "pills" });
    for (const [id, name] of [["claude", "Claude Code"], ["codex", "Codex"], ["antigravity", "Antigravity"]]) {
      pills.append(el("span", { class: `pill ${id}` }, [await inlineSvg(PROVIDER_ICONS[id]), name]));
    }
    box.append(pills);
    for (const p of pills.querySelectorAll(".pill.claude .logo")) p.style.color = "#D97757";
    appWindow({ x: 1200, y: 540, s: 1.68, anchor: "center" });
  },

  // Close-up of one card with annotations.
  async 2() {
    background();
    copy({ ...T.s2, center: true });
    const [plan, today, reset, quota] = T.s2.callouts;
    appWindow({
      x: 960 - 190 * 2.35,
      y: 1040,
      s: 2.35,
      anchor: "bottom",
      providers: "claude",
      onLayout(ctx) {
        const planRect = rectIn(ctx, ".provider-card .card-status", 0);
        const todayRect = rectIn(ctx, ".provider-card .card-status", 1);
        const resetRect = rectIn(ctx, ".progress-reset", 0);
        const quotaRect = rectIn(ctx, ".progress-value", 1);
        callouts([
          { label: plan[0], detail: plan[1], side: "left", target: planRect, at: planRect.y - 50 },
          { label: today[0], detail: today[1], side: "left", target: todayRect, at: todayRect.y + 110 },
          { label: reset[0], detail: reset[1], side: "right", target: resetRect, at: resetRect.y - 70 },
          { label: quota[0], detail: quota[1], side: "right", target: quotaRect, at: quotaRect.y + 90 },
        ]);
      },
    });
  },

  // Near-limit colors + native toasts.
  async 3() {
    background();
    copy({ ...T.s3, width: 900 });
    appWindow({ x: 1080, y: 50, s: 1.55, data: "alert", providers: "claude,codex" });
    const toasts = el("div", { class: "toasts" });
    toasts.style.setProperty("--s", 1.4);
    for (const [title, body] of T.s3.toasts.slice(0, 1)) {
      toasts.append(
        el("div", { class: "toast" }, [
          el("div", { class: "toast-head" }, [el("img", { src: ICON, alt: "" }), "WinAIUsage", el("span", { class: "ctl", text: "  " })]),
          el("div", { class: "toast-title", text: title }),
          el("div", { class: "toast-body", text: body }),
        ]),
      );
    }
    stage.append(toasts);
    // Bottom-right, where Windows shows toasts.
    toasts.style.right = "36px";
    toasts.style.bottom = "36px";
  },

  // Light, dark and accent colors side by side.
  async 4() {
    background();
    copy({ ...T.s4, center: true });
    const s = 1.24;
    const w = 380 * s;
    const gap = 56;
    const left = (1920 - (3 * w + 2 * gap)) / 2;
    appWindow({ x: left, y: 1060, s, anchor: "bottom", theme: "dark" });
    appWindow({ x: left + w + gap, y: 1060, s, anchor: "bottom", theme: "light", accent: "purple" });
    appWindow({ x: left + 2 * (w + gap), y: 1060, s, anchor: "bottom", theme: "dark", accent: "teal" });
  },

  // Full desktop with the popup above the tray.
  async 5() {
    background();
    copy(T.s5);
    const tray = el("div", { class: "tray" }, [
      el("span", { text: "" }),
      el("div", { class: "tray-app" }, el("img", { src: ICON, alt: "" })),
      el("span", { text: "" }),
      el("span", { text: "" }),
      el("span", { text: "" }),
      el("span", { class: "clock" }, [T.s5.clock[0], el("br"), T.s5.clock[1]]),
    ]);
    const tile = (bg) => el("div", { class: "tb-btn" }, el("div", { class: "app-tile", style: { background: bg } }));
    const apps = el("div", { class: "apps" }, [
      el("div", { class: "tb-btn" }, el("div", { class: "winlogo" }, [el("i"), el("i"), el("i"), el("i")])),
      el("div", { class: "tb-btn", text: "" }),
      tile("linear-gradient(135deg,#ffd86b,#f2a33a)"),
      tile("linear-gradient(135deg,#3fd0ff,#1e6fe0)"),
      tile("linear-gradient(135deg,#d97757,#b4532f)"),
      tile("linear-gradient(135deg,#2b2b2b,#555)"),
    ]);
    stage.append(el("div", { class: "taskbar" }, [apps, tray]));
    appWindow({ x: 1920 - 12 - 380 * 1.5 - 30, y: 1080 - 60 - 14, s: 1.5, anchor: "bottom" });
  },

  // Settings dialog.
  async 6() {
    background();
    copy(T.s6);
    appWindow({
      x: 1170,
      y: 540,
      s: 1.7,
      anchor: "center",
      onReady({ iframe }) {
        iframe.contentDocument.querySelectorAll(".header-actions button")[1].click();
      },
    });
  },
};

// 9:16 poster art (Store "Arte de póster", 720x1080 exported at 2x = 1440x2160).
slides[7] = async () => {
  stage.classList.add("poster");
  background();
  stage.append(
    el("div", { class: "poster-head" }, [
      el("img", { src: ICON, alt: "" }),
      el("div", { class: "poster-name", text: "WinAIUsage" }),
      el("div", { class: "poster-tagline", text: T.poster.tagline }),
    ]),
  );
  // Bleeds off the bottom edge on purpose.
  appWindow({ x: (720 - 380 * 1.55) / 2, y: 520, s: 1.55 });
};

// 16:9 hero art (Store "Arte de superhéroe"). Must not show the product name: no copy,
// and both windows bleed off the bottom so their "WinAIUsage x.y.z" footer stays hidden.
// The left side stays calm because the Store overlays the title and buttons there.
slides[8] = async () => {
  stage.classList.add("hero-art");
  background();
  appWindow({ x: 850, y: 300, s: 1.36, theme: "light", accent: "purple" });
  appWindow({ x: 1190, y: 92, s: 1.76 });
};

// ── Boot ──────────────────────────────────────────────────────────────────────

await slides[slide]?.();

if (params.has("fit")) {
  document.body.classList.add("fit");
  const fit = () => {
    const k = Math.min(innerWidth / stage.offsetWidth, innerHeight / stage.offsetHeight);
    stage.style.transform = `scale(${k})`;
  };
  addEventListener("resize", fit);
  fit();
}
