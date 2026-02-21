#!/bin/bash
# CEF Browser PoC - 実行スクリプト
# CEF のランタイムファイルをバイナリと同じディレクトリにセットアップして実行

set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BUILD_DIR="$SCRIPT_DIR/target/debug"
CEF_OUT="$BUILD_DIR/build/wew-*/out/cef"
CEF_DIR=$(echo $CEF_OUT)
BUNDLE_DIR="$SCRIPT_DIR/target/bundle"

# パッケージング
echo "[setup] Bundling CEF runtime files..."
mkdir -p "$BUNDLE_DIR"

# CEF 共有ライブラリ
cp -u "$CEF_DIR/Release/"*.so* "$BUNDLE_DIR/" 2>/dev/null || true
cp -u "$CEF_DIR/Release/"*.bin "$BUNDLE_DIR/" 2>/dev/null || true
cp -u "$CEF_DIR/Release/"*.dat "$BUNDLE_DIR/" 2>/dev/null || true

# CEF リソース (icu, locales, etc.)
cp -u "$CEF_DIR/Resources/"*.pak "$BUNDLE_DIR/" 2>/dev/null || true
cp -u "$CEF_DIR/Resources/"*.dat "$BUNDLE_DIR/" 2>/dev/null || true
cp -r "$CEF_DIR/Resources/locales" "$BUNDLE_DIR/" 2>/dev/null || true

# ビルド済みバイナリ
cp -u "$BUILD_DIR/cef-browser" "$BUNDLE_DIR/"

echo "[setup] Bundle ready at $BUNDLE_DIR"
echo "[setup] Starting CEF Browser..."

# LD_LIBRARY_PATH を設定
cd "$BUNDLE_DIR"
export LD_LIBRARY_PATH="$BUNDLE_DIR:$LD_LIBRARY_PATH"
export PUBLIC_DIR="$SCRIPT_DIR/public"

# Xvfb 起動 (既存の DISPLAY があればスキップ)
if [ -z "$DISPLAY" ]; then
    DISPLAY_NUM=99
    export DISPLAY=":$DISPLAY_NUM"
    Xvfb "$DISPLAY" -screen 0 1280x720x24 -ac &
    XVFB_PID=$!
    sleep 1
    echo "[setup] Xvfb started on $DISPLAY (PID: $XVFB_PID)"
    trap "kill $XVFB_PID 2>/dev/null" EXIT
fi

# CDP ポート (デフォルト: 9222)
CDP_PORT="${CDP_PORT:-9222}"

# CEF フラグ (ヘッドレス環境用)
exec ./cef-browser \
    --no-sandbox \
    --disable-gpu \
    --disable-gpu-compositing \
    --disable-software-rasterizer \
    --remote-debugging-port="$CDP_PORT" \
    "$@"
