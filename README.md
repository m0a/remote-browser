# remote-browser

Rust + CEF (Chromium Embedded Framework) によるリモートブラウザ。
CEF の Off-Screen Rendering でピクセルバッファを直接取得し、JPEG フレームを WebSocket で PWA ビューアに配信する。

## 目的

AI エージェント (Claude Code の agent-browser 等) でブラウザ自動化を行う際に、認証 (ログイン/2FA/CAPTCHA) を人間がスマホから引き継ぐためのリモートブラウザ。

1. CEF ブラウザを起動し、CDP エンドポイントを公開
2. 人間はスマホから PWA ビューア (Tailscale HTTPS) にアクセスし、タッチ操作で認証
3. 認証完了後、AI が CDP 経由で同じブラウザセッションに接続し自動化を継続

## アーキテクチャ

```
cef-browser (Rust シングルバイナリ)
  ├── CEF OSR → BGRA → JPEG → WebSocket バイナリフレーム (30 FPS)
  ├── HTTP/WS サーバー (PORT=3000)
  │     ├── PWA 静的ファイル配信
  │     ├── Session REST API (/api/sessions)
  │     ├── フレーム配信 (/ws?session=<id>)
  │     └── 入力受信 (WebSocket JSON)
  └── CDP エンドポイント (--remote-debugging-port=9222)

tailscale serve --bg 3000
  → https://<hostname>.ts.net/ で HTTPS 公開

スマホ: https://<hostname>.ts.net → PWA (タブ切り替え + タッチ操作)
AI:    agent-browser --cdp 9222 (localhost)
```

## 機能

- **マルチセッション (タブ)** — 複数の独立したブラウザセッションをタブ UI で切り替え
- CEF OSR → JPEG → WebSocket バイナリフレーム (30 FPS)
- ネイティブダイアログ完全抑制 (alert/confirm/prompt/file picker/WebAuthn)
- WebAuthn パスキー自動キャンセル + パスワード認証フォールバック誘導
- ブラウザツールバー (戻る/進む/リロード/URL バー)
- ファイルダウンロード (`DOWNLOAD_DIR` に保存 + Viewer トースト通知)
- CDP エンドポイント
- マウス/キーボード/タッチ/スクロール入力中継
- ピンチズーム + パン + ダブルタップズーム (スマホ操作支援)
- IME 日本語入力対応
- フレーム差分スキップ (変化なし時は送信しない)
- Xvfb 自動起動 (DISPLAY 未設定時)
- Tailscale 自動連携

## クイックスタート

### ビルドと起動

```bash
cd cef-browser
cargo build
./target/debug/cef-browser [URL]
```

`cargo build` 時に CEF ランタイムファイルが `target/debug/` に自動バンドルされる。
rpath 設定済みのため `LD_LIBRARY_PATH` 不要。Xvfb / Tailscale も自動起動。

### 起動後の出力

```
VIEWER_PORT=3000
CDP_PORT=9222
TAILSCALE_URL=https://<hostname>.ts.net/
```

## マルチセッション

PWA Viewer のタブ UI と REST API で、複数の独立したブラウザセッションを管理できる。

### タブ操作 (PWA Viewer)

- **タブクリック** — セッション切り替え
- **`+` ボタン** — 新規セッション (Google を開く)
- **`×` ボタン** — セッション削除

### REST API

```bash
# セッション一覧
curl http://localhost:3000/api/sessions
# → [{"id":"0","url":"https://www.google.com/","title":"Google"}]

# 新規セッション作成
curl -X POST http://localhost:3000/api/sessions \
  -H "Content-Type: application/json" \
  -d '{"url":"https://github.com"}'
# → {"id":"1","url":"https://github.com","title":""}

# セッション削除
curl -X DELETE http://localhost:3000/api/sessions/1
# → 204 No Content
```

### WebSocket

```
ws://localhost:3000/ws?session=0   # セッション 0 のフレーム/入力
ws://localhost:3000/ws?session=1   # セッション 1 のフレーム/入力
```

## 環境変数

| 変数 | デフォルト | 説明 |
|------|-----------|------|
| `PORT` | `3000` | HTTP/WS サーバーポート |
| `CDP_PORT` | `9222` | Chrome DevTools Protocol ポート |
| `START_URL` | `https://www.google.com` | 初期 URL |
| `PUBLIC_DIR` | `public` | 静的ファイルディレクトリ |
| `DOWNLOAD_DIR` | `./downloads` | ダウンロードファイルの保存先 |
| `NO_TAILSCALE` | - | 設定すると Tailscale 自動連携を無効化 |

## スマホからのアクセス

1. `https://<hostname>.ts.net/` にアクセス
2. タブバーでセッションを切り替え、`+` で新規タブ作成
3. タッチ操作で直接ブラウザを操作
4. ピンチで拡大、ダブルタップで 2x ズーム/リセット
5. キーボードボタン (右下) でソフトキーボードを表示
6. 「ホーム画面に追加」で PWA としてインストール

## AI からの接続

```bash
# CDP 疎通確認
curl http://localhost:9222/json/version

# agent-browser で接続
agent-browser --cdp 9222 snapshot -i
```

## ビルド要件

- Rust 1.86+
- cmake, ninja, clang (libclang)
- Xvfb (xorg-server-xvfb) - ヘッドレス環境用、自動起動

## ファイル構成

```
remote-browser/
├── README.md
├── CLAUDE.md             # AI 向けプロジェクトドキュメント
└── cef-browser/
    ├── Cargo.toml
    ├── build.rs           # CEF ファイルバンドル + rpath 設定
    ├── src/
    │   └── main.rs        # HTTP/WS サーバー + セッション管理 + 入力ハンドリング
    ├── public/
    │   ├── index.html     # PWA ビューア (タブ UI)
    │   ├── app.js         # Canvas 描画 + タブ管理 + タッチ/キーボード入力
    │   ├── style.css
    │   ├── manifest.json  # PWA マニフェスト
    │   ├── sw.js          # Service Worker
    │   └── icon.svg
    └── wew/               # CEF Rust バインディング (mycrl/wew フォーク)
        ├── src/
        └── cxx/           # C++ (subprocess, dialog/download handlers)
```

## ライセンス

MIT
