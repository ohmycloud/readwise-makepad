#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
project_dir="$(cd "$script_dir/.." && pwd)"
cd "$project_dir"

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

android_abi="${READWISE_ANDROID_ABI:-aarch64}"
version_code="${READWISE_ANDROID_VERSION_CODE:-2}"
dist_dir="$project_dir/target/dist"

common_args=(
    "--abi=$android_abi"
    "--version-code=$version_code"
)

if [[ -n "${READWISE_ANDROID_SDK_PATH:-}" ]]; then
    common_args+=("--sdk-path=$READWISE_ANDROID_SDK_PATH")
fi

cargo makepad android "${common_args[@]}" build \
    -p readwise-app --release --locked --offline

# cargo-makepad currently generates a fixed MakepadApp subclass and does not
# include app-owned Java sources. Compile our SAF picker subclass into its
# persistent Java output, then run the packager once more. On the second pass
# cargo-makepad reuses those classes while rebuilding/signing the APK.
android_build_root="$project_dir/target/android/makepad-android-apk/readwise_app"
generated_activity="$android_build_root/tmp/org/ohmycloud/readwise/MakepadApp.java"
java_classes="$android_build_root/java"
activity_source="$project_dir/app/android/MakepadApp.java"

if [[ ! -f "$generated_activity" || ! -d "$java_classes" ]]; then
    echo "cargo-makepad did not generate the expected Android Java staging tree" >&2
    exit 1
fi

if [[ -n "${READWISE_ANDROID_SDK_PATH:-}" ]]; then
    android_sdk_dir="$(cd "$READWISE_ANDROID_SDK_PATH" && pwd)"
else
    android_jar_guess="$(find "$HOME/.cargo/git/checkouts" -type f \
        -path '*/tools/cargo_makepad/android_33_*/platforms/*/android.jar' \
        -print -quit 2>/dev/null)"
    if [[ -z "$android_jar_guess" ]]; then
        echo "Could not locate cargo-makepad's Android SDK; set READWISE_ANDROID_SDK_PATH" >&2
        exit 1
    fi
    android_sdk_dir="$(dirname "$(dirname "$(dirname "$android_jar_guess")")")"
fi

android_jar="$(find "$android_sdk_dir/platforms" -type f -name android.jar -print -quit)"
if [[ -z "$android_jar" ]]; then
    echo "Could not locate android.jar under $android_sdk_dir/platforms" >&2
    exit 1
fi

install -m 0644 "$activity_source" "$generated_activity"
cargo makepad android "${common_args[@]}" javac \
    -source 1.8 \
    -target 1.8 \
    -Xlint:-options \
    -classpath "$android_jar:$java_classes" \
    -d "$java_classes" \
    "$generated_activity"

activity_class="$java_classes/org/ohmycloud/readwise/MakepadApp.class"
if [[ ! -f "$activity_class" ]] || ! grep -a -q 'nativeOnEpubPickerResult' "$activity_class"; then
    echo "Android EPUB picker Activity bridge did not compile correctly" >&2
    exit 1
fi

cargo makepad android "${common_args[@]}" build \
    -p readwise-app --release --locked --offline

built_apk="$project_dir/target/android/makepad-android-apk/readwise_app/apk/read_wise.apk"
dist_apk="$dist_dir/ReadWise-android-$android_abi-$app_version.apk"

if [[ ! -f "$built_apk" ]]; then
    echo "Expected APK was not generated at $built_apk" >&2
    exit 1
fi

built_dex="$android_build_root/apk/classes.dex"
if [[ ! -f "$built_dex" ]] \
    || ! grep -a -q 'openEpubPicker' "$built_dex" \
    || ! grep -a -q 'nativeOnEpubPickerResult' "$built_dex"; then
    echo "Generated APK is missing the Android EPUB picker bridge" >&2
    exit 1
fi

mkdir -p "$dist_dir"
install -m 0644 "$built_apk" "$dist_apk"
test -s "$dist_apk"

echo "Android APK: $dist_apk"
echo "The APK uses cargo-makepad's development signature and is intended for device testing."

release_keystore="${READWISE_ANDROID_KEYSTORE:-}"
if [[ -z "$release_keystore" ]]; then
    echo "Set READWISE_ANDROID_KEYSTORE and MAKEPAD_KEYSTORE_PASS to also build a Play Store AAB."
    exit 0
fi

if [[ ! -f "$release_keystore" ]]; then
    echo "Release keystore does not exist: $release_keystore" >&2
    exit 1
fi

if [[ -z "${MAKEPAD_KEYSTORE_PASS:-}" ]]; then
    echo "MAKEPAD_KEYSTORE_PASS is required when READWISE_ANDROID_KEYSTORE is set." >&2
    exit 1
fi

signing_args=("--keystore=$release_keystore")
if [[ -n "${READWISE_ANDROID_KEY_ALIAS:-}" ]]; then
    signing_args+=("--keystore-key-alias=$READWISE_ANDROID_KEY_ALIAS")
fi

cargo makepad android "${common_args[@]}" "${signing_args[@]}" build-aab \
    -p readwise-app --release --locked --offline

built_aab="$project_dir/target/android/makepad-android-aab/readwise_app/read_wise.aab"
dist_aab="$dist_dir/ReadWise-android-$android_abi-$app_version.aab"

if [[ ! -f "$built_aab" ]]; then
    echo "Expected AAB was not generated at $built_aab" >&2
    exit 1
fi

install -m 0644 "$built_aab" "$dist_aab"
test -s "$dist_aab"

echo "Play Store AAB: $dist_aab"
echo "The AAB was signed with the configured release keystore."
