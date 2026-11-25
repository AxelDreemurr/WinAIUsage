// Exports the Microsoft Store screenshots and hero art (3840x2160) and 9:16 poster (1440x2160) with headless Edge,
// driven over the DevTools protocol so each capture waits for the page's ready signal.
// Needs the Vite dev server running (`npm run dev`).
// Usage: node marketing/export-store-shots.mjs [name-filter]   e.g. `hero-art` exports only that image.
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const EDGE = "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe";
const BASE = "http://localhost:1420/marketing/store-shots.html";
// [file name, CSS width, CSS height]; exported at 2x.
const SLIDES = [
  ["01-hero", 1920, 1080],
  ["02-today-cost", 1920, 1080],
  ["03-alerts", 1920, 1080],
  ["04-windows-11", 1920, 1080],
  ["05-tray", 1920, 1080],
  ["06-settings", 1920, 1080],
  ["poster-9x16", 720, 1080],
  ["hero-art-16x9", 1920, 1080],
];
const OUT = join(dirname(fileURLToPath(import.meta.url)), "store-screenshots");
const PORT = 9333;
const only = process.argv[2];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const profile = mkdtempSync(join(tmpdir(), "winaiusage-shots-"));
const edge = spawn(EDGE, ["--headless=new", "--hide-scrollbars", `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`, "about:blank"]);

try {
  let pageWs;
  for (let i = 0; i < 50 && !pageWs; i++) {
    await sleep(200);
    try {
      const targets = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      pageWs = targets.find((t) => t.type === "page")?.webSocketDebuggerUrl;
    } catch {}
  }
  if (!pageWs) throw new Error("Edge DevTools endpoint not reachable");

  const ws = new WebSocket(pageWs);
  await new Promise((r) => ws.addEventListener("open", r, { once: true }));
  let nextId = 1;
  const waiting = new Map();
  ws.addEventListener("message", (e) => {
    const msg = JSON.parse(e.data);
    if (msg.id && waiting.has(msg.id)) {
      waiting.get(msg.id)(msg);
      waiting.delete(msg.id);
    }
  });
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = nextId++;
      waiting.set(id, (msg) => (msg.error ? reject(new Error(`${method}: ${msg.error.message}`)) : resolve(msg.result)));
      ws.send(JSON.stringify({ id, method, params }));
    });

  await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "dark" }] });

  for (const lang of ["es", "en"]) {
    mkdirSync(join(OUT, lang), { recursive: true });
    for (const [i, [name, width, height]] of SLIDES.entries()) {
      if (only && !name.includes(only)) continue;
      await send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 2, mobile: false });
      await send("Page.navigate", { url: `${BASE}?slide=${i + 1}&lang=${lang}` });
      let ready = false;
      for (let t = 0; t < 100 && !ready; t++) {
        await sleep(150);
        const { result } = await send("Runtime.evaluate", { expression: "document.body?.dataset.ready === '1'" });
        ready = result.value === true;
      }
      if (!ready) throw new Error(`slide ${i + 1} (${lang}) never became ready`);
      await sleep(800); // let late layout passes (callouts, fonts) settle
      const { data } = await send("Page.captureScreenshot", { format: "png", captureBeyondViewport: false });
      const file = join(OUT, lang, `${name}.png`);
      writeFileSync(file, Buffer.from(data, "base64"));
      console.log(file);
    }
  }
  ws.close();
} finally {
  edge.kill();
  await sleep(500);
  rmSync(profile, { recursive: true, force: true });
}
