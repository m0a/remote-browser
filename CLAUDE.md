# remote-browser

n.eko ベースのリモートブラウザ + CDP (Chrome DevTools Protocol) 対応プロジェクト。
Linux サーバ (X server なし) 上で Docker コンテナとしてブラウザを動かし、
Android 端末等から WebRTC で操作、Claude Code (CC) が CDP 経由で自動化を引き継ぐ。

## 目的・ユースケース

1. CC の agent-browser 等でブラウザ自動化を行う際、認証(ログイン/2FA/CAPTCHA)を人間が引き継ぐ
2. 人間は Android 端末の Web ブラウザから WebRTC UI にアクセスし、認証操作を行う
3. 認証完了後、CC が CDP 経由で同じブラウザセッションに接続し自動化を継続する

## アーキテクチャ

```
┌─────────────────────────────────────┐
│  Docker Container (remote-browser)  │
│                                     │
│  ┌───────────┐  ┌────────────────┐  │
│  │  Xvfb     │  │  openbox (WM)  │  │
│  └─────┬─────┘  └────────────────┘  │
│        │                             │
│  ┌─────┴──────────────────────────┐  │
│  │  Google Chrome                 │  │
│  │  --remote-debugging-port=9222  │  │
│  │  (127.0.0.1:9222 に bind)     │  │
│  └─────┬──────────────────────────┘  │
│        │                             │
│  ┌─────┴──────────────────────────┐  │
│  │  socat proxy                   │  │
│  │  0.0.0.0:9223 → 127.0.0.1:9222│  │
│  └────────────────────────────────┘  │
│                                     │
│  ┌────────────────────────────────┐  │
│  │  neko server (WebRTC + Web UI) │  │
│  │  0.0.0.0:8080                  │  │
│  └────────────────────────────────┘  │
└─────────────────────────────────────┘

ポート:
  8080/tcp        → WebRTC UI (人間がブラウザ操作)
  52000-52100/udp → WebRTC メディアストリーム
  9223/tcp        → CDP (CC が自動化に使用)
```

## 技術的な注意点

### Chrome の CDP バインド問題
Chrome は `--remote-debugging-address=0.0.0.0` を指定しても **127.0.0.1 にしかバインドしない**
(Chromium のセキュリティ仕様)。そのため `socat` で TCP プロキシを経由させる必要がある:
```
socat TCP-LISTEN:9223,fork,reuseaddr TCP:127.0.0.1:9222
```

### DevTools ポリシー
デフォルトの n.eko chromium イメージは `DeveloperToolsAvailability: 2` (無効) に設定されている。
CDP を使うには `policies.json` で `1` (有効) に変更する必要がある。

### user-data-dir
デバッグモードで Chrome を起動するには、デフォルトとは別の `--user-data-dir` が必要。
`/home/neko/chrome-profile` を使用する。

## セットアップ手順

### 前提条件
- Docker + Docker Compose がインストール済み
- ポート 8080, 9223, 52000-52100/udp が利用可能

### 起動
```bash
cd /home/m0a/repos/remote-browser
docker compose up -d
```

### 接続
- **WebRTC UI (人間操作):** `http://<server-ip>:8080`
  - User パスワード: `neko`
  - Admin パスワード: `admin`
- **CDP (CC 自動化):** `http://localhost:9223`

### CDP 接続確認
```bash
curl http://localhost:9223/json/version
```

### Playwright からの接続
```javascript
const browser = await playwright.chromium.connectOverCDP('http://localhost:9223');
```

### Puppeteer からの接続
```javascript
const browser = await puppeteer.connect({ browserURL: 'http://localhost:9223' });
```

## 運用フロー

1. `docker compose up -d` でコンテナ起動
2. Android 端末のブラウザで `http://<server-ip>:8080` にアクセス
3. admin パスワードでログインし、ブラウザを操作(認証、ログイン等)
4. CC が `ws://localhost:9223` で CDP 接続し、自動化を引き継ぐ
5. 作業完了後 `docker compose down` で停止

## セキュリティ考慮

- CDP ポート (9223) は **localhost のみ** に公開すること (外部に露出させない)
- WebRTC UI のパスワードは本番では強いものに変更すること
- 必要に応じて Tailscale/WireGuard 等の VPN 経由でアクセスすること
- chrome-profile ボリュームにはセッション情報が残るため適切に管理すること

## ファイル構成

```
remote-browser/
├── CLAUDE.md             # このファイル (プロジェクト説明)
├── Dockerfile            # n.eko google-chrome + socat + CDP 対応
├── docker-compose.yml    # サービス定義
├── supervisord.conf      # Chrome + socat proxy + openbox 設定
├── policies.json         # Chrome DevTools 有効化ポリシー
└── openbox.xml           # ウィンドウマネージャ設定
```

## ベースプロジェクト

- [m1k1o/neko](https://github.com/m1k1o/neko) - セルフホスト仮想ブラウザ (Apache-2.0)
- [m1k1o/neko-apps](https://github.com/m1k1o/neko-apps) - neko アプリ集 (chrome-remote-debug)
- 参考: [Issue #391](https://github.com/m1k1o/neko/issues/391) - Playwright + neko の議論

## コマンドリファレンス

```bash
# ビルド
docker compose build

# 起動
docker compose up -d

# ログ確認
docker compose logs -f

# 停止
docker compose down

# Chrome プロファイルのクリア
rm -rf ./chrome-profile/*

# CDP 疎通確認
curl http://localhost:9223/json/version
```
