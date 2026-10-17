#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$(pwd)/target/release-tools/installed/bin:$PATH"
[[ "$(uname -s)" == Darwin ]] || { echo 'iOS packaging requires macOS and Xcode' >&2; exit 1; }
version="$(python3 -c 'import tomllib; print(tomllib.load(open("app/Cargo.toml", "rb"))["package"]["version"])')"
bundle_id="${READWISE_IOS_BUNDLE_ID:-org.ohmycloud.readwise}"
identity="${READWISE_IOS_CODESIGN_IDENTITY:-}"
profile="${READWISE_IOS_PROFILE:-}"
if [[ -n "$identity" || -n "$profile" ]]; then
    [[ -n "$identity" && -f "$profile" ]] || { echo 'Set both READWISE_IOS_CODESIGN_IDENTITY and READWISE_IOS_PROFILE' >&2; exit 1; }
fi
# Install the pinned release tool with scripts/install_release_tools.py first.
cargo makepad apple ios "--org=${bundle_id%.*}" "--app=${bundle_id##*.}" \
    build-device -p readwise-app --release --locked --offline
stage="target/dist/ios-stage"
app="$stage/Payload/ReadWise.app"
mkdir -p "$stage/Payload"
rm -rf "$app"
ditto target/apple/makepad-apple-app/aarch64-apple-ios/release/readwise-app.app "$app"
rm -rf "$app/_CodeSignature"
rm -f "$app/embedded.mobileprovision"
codesign --remove-signature "$app/readwise-app" 2>/dev/null || true
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$app/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$app/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleDisplayName ReadWise' "$app/Info.plist"
suffix=unsigned
if [[ -n "$identity" ]]; then
    security cms -D -i "$profile" > "$stage/profile.plist"
    python3 - "$stage/profile.plist" "$stage/entitlements.plist" "$bundle_id" <<'PY'
import datetime, fnmatch, plistlib, sys
with open(sys.argv[1], 'rb') as f:
    profile = plistlib.load(f)
if profile['ExpirationDate'] < datetime.datetime.now(datetime.timezone.utc).replace(tzinfo=None):
    raise SystemExit('Provisioning profile expired')
entitlements = profile['Entitlements']
prefix = profile['ApplicationIdentifierPrefix'][0]
app_id = prefix + '.' + sys.argv[3]
if not fnmatch.fnmatchcase(app_id, entitlements['application-identifier']):
    raise SystemExit('Profile does not match bundle identifier')
entitlements['application-identifier'] = app_id
if 'keychain-access-groups' in entitlements:
    entitlements['keychain-access-groups'] = [s.replace('*', sys.argv[3]) for s in entitlements['keychain-access-groups']]
with open(sys.argv[2], 'wb') as f:
    plistlib.dump(entitlements, f)
PY
    cp "$profile" "$app/embedded.mobileprovision"
    codesign --force --sign "$identity" --entitlements "$stage/entitlements.plist" --generate-entitlement-der "$app"
    codesign --verify --deep --strict "$app"
    suffix=signed
fi
archive="$(pwd)/target/dist/ReadWise-iOS-arm64-$version-$suffix.ipa"
rm -f "$archive"
(cd "$stage" && /usr/bin/zip -qry "$archive" Payload)
unzip -tq "$archive"
echo "iOS IPA: $archive"
[[ "$suffix" != unsigned ]] || echo 'Unsigned device IPA: re-sign before installing; not an App Store submission.'
