#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
project_dir="$(cd "$script_dir/.." && pwd)"
cd "$project_dir"

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "This packaging script must run on Linux so the artifact uses the native Linux ABI." >&2
    exit 1
fi

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

case "$(uname -m)" in
    x86_64) package_arch="x86_64" ;;
    aarch64|arm64) package_arch="aarch64" ;;
    *) package_arch="$(uname -m)" ;;
esac

cargo makepad desktop build -p readwise-app --release --locked --offline

built_binary="$project_dir/target/release/readwise-app"
dist_dir="$project_dir/target/dist"
bundle_name="ReadWise-linux-$package_arch-$app_version"
bundle_dir="$dist_dir/$bundle_name"
archive="$dist_dir/$bundle_name.tar.gz"

if [[ ! -x "$built_binary" ]]; then
    echo "Expected release binary was not generated at $built_binary" >&2
    exit 1
fi

mkdir -p "$dist_dir"
rm -rf "$bundle_dir"
rm -f "$archive"
mkdir -p "$bundle_dir"

install -m 0755 "$built_binary" "$bundle_dir/ReadWise"
install -m 0644 app/resources/icon_512.png "$bundle_dir/ReadWise.png"
install -m 0755 scripts/install_linux_user.sh "$bundle_dir/install-user.sh"

cat > "$bundle_dir/README.txt" <<'EOF'
ReadWise for Linux
==================

Run without installing:

    ./ReadWise

Install for the current user (no root privileges required):

    ./install-user.sh

The installer copies the executable into ~/.local/bin by default and creates
the XDG desktop entry and icon. Set XDG_BIN_HOME or XDG_DATA_HOME before
running it to select different user directories.
EOF

tar -C "$dist_dir" -czf "$archive" "$bundle_name"

test -x "$bundle_dir/ReadWise"
test -x "$bundle_dir/install-user.sh"
test -s "$bundle_dir/ReadWise.png"
tar -tzf "$archive" >/dev/null

echo "Linux bundle: $bundle_dir"
echo "Distribution archive: $archive"
echo "The archive is built for $package_arch and contains a user-level desktop installer."
