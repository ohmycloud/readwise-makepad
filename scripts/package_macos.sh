#!/bin/bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
project_dir="$(cd "$script_dir/.." && pwd)"
cd "$project_dir"

if [[ "$(uname -s)" != "Darwin" ]]; then
    echo "This packaging script requires macOS (codesign, PlistBuddy, and ditto)." >&2
    exit 1
fi

bundle_id="${READWISE_BUNDLE_ID:-org.ohmycloud.readwise}"
signing_identity="${READWISE_CODESIGN_IDENTITY:--}"
app_version="$(awk '
    /^\[package\]$/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && /^version[[:space:]]*=/ {
        value = $0
        sub(/^[^=]*=[[:space:]]*"/, "", value)
        sub(/"[[:space:]]*$/, "", value)
        print value
        exit
    }
' app/Cargo.toml)"

if [[ -z "$app_version" ]]; then
    echo "Could not read readwise-app version from app/Cargo.toml" >&2
    exit 1
fi

cargo makepad desktop build -p readwise-app --release --locked --offline

built_app="$project_dir/target/makepad-desktop/macos/release/readwise-app.app"
dist_dir="$project_dir/target/dist"
dist_app="$dist_dir/ReadWise.app"
archive="$dist_dir/ReadWise-macOS-$app_version.zip"

if [[ ! -x "$built_app/Contents/MacOS/readwise-app" ]]; then
    echo "Expected app bundle was not generated at $built_app" >&2
    exit 1
fi

mkdir -p "$dist_dir"
rm -rf "$dist_app"
ditto "$built_app" "$dist_app"

plist="$dist_app/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleIdentifier $bundle_id" "$plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleName ReadWise" "$plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleDisplayName ReadWise" "$plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $app_version" "$plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $app_version" "$plist"
/usr/libexec/PlistBuddy -c "Add :LSApplicationCategoryType string public.app-category.education" "$plist" 2>/dev/null \
    || /usr/libexec/PlistBuddy -c "Set :LSApplicationCategoryType public.app-category.education" "$plist"
/usr/libexec/PlistBuddy -c "Add :NSHighResolutionCapable bool true" "$plist" 2>/dev/null \
    || /usr/libexec/PlistBuddy -c "Set :NSHighResolutionCapable true" "$plist"

plutil -lint "$plist"

if [[ "$signing_identity" == "-" ]]; then
    codesign --force --deep --sign - "$dist_app"
else
    codesign --force --deep --options runtime --timestamp --sign "$signing_identity" "$dist_app"
fi
codesign --verify --deep --strict --verbose=2 "$dist_app"

rm -f "$archive"
ditto -c -k --sequesterRsrc --keepParent "$dist_app" "$archive"

echo "macOS app: $dist_app"
echo "Distribution archive: $archive"
if [[ "$signing_identity" == "-" ]]; then
    echo "The bundle is ad-hoc signed for local testing. Set READWISE_CODESIGN_IDENTITY to a Developer ID Application identity for external distribution."
fi
