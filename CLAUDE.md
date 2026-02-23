# remote-browser

Chrome headless + CDP screencast ベースのリモートブラウザ。
シングルバイナリで Chrome の起動から PWA 配信まで一括管理する。

## 目的・ユースケース

1. CC の agent-browser 等でブラウザ自動化を行う際、認証(ログイン/2FA/CAPTCHA)を人間が引き継ぐ
2. 人間はスマホから PWA ビューア (HTTPS) にアクセスし、タッチ操作で認証
3. 認証完了後、CC が CDP 経由で同じブラウザセッションに接続し自動化を継続する

## アーキテクチャ

```
ホスト上で直接実行 (Docker 不要):

  viewer-server (シングルバイナリ)
    ├── Chrome --headless=new --remote-debugging-port=0
    │     CDP ポートは自動割り当て (stderr から検出)
    └── HTTP/WS サーバー (PORT=0)
          ポートは自動割り当て (stdout に出力)

  tailscale serve --bg <viewer-port>
    → https://<hostname>.ts.net/ で HTTPS 公開

スマホ: https://<hostname>.ts.net → PWA (CDP screencast)
AI:    agent-browser --cdp <cdp-port> (localhost)
```

## 前提条件

- Google Chrome または Chromium がインストール済み
- Bun ランタイム (開発時) または ビルド済みバイナリ
- Tailscale (HTTPS 公開に使用、任意)

## 起動

```bash
# 開発モード (Bun で直接実行)
bun run viewer/server.ts

# ビルド済みバイナリ
./viewer/viewer-server
```

起動すると stdout に以下が出力される:
```
VIEWER_PORT=<port>
CDP_PORT=<port>
```

### 環境変数

| 変数 | デフォルト | 説明 |
|------|-----------|------|
| `CHROME_BIN` | 自動検出 | Chrome バイナリパス |
| `CHROME_PROFILE` | `./chrome-profile` | Chrome プロファイルディレクトリ |
| `CDP_PORT` | (自動) | 指定すると Chrome を起動せず既存の CDP に接続 |
| `PORT` | `0` (自動) | HTTP/WS サーバーポート |
| `PUBLIC_DIR` | `viewer/public` | 静的ファイルディレクトリ |

### HTTPS 公開 (Tailscale)

```bash
# viewer-server 起動後、出力された VIEWER_PORT で tailscale serve を設定
tailscale serve --bg <VIEWER_PORT>
```

### スマホからのアクセス

1. `https://<hostname>.ts.net/` にアクセス
2. ブラウザのビューポートがフルスクリーンで表示される
3. タッチ操作で直接ブラウザを操作可能
4. キーボードボタン (右下) でソフトキーボードを表示
5. 「ホーム画面に追加」で PWA としてインストール

## AI (Claude Code) からの接続

```bash
# CDP 疎通確認 (出力された CDP_PORT を使用)
curl http://localhost:<CDP_PORT>/json/version
```

```javascript
// Playwright
const browser = await playwright.chromium.connectOverCDP('http://localhost:<CDP_PORT>');

// Puppeteer
const browser = await puppeteer.connect({ browserURL: 'http://localhost:<CDP_PORT>' });
```

## セキュリティ考慮

- CDP は 127.0.0.1 にのみバインドされる (Chrome のセキュリティ仕様)
- PWA は Tailscale (WireGuard VPN) 経由の HTTPS でアクセス
- chrome-profile にはセッション情報が残るため適切に管理すること

## ビルド

```bash
# シングルバイナリのビルド
bun build --compile viewer/server.ts --outfile viewer/viewer-server --target=bun-linux-x64
```

## ファイル構成

```
remote-browser/
├── CLAUDE.md             # このファイル
├── policies.json         # Chrome DevTools ポリシー (リファレンス)
└── viewer/
    ├── server.ts          # Chrome launcher + CDP bridge + HTTP server (Bun)
    ├── viewer-server      # ビルド済みバイナリ
    └── public/
        ├── index.html     # PWA ビューア
        ├── app.js         # Canvas 描画 + タッチ/キーボード入力
        ├── style.css      # フルスクリーン スタイル
        ├── manifest.json  # PWA マニフェスト
        ├── sw.js          # Service Worker
        └── icon.svg       # PWA アイコン
```

## コマンドリファレンス

```bash
# 起動 (開発)
bun run viewer/server.ts

# 起動 (既存 Chrome に接続)
CDP_PORT=9222 bun run viewer/server.ts

# ビルド
bun build --compile viewer/server.ts --outfile viewer/viewer-server --target=bun-linux-x64

# Chrome プロファイルのクリア
rm -rf ./chrome-profile/*

# HTTPS 公開
tailscale serve --bg <VIEWER_PORT>

# HTTPS 解除
tailscale serve off
```

## CEF Browser (cef-browser/)

Rust + CEF (mycrl/wew フォーク) による次世代リモートブラウザ。
CEF の Off-Screen Rendering で直接ピクセルバッファを取得し、
ネイティブダイアログをゼロにしている。

### 機能

- CEF OSR → BGRA → JPEG → WebSocket バイナリフレーム (30 FPS)
- ネイティブダイアログ完全抑制 (alert/confirm/prompt/file picker)
- ダイアログイベントの viewer 通知 (トースト UI)
- ブラウザツールバー (戻る/進む/リロード/URL バー)
- CDP エンドポイント (`--remote-debugging-port`)
- マウス/キーボード/タッチ/スクロール入力中継
- フレーム差分スキップ (変化なし時は送信しない)

### 起動 (開発)

```bash
cd cef-browser
cargo build
bash run.sh [URL]
```

### 起動 (Docker)

```bash
docker compose up -d
docker compose ps   # ポート確認
```

### 環境変数

| 変数 | デフォルト | 説明 |
|------|-----------|------|
| `PORT` | `3000` | HTTP/WS サーバーポート |
| `CDP_PORT` | `9222` | Chrome DevTools Protocol ポート |
| `START_URL` | `https://www.google.com` | 初期 URL |
| `PUBLIC_DIR` | `public` | 静的ファイルディレクトリ |

### 依存 (ネイティブビルド)

- Rust 1.86+, cmake, ninja, clang (libclang)
- Xvfb (xorg-server-xvfb)
