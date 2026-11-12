#!/usr/bin/env bash
# Package a release build into dist/.
# Usage: scripts/package_release.sh <windows-x64|linux-x64|macos-universal> <tag>
# Expects `cargo build --release` output in target/release/. ProjectorHUD is a
# single program with nothing to bundle beside it.
set -euo pipefail
# A failure inside $(...) must stop the script too.
shopt -s inherit_errexit 2>/dev/null || true

platform="$1"
tag="$2"
name="ProjectorHUD-${tag}-${platform}"

cd "$(dirname "$0")/.."
root="$(pwd)"
rm -rf dist
stage="dist/stage/${name}"
mkdir -p "$stage"
cp README.md LICENSE "$stage/"

case "$platform" in
  windows-x64)
    cp target/release/projectorhud.exe "$stage/ProjectorHUD.exe"
    (cd dist/stage && 7z a -tzip -bso0 "${root}/dist/${name}.zip" "$name")
    ;;
  linux-x64)
    cp target/release/projectorhud "$stage/"
    chmod +x "$stage/projectorhud"
    tar -C dist/stage -czf "dist/${name}.tar.gz" "$name"
    ;;
  macos-universal)
    app="$stage/ProjectorHUD.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp target/release/projectorhud "$app/Contents/MacOS/projectorhud"
    chmod +x "$app/Contents/MacOS/projectorhud"
    cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>ProjectorHUD</string>
  <key>CFBundleDisplayName</key><string>ProjectorHUD</string>
  <key>CFBundleIdentifier</key><string>io.github.r2a42.projectorhud</string>
  <key>CFBundleExecutable</key><string>projectorhud</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${tag#v}</string>
  <key>CFBundleVersion</key><string>${tag#v}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSLocalNetworkUsageDescription</key><string>ProjectorHUD finds and controls projectors on your local network.</string>
</dict>
</plist>
PLIST
    if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
      # A Developer ID signature with the hardened runtime and a secure
      # timestamp, which is what notarisation asks for.
      ent="$root/assets/macos/entitlements.plist"
      codesign --force --timestamp --options runtime --entitlements "$ent" --sign "$MACOS_SIGN_IDENTITY" "$app"
      codesign --verify --deep --strict --verbose=2 "$app"
      if [ -n "${APPLE_API_KEY_PATH:-}" ]; then
        # Notarise (Apple scans it and records it), then staple the ticket so the
        # app opens even offline.
        (cd dist/stage && ditto -c -k --keepParent "$name" "${root}/dist/notarize.zip")
        out="$(xcrun notarytool submit "dist/notarize.zip" --key "$APPLE_API_KEY_PATH" \
          --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID" --wait --timeout 40m 2>&1)" || true
        echo "$out"
        id="$(sed -nE 's/^ *id: ([0-9a-f-]{36}).*/\1/p' <<< "$out" | head -1)"
        if ! grep -q "status: Accepted" <<< "$out"; then
          if [ -n "$id" ]; then
            xcrun notarytool log "$id" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID" || true
          fi
          echo "notarisation did not succeed" >&2
          exit 1
        fi
        rm -f dist/notarize.zip
        xcrun stapler staple "$app"
        xcrun stapler validate "$app"
        spctl --assess --type execute --verbose=2 "$app"
      else
        echo "signed, not notarised (no App Store Connect API key given)"
      fi
    else
      # Apple Silicon only runs signed code: ad-hoc sign the app.
      codesign --force --sign - "$app"
      codesign --verify --deep --strict "$app"
    fi
    (cd dist/stage && ditto -c -k --keepParent "$name" "${root}/dist/${name}.zip")
    ;;
  *)
    echo "unknown platform: $platform" >&2
    exit 1
    ;;
esac

rm -rf dist/stage
ls -l dist
