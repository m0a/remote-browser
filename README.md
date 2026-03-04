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
  │     ├── フレーム配信 (WebSocket binary)
  │     └── 入力受信 (WebSocket JSON)
  └── CDP エンドポイント (--remote-debugging-port=9222)

tailscale serve --bg 3000
  → https://<hostname>.ts.net/ で HTTPS 公開

スマホ: https://<hostname>.ts.net → PWA (タッチ操作)
AI:    agent-browser --cdp 9222 (localhost)
```

## 機能

- CEF OSR → JPEG → WebSocket バイナリフレーム (30 FPS)
- ネイティブダイアログ完全抑制 (alert/confirm/prompt/file picker/WebAuthn)
- WebAuthn パスキー自動キャンセル + パスワード認証フォールバック誘導
- ブラウザツールバー (戻る/進む/リロード/URL バー)
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

### Docker

```bash
docker compose up -d
docker compose ps   # ポート確認
```

### 起動後の出力

```
VIEWER_PORT=3000
CDP_PORT=9222
TAILSCALE_URL=https://<hostname>.ts.net/
```

## 環境変数

| 変数 | デフォルト | 説明 |
|------|-----------|------|
| `PORT` | `3000` | HTTP/WS サーバーポート |
| `CDP_PORT` | `9222` | Chrome DevTools Protocol ポート |
| `START_URL` | `https://www.google.com` | 初期 URL |
| `PUBLIC_DIR` | `public` | 静的ファイルディレクトリ |
| `NO_TAILSCALE` | - | 設定すると Tailscale 自動連携を無効化 |

## スマホからのアクセス

1. `https://<hostname>.ts.net/` にアクセス
2. ブラウザ画面がフルスクリーンで表示される
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
├── docker-compose.yml
└── cef-browser/
    ├── Cargo.toml
    ├── build.rs           # CEF ファイルバンドル + rpath 設定
    ├── Dockerfile
    ├── src/
    │   └── main.rs        # HTTP/WS サーバー + 入力ハンドリング
    ├── public/
    │   ├── index.html     # PWA ビューア
    │   ├── app.js         # Canvas 描画 + タッチ/キーボード入力
    │   ├── style.css
    │   ├── manifest.json  # PWA マニフェスト
    │   ├── sw.js          # Service Worker
    │   └── icon.svg
    └── wew/               # CEF Rust バインディング (mycrl/wew フォーク)
        ├── src/
        └── cxx/           # C++ (subprocess, dialog handlers)
```

## ライセンス

MIT
