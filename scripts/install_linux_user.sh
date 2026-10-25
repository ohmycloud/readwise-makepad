#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
source_binary="$script_dir/ReadWise"
source_icon="$script_dir/ReadWise.png"

if [[ "$(uname -s)" != "Linux" ]]; then
    echo "This installer is intended for Linux." >&2
    exit 1
fi

if [[ ! -x "$source_binary" || ! -f "$source_icon" ]]; then
    echo "Run install-user.sh from the extracted ReadWise Linux bundle." >&2
    exit 1
fi

user_bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
user_data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
desktop_dir="$user_data_dir/applications"
icon_dir="$user_data_dir/icons/hicolor/512x512/apps"
installed_binary="$user_bin_dir/readwise"
installed_icon="$icon_dir/org.ohmycloud.readwise.png"
desktop_file="$desktop_dir/org.ohmycloud.readwise.desktop"

mkdir -p "$user_bin_dir" "$desktop_dir" "$icon_dir"
install -m 0755 "$source_binary" "$installed_binary"
install -m 0644 "$source_icon" "$installed_icon"
if [[ -d "$script_dir/third-party/typst-assets" ]]; then
    notice_dir="$user_data_dir/readwise/third-party/typst-assets"
    mkdir -p "$notice_dir"
    install -m 0644 "$script_dir/third-party/typst-assets/"{LICENSE,NOTICE} "$notice_dir/"
fi

cat > "$desktop_file" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=ReadWise
Comment=English EPUB reading and learning
Exec="$installed_binary"
Icon=org.ohmycloud.readwise
Terminal=false
Categories=Education;Utility;
StartupNotify=true
EOF

chmod 0644 "$desktop_file"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$desktop_dir" >/dev/null 2>&1 || true
fi

if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$user_data_dir/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "Installed ReadWise for the current user."
echo "Executable: $installed_binary"
echo "Desktop entry: $desktop_file"
