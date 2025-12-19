#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."

TEAM_ID="WJQ6TR5FJS"
BUNDLE_ID="com.loaf.app"

# Notarization credentials (key file is the only secret - keep it outside repo)
API_KEY="${LOAF_API_KEY:-$HOME/Downloads/App Store Connect AuthKey.p8}"
API_KEY_ID="${LOAF_API_KEY_ID:-AR54MST7DV}"
API_ISSUER="${LOAF_API_ISSUER:-9c0ad768-8373-4b79-b2ea-210843ade8d4}"

if [[ ! -f "$API_KEY" ]]; then
    echo "Error: API key not found at $API_KEY"
    echo "Download from App Store Connect → Users and Access → Integrations → API"
    exit 1
fi

echo "Building Zig library..."
zig build -Doptimize=ReleaseFast

echo "Generating Xcode project..."
cd swift
if ! command -v xcodegen &> /dev/null; then
    echo "Installing xcodegen..."
    brew install xcodegen
fi
xcodegen generate --quiet

echo "Building and signing with Xcode..."
xcodebuild -project Loaf.xcodeproj \
    -scheme Loaf \
    -configuration Release \
    -derivedDataPath build \
    ARCHS=arm64 \
    CODE_SIGN_STYLE=Automatic \
    DEVELOPMENT_TEAM="$TEAM_ID" \
    -quiet

APP_PATH="build/Build/Products/Release/Loaf.app"
EXT_PATH="$APP_PATH/Contents/Extensions/LoafExtension.appex"
EXT_FRAMEWORKS="$EXT_PATH/Contents/Frameworks"

echo "Embedding Zig library into extension..."
mkdir -p "$EXT_FRAMEWORKS"
cp ../zig-out/lib/libloaf.dylib "$EXT_FRAMEWORKS/"

# Fix the library's install name to use @rpath
install_name_tool -id "@rpath/libloaf.dylib" "$EXT_FRAMEWORKS/libloaf.dylib"

echo "Re-signing with Developer ID and hardened runtime..."
# Sign the dylib first
codesign --force --options runtime --timestamp \
    --sign "Developer ID Application: Andrew Gazelka (WJQ6TR5FJS)" \
    "$EXT_FRAMEWORKS/libloaf.dylib"
# Sign the extension first (inside-out signing)
codesign --force --options runtime --timestamp \
    --sign "Developer ID Application: Andrew Gazelka (WJQ6TR5FJS)" \
    --entitlements LoafExtension/LoafExtension.entitlements \
    "$EXT_PATH"

# Sign the main app
codesign --force --options runtime --timestamp \
    --sign "Developer ID Application: Andrew Gazelka (WJQ6TR5FJS)" \
    --entitlements LoafApp/Loaf.entitlements \
    "$APP_PATH"

echo "Creating ZIP for notarization..."
cd build/Build/Products/Release
zip -r -q Loaf.zip Loaf.app
cd -

echo "Notarizing with Apple..."
xcrun notarytool submit build/Build/Products/Release/Loaf.zip \
    --key "$API_KEY" \
    --key-id "$API_KEY_ID" \
    --issuer "$API_ISSUER" \
    --wait

echo "Stapling notarization ticket..."
xcrun stapler staple "$APP_PATH"

echo "Installing to /Applications..."
trash /Applications/Loaf.app 2>/dev/null || true
cp -R "$APP_PATH" /Applications/

# Clean up build to avoid duplicate registrations
trash "$APP_PATH" 2>/dev/null || true

echo "Registering extension..."
pluginkit -a /Applications/Loaf.app/Contents/Extensions/LoafExtension.appex

echo ""
echo "Build complete! App is signed and notarized."
echo ""
echo "To enable the extension:"
echo "  1. System Settings → General → Login Items & Extensions"
echo "  2. Click 'File System Extensions' → Enable 'Loaf'"
