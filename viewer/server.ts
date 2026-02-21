// remote-browser: Chrome headless + CDP screencast viewer (single binary)
//
// Chrome を headless モードで起動し、CDP screencast でブラウザ画面を
// WebSocket 経由でクライアントに配信する。タッチ/キーボード入力も中継する。
//
// Usage:
//   bun run viewer/server.ts              # Chrome を自動起動
//   CDP_PORT=9222 bun run viewer/server.ts # 既存の Chrome に接続

const PUBLIC_DIR = process.env.PUBLIC_DIR || import.meta.dir + "/public";
const CHROME_PROFILE = process.env.CHROME_PROFILE || process.cwd() + "/chrome-profile";

// Chrome バイナリを検出
function findChromeBin(): string {
  if (process.env.CHROME_BIN) return process.env.CHROME_BIN;

  const candidates = [
    "google-chrome-stable",
    "google-chrome",
    "chromium-browser",
    "chromium",
  ];
  for (const bin of candidates) {
    try {
      const result = Bun.spawnSync(["which", bin]);
      if (result.exitCode === 0) return bin;
    } catch {
      // not found
    }
  }
  console.error(
    "ERROR: Chrome/Chromium が見つかりません。以下のいずれかをインストールしてください:\n" +
      "  Arch Linux: pacman -S google-chrome (AUR) or chromium\n" +
      "  Ubuntu/Debian: apt install google-chrome-stable or chromium-browser\n" +
      "  または CHROME_BIN 環境変数でパスを指定してください"
  );
  process.exit(1);
}

const CHROME_BIN = findChromeBin();

// --- Xvfb launcher ---

let xvfbProc: ReturnType<typeof Bun.spawn> | null = null;
let socatProc: ReturnType<typeof Bun.spawn> | null = null;
let vncProc: ReturnType<typeof Bun.spawn> | null = null;
let websockifyProc: ReturnType<typeof Bun.spawn> | null = null;

async function ensureDisplay(): Promise<string> {
  // DISPLAY が既にあればそのまま使う
  if (process.env.DISPLAY) return process.env.DISPLAY;

  // Xvfb を起動
  const display = ":99";
  xvfbProc = Bun.spawn(
    ["Xvfb", display, "-screen", "0", "1366x768x24", "-nolisten", "tcp"],
    { stdout: "ignore", stderr: "ignore" }
  );
  await Bun.sleep(500); // Xvfb 起動待ち
  console.log(`Xvfb started on ${display}`);

  // x11vnc を起動
  const vncPort = parseInt(process.env.VNC_PORT || "5900");
  try {
    vncProc = Bun.spawn(
      [
        "x11vnc",
        "-display", display,
        "-nopw",
        "-forever",
        "-shared",
        "-rfbport", String(vncPort),
      ],
      { stdout: "ignore", stderr: "ignore" }
    );
    console.log(`VNC_PORT=${vncPort}`);
  } catch {
    console.log("x11vnc: not found (VNC skipped)");
  }

  // websockify + noVNC を起動 (WebSocket→TCP ブリッジ + noVNC Web UI)
  const noVncPort = parseInt(process.env.NOVNC_PORT || "6080");
  try {
    websockifyProc = Bun.spawn(
      [
        "websockify",
        "--web", "/usr/share/novnc",
        String(noVncPort),
        `localhost:${vncPort}`,
      ],
      { stdout: "ignore", stderr: "ignore" }
    );
    console.log(`NOVNC_PORT=${noVncPort}`);
  } catch {
    console.log("websockify: not found (noVNC skipped)");
  }

  return display;
}

// --- Chrome launcher ---

interface ChromeInstance {
  proc: ReturnType<typeof Bun.spawn>;
  cdpPort: number;
}

async function launchChrome(requestedPort = 0): Promise<ChromeInstance> {
  const { mkdirSync, readFileSync, unlinkSync } = await import("fs");
  mkdirSync(CHROME_PROFILE, { recursive: true });

  const display = await ensureDisplay();

  const stderrPath = `${CHROME_PROFILE}/.chrome-stderr`;

  const proc = Bun.spawn(
    [
      CHROME_BIN,
      "--no-first-run",
      "--disable-gpu",
      "--disable-software-rasterizer",
      `--remote-debugging-port=${requestedPort}`,
      `--user-data-dir=${CHROME_PROFILE}`,
      "--window-size=1366,768",
      "--no-sandbox",
      "--lang=ja",
      "--accept-lang=ja,en-US;q=0.9,en;q=0.8",
      "--disable-blink-features=AutomationControlled",
      "--disable-features=WebAuthentication,WebAuthenticationConditionalUI,WebIdentityDigitalCredentials",
      "--disable-webauthn-ui",
      "about:blank",
    ],
    {
      stdout: "inherit",
      stderr: "inherit",
      env: { ...process.env, DISPLAY: display, LANG: "ja_JP.UTF-8" },
    }
  );

  // 固定ポート指定時は CDP ready まで待機、0 の場合は stderr から検出
  let cdpPort: number;
  if (requestedPort > 0) {
    cdpPort = requestedPort;
    await waitForCdpReady(cdpPort);
    try { unlinkSync(stderrPath); } catch {}
  } else {
    cdpPort = await detectCdpPort(stderrPath, readFileSync, unlinkSync);
  }
  return { proc, cdpPort };
}

async function waitForCdpReady(port: number, maxRetries = 50): Promise<void> {
  for (let i = 0; i < maxRetries; i++) {
    try {
      const res = await fetch(`http://127.0.0.1:${port}/json/version`);
      if (res.ok) return;
    } catch {
      // not ready yet
    }
    await Bun.sleep(300);
  }
  throw new Error(`Chrome CDP (port ${port}) が応答しません`);
}

async function detectCdpPort(
  stderrPath: string,
  readFileSync: (path: string, encoding: string) => string,
  unlinkSync: (path: string) => void,
): Promise<number> {
  for (let i = 0; i < 50; i++) {
    try {
      const content = readFileSync(stderrPath, "utf-8");
      const match = content.match(/DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)\//);
      if (match) {
        try { unlinkSync(stderrPath); } catch {}
        return parseInt(match[1]);
      }
    } catch {
      // file not ready yet
    }
    await Bun.sleep(300);
  }
  throw new Error("Chrome CDP ポートを検出できませんでした");
}

// --- CDP helpers ---

async function getPageWsUrl(cdpHost: string, cdpPort: number): Promise<string> {
  const res = await fetch(`http://${cdpHost}:${cdpPort}/json`);
  const targets = (await res.json()) as Array<{
    type: string;
    webSocketDebuggerUrl: string;
  }>;
  const page = targets.find((t) => t.type === "page");
  if (!page) throw new Error("No page target found");
  return page.webSocketDebuggerUrl.replace(
    /ws:\/\/[^/]+/,
    `ws://${cdpHost}:${cdpPort}`
  );
}

async function getPageWsUrlWithRetry(
  cdpHost: string,
  cdpPort: number,
  maxRetries = 30,
  delayMs = 1000
): Promise<string> {
  for (let i = 0; i < maxRetries; i++) {
    try {
      return await getPageWsUrl(cdpHost, cdpPort);
    } catch {
      if (i === maxRetries - 1) throw new Error("Failed to connect to Chrome CDP");
      console.log(`Waiting for Chrome CDP... (${i + 1}/${maxRetries})`);
      await Bun.sleep(delayMs);
    }
  }
  throw new Error("Failed to connect to Chrome CDP");
}

// --- MIME types ---

const MIME_TYPES: Record<string, string> = {
  ".html": "text/html; charset=utf-8",
  ".js": "application/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".ico": "image/x-icon",
  ".webmanifest": "application/manifest+json",
};

function getMimeType(path: string): string {
  const ext = path.substring(path.lastIndexOf("."));
  return MIME_TYPES[ext] || "application/octet-stream";
}

// --- Main ---

const cdpHost = "127.0.0.1";
let chrome: ChromeInstance | null = null;
let cdpPort: number;

const requestedCdpPort = parseInt(process.env.CDP_PORT || "0");

if (process.env.EXTERNAL_CDP) {
  // 既存の Chrome に接続 (Chrome を起動しない)
  cdpPort = requestedCdpPort || 9222;
  console.log(`Connecting to existing Chrome CDP on port ${cdpPort}`);
} else {
  // Chrome を起動 (CDP_PORT 指定時は固定ポート、未指定時は自動割り当て)
  console.log("Launching Chrome headless...");
  chrome = await launchChrome(requestedCdpPort);
  cdpPort = chrome.cdpPort;
  console.log(`Chrome CDP: http://127.0.0.1:${cdpPort}`);

  // socat で CDP を 0.0.0.0 に公開 (Docker コンテナ内で必要)
  const cdpProxyPort = parseInt(process.env.CDP_PROXY_PORT || "9223");
  try {
    socatProc = Bun.spawn(
      ["socat", `TCP-LISTEN:${cdpProxyPort},fork,reuseaddr`, `TCP:127.0.0.1:${cdpPort}`],
      { stdout: "ignore", stderr: "ignore" }
    );
    console.log(`CDP_PROXY_PORT=${cdpProxyPort}`);
  } catch {
    console.log("socat: not found (CDP proxy skipped)");
  }
}

// --- Per-client state ---

interface ClientData {
  cdp: WebSocket | null;
  cdpMsgId: number;
}

const PORT = parseInt(process.env.PORT || "0");

const server = Bun.serve<ClientData>({
  port: PORT,
  hostname: "0.0.0.0",

  async fetch(req, server) {
    const url = new URL(req.url);

    if (url.pathname === "/ws") {
      const upgraded = server.upgrade(req, {
        data: { cdp: null, cdpMsgId: 0 } as ClientData,
      });
      if (upgraded) return undefined;
      return new Response("WebSocket upgrade failed", { status: 500 });
    }

    let filePath = url.pathname;
    if (filePath === "/") filePath = "/index.html";

    if (filePath.includes("..")) {
      return new Response("Forbidden", { status: 403 });
    }

    const file = Bun.file(PUBLIC_DIR + filePath);
    if (await file.exists()) {
      return new Response(file, {
        headers: { "Content-Type": getMimeType(filePath) },
      });
    }

    return new Response("Not Found", { status: 404 });
  },

  websocket: {
    async open(ws) {
      console.log("Viewer client connected");

      try {
        const cdpUrl = await getPageWsUrlWithRetry(cdpHost, cdpPort, 10, 500);
        console.log(`Connecting to CDP: ${cdpUrl}`);

        const cdp = new WebSocket(cdpUrl);
        ws.data.cdp = cdp;

        cdp.addEventListener("open", () => {
          console.log("CDP connected, starting screencast");
          const id = ++ws.data.cdpMsgId;
          cdp.send(
            JSON.stringify({
              id,
              method: "Page.startScreencast",
              params: {
                format: "jpeg",
                quality: 60,
                maxWidth: 1366,
                maxHeight: 768,
              },
            })
          );
          // URL 変更イベントを有効化
          const enableId = ++ws.data.cdpMsgId;
          cdp.send(JSON.stringify({ id: enableId, method: "Page.enable" }));
        });

        cdp.addEventListener("message", (event) => {
          try {
            const msg = JSON.parse(event.data as string);

            if (msg.method === "Page.screencastFrame") {
              const { data, metadata, sessionId } = msg.params;
              ws.send(JSON.stringify({ type: "frame", data, metadata }));
              const ackId = ++ws.data.cdpMsgId;
              cdp.send(
                JSON.stringify({
                  id: ackId,
                  method: "Page.screencastFrameAck",
                  params: { sessionId },
                })
              );
            } else if (msg.method === "Page.frameNavigated") {
              const url = msg.params?.frame?.url;
              if (url && msg.params.frame.parentId === undefined) {
                ws.send(JSON.stringify({ type: "url", url }));
              }
            }
          } catch (e) {
            console.error("Error processing CDP message:", e);
          }
        });

        cdp.addEventListener("close", () => {
          console.log("CDP connection closed");
          try {
            ws.send(
              JSON.stringify({ type: "error", message: "CDP connection closed" })
            );
            ws.close(1000, "CDP disconnected");
          } catch {
            // Client may already be disconnected
          }
        });

        cdp.addEventListener("error", (e) => {
          console.error("CDP WebSocket error:", e);
        });
      } catch (e) {
        console.error("Failed to connect to CDP:", e);
        try {
          ws.send(
            JSON.stringify({
              type: "error",
              message: "Failed to connect to Chrome",
            })
          );
          ws.close(1011, "CDP connection failed");
        } catch {
          // Client may already be disconnected
        }
      }
    },

    message(ws, message) {
      const cdp = ws.data.cdp;
      if (!cdp || cdp.readyState !== WebSocket.OPEN) return;

      try {
        const msg = JSON.parse(message as string);
        console.log(`Input: ${msg.type} ${msg.eventType || ""}`);

        switch (msg.type) {
          case "input_mouse": {
            const id = ++ws.data.cdpMsgId;
            cdp.send(
              JSON.stringify({
                id,
                method: "Input.dispatchMouseEvent",
                params: {
                  type: msg.eventType,
                  x: msg.x,
                  y: msg.y,
                  button: msg.button || "left",
                  buttons: msg.buttons || 0,
                  clickCount: msg.clickCount || 0,
                  modifiers: msg.modifiers || 0,
                },
              })
            );
            break;
          }

          case "input_touch": {
            const id = ++ws.data.cdpMsgId;
            cdp.send(
              JSON.stringify({
                id,
                method: "Input.dispatchTouchEvent",
                params: {
                  type: msg.eventType,
                  touchPoints: msg.touchPoints || [],
                  modifiers: msg.modifiers || 0,
                },
              })
            );
            break;
          }

          case "input_key": {
            const id = ++ws.data.cdpMsgId;
            cdp.send(
              JSON.stringify({
                id,
                method: "Input.dispatchKeyEvent",
                params: {
                  type: msg.eventType,
                  key: msg.key,
                  code: msg.code,
                  text: msg.text,
                  unmodifiedText: msg.text,
                  modifiers: msg.modifiers || 0,
                  windowsVirtualKeyCode: msg.keyCode || 0,
                  nativeVirtualKeyCode: msg.keyCode || 0,
                },
              })
            );
            break;
          }

          case "input_text": {
            const id = ++ws.data.cdpMsgId;
            cdp.send(
              JSON.stringify({
                id,
                method: "Input.insertText",
                params: { text: msg.text },
              })
            );
            break;
          }

          case "input_scroll": {
            const id = ++ws.data.cdpMsgId;
            cdp.send(
              JSON.stringify({
                id,
                method: "Input.dispatchMouseEvent",
                params: {
                  type: "mouseWheel",
                  x: msg.x,
                  y: msg.y,
                  deltaX: msg.deltaX || 0,
                  deltaY: msg.deltaY || 0,
                },
              })
            );
            break;
          }
        }
      } catch (e) {
        console.error("Error processing viewer message:", e);
      }
    },

    close(ws) {
      console.log("Viewer client disconnected");
      if (ws.data.cdp) {
        ws.data.cdp.close();
        ws.data.cdp = null;
      }
    },
  },
});

console.log(`VIEWER_PORT=${server.port}`);
console.log(`CDP_PORT=${cdpPort}`);

// --- Tailscale serve ---

let tailscaleEnabled = false;

async function setupTailscale(viewerPort: number) {
  if (process.env.NO_TAILSCALE) return;

  try {
    const result = Bun.spawnSync(["tailscale", "serve", "--bg", String(viewerPort)]);
    if (result.exitCode === 0) {
      tailscaleEnabled = true;
      // hostname を取得して URL を表示
      const statusResult = Bun.spawnSync(["tailscale", "status", "--json"]);
      if (statusResult.exitCode === 0) {
        const status = JSON.parse(statusResult.stdout.toString());
        const hostname = status.Self?.DNSName?.replace(/\.$/, "");
        if (hostname) {
          console.log(`PWA: https://${hostname}/`);
        }
      }
    } else {
      console.log("tailscale serve: not available (skipped)");
    }
  } catch {
    console.log("tailscale: not found (skipped)");
  }
}

await setupTailscale(server.port);

// --- Cleanup ---

function shutdown() {
  console.log("\nShutting down...");
  if (tailscaleEnabled) {
    Bun.spawnSync(["tailscale", "serve", "--https=443", "off"]);
    console.log("tailscale serve: disabled");
  }
  server.stop();
  if (chrome) {
    chrome.proc.kill();
  }
  if (socatProc) {
    socatProc.kill();
  }
  if (websockifyProc) {
    websockifyProc.kill();
  }
  if (vncProc) {
    vncProc.kill();
  }
  if (xvfbProc) {
    xvfbProc.kill();
  }
  process.exit(0);
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
