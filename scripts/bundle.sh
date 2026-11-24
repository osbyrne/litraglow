#!/bin/zsh
# Builds target/LitraGlow.app (menu-bar-only app bundle, ad-hoc signed).
# Usage: scripts/bundle.sh [--install]   (--install copies it to ~/Applications)
set -euo pipefail
cd "${0:A:h}/.."

APP=target/LitraGlow.app
VERSION=$(cargo metadata --no-deps --format-version 1 | jq -r '.packages[0].version')

cargo build --release
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/litraglow "$APP/Contents/MacOS/LitraGlow"

# Icon: assets/icon.png (1024x1024, see scripts/make_icon.py) -> iconset -> icns.
tmp=$(mktemp -d)
set=$tmp/AppIcon.iconset; mkdir "$set"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon.png --out "$set/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) assets/icon.png --out "$set/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$set" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$tmp"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Litra Glow</string>
  <key>CFBundleDisplayName</key><string>Litra Glow</string>
  <key>CFBundleIdentifier</key><string>com.osbyrne.litraglow</string>
  <key>CFBundleExecutable</key><string>LitraGlow</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST

codesign --force --sign - "$APP"
echo "Built $APP"

if [[ "${1:-}" == "--install" ]]; then
  mkdir -p ~/Applications
  rm -rf ~/Applications/LitraGlow.app
  cp -R "$APP" ~/Applications/
  echo "Installed to ~/Applications/LitraGlow.app"
fi
