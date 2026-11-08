#!/bin/bash
# macOS のキーチェーンが更新前後のアプリを同じ署名として識別できる形でビルドする。
set -euo pipefail

cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
    echo "このスクリプトは macOS 用です。" >&2
    exit 1
fi

profile="${1:-debug}"
case "$profile" in
    debug) cargo_profile=dev ;;
    release) cargo_profile=release ;;
    *) echo "使い方: bash scripts/build-macos-app.sh [debug|release]" >&2; exit 1 ;;
esac

identity="${YOMITONE_SIGNING_IDENTITY:-}"
if [[ -z "$identity" ]]; then
    identities=$(security find-identity -v -p codesigning | sed -nE '/"(Apple Development:|Mac Developer:|Developer ID Application:)/s/^[[:space:]]*[0-9]+\) ([0-9A-F]+).*/\1/p')
    count=$(printf '%s\n' "$identities" | sed '/^$/d' | wc -l | tr -d ' ')
    if [[ "$count" != 1 ]]; then
        echo "コード署名証明書を YOMITONE_SIGNING_IDENTITY で指定してください。" >&2
        exit 1
    fi
    identity="$identities"
fi
if [[ "$identity" == - ]]; then
    echo "アドホック署名は利用できません。コード署名証明書を指定してください。" >&2
    exit 1
fi

cargo build -p yomitone-app --locked --profile "$cargo_profile"
app="target/$profile/Yomitone.app"
mkdir -p "$app/Contents/MacOS"
cp "target/$profile/yomitone-app" "$app/Contents/MacOS/Yomitone"
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>Yomitone</string>
<key>CFBundleIdentifier</key><string>io.github.mokuichi147.yomitone</string>
<key>CFBundleName</key><string>Yomitone</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>0.1.0</string>
</dict></plist>
PLIST

codesign --force --sign "$identity" --options runtime --timestamp=none "$app"
codesign --verify --strict -R='anchor apple generic' "$app"
# 実行ファイルを直接起動する場合も、同じ証明書と識別子を使う。
codesign --force --sign "$identity" --identifier io.github.mokuichi147.yomitone --options runtime --timestamp=none "target/$profile/yomitone-app"
echo "作成しました: $app"
