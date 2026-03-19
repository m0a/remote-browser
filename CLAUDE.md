# remote-browser

Rust + CEF (mycrl/wew フォーク) によるリモートブラウザ。
CEF の Off-Screen Rendering で直接ピクセルバッファを取得し、
JPEG フレームを WebSocket で PWA ビューアに配信する。

## 目的・ユースケース

1. CC の agent-browser 等でブラウザ自動化を行う際、認証(ログイン/2FA/CAPTCHA)を人間が引き継ぐ
2. 人間はスマホから PWA ビューア (HTTPS) にアクセスし、タッチ操作で認証
3. 認証完了後、CC が CDP 経由で同じブラウザセッションに接続し自動化を継続する

## アーキテクチャ

```
cef-browser (Rust バイナリ)
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

- CEF OSR → BGRA → JPEG → WebSocket バイナリフレーム (30 FPS)
- ネイティブダイアログ完全抑制 (alert/confirm/prompt/file picker/WebAuthn)
- WebAuthn パスキー自動キャンセル + フォールバック誘導
- ダイアログイベントの viewer 通知 (トースト UI)
- ブラウザツールバー (戻る/進む/リロード/URL バー)
- CDP エンドポイント (`--remote-debugging-port`)
- マウス/キーボード/タッチ/スクロール入力中継
- ピンチズーム + パン (スマホ操作支援)
- ダブルタップズーム (1x ↔ 2x)
- IME 日本語入力対応
- フレーム差分スキップ (変化なし時は送信しない)
- 日本語ロケール (Accept-Language: ja)
- ファイルダウンロード (`DOWNLOAD_DIR` に保存 + viewer トースト通知)

## 起動 (開発)

```bash
cd cef-browser
cargo build
./target/debug/cef-browser [URL]
```

`cargo build` 時に CEF ランタイムファイルと public/ が `target/debug/` に自動バンドルされる。
rpath 設定済みのため `LD_LIBRARY_PATH` 不要。Xvfb / Tailscale も自動起動。

起動すると stderr に以下が出力される:
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
| `DOWNLOAD_DIR` | `./downloads` | ダウンロードファイルの保存先 |

## HTTPS 公開 (Tailscale)

```bash
tailscale serve --bg 3000
# → https://<hostname>.ts.net/
```

## スマホからのアクセス

1. `https://<hostname>.ts.net/` にアクセス
2. ブラウザ画面がフルスクリーンで表示される
3. タッチ操作で直接ブラウザを操作可能
4. ピンチで拡大、ダブルタップで 2x ズーム/リセット
5. キーボードボタン (右下) でソフトキーボードを表示
6. 「ホーム画面に追加」で PWA としてインストール

## AI (Claude Code) からの接続

```bash
# CDP 疎通確認
curl http://localhost:9222/json/version

# agent-browser で接続
agent-browser --cdp 9222 snapshot -i
```

## 依存 (ネイティブビルド)

- Rust 1.86+, cmake, ninja, clang (libclang)
- Xvfb (xorg-server-xvfb)

## ファイル構成

```
remote-browser/
├── CLAUDE.md
├── .gitignore
└── cef-browser/
    ├── Cargo.toml
    ├── Cargo.lock
    ├── build.rs           # CEF ファイルバンドル + rpath 設定
    ├── src/
    │   ├── main.rs          # エントリポイント + CEF 初期化 + セットアップ
    │   ├── session.rs       # Session, AppState, SessionCmd 構造体
    │   ├── handler.rs       # FrameHandler (CEF コールバック + JPEG エンコード)
    │   ├── input.rs         # 入力メッセージ型 + handle_input
    │   ├── api.rs           # REST API (sessions CRUD + CDP ターゲット)
    │   └── ws.rs            # WebSocket ハンドラ
    ├── public/
    │   ├── index.html        # PWA ビューア
    │   ├── app.js            # Canvas 描画 + タッチ/キーボード入力
    │   ├── style.css         # フルスクリーン スタイル
    │   ├── manifest.json     # PWA マニフェスト
    │   ├── sw.js             # Service Worker
    │   └── icon.svg          # PWA アイコン
    └── wew/                  # CEF Rust バインディング (mycrl/wew フォーク)
        ├── src/              # Rust ソース
        └── cxx/              # C++ ソース (subprocess, dialog handlers)
```

## コマンドリファレンス

```bash
# 起動 (開発)
cd cef-browser && cargo build && ./target/debug/cef-browser [URL]

# 環境変数で設定をオーバーライド
PORT=8080 CDP_PORT=9333 START_URL=https://example.com ./target/debug/cef-browser

# Tailscale 無効化
NO_TAILSCALE=1 ./target/debug/cef-browser
```
