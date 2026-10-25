#!/usr/bin/env bash
# Build Topcoat's matching asset bundle, then start the local Web application.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
if ! command -v topcoat >/dev/null 2>&1; then
    printf '%s\n' '缺少 Topcoat CLI，请先安装：cargo install topcoat-cli --version 0.11.0 --locked' >&2
    exit 1
fi
if [[ "$(topcoat --version)" != "topcoat 0.11.0" ]]; then
    printf '%s\n' '请使用与项目匹配的 Topcoat CLI 0.11.0。' >&2
    exit 1
fi
topcoat asset bundle -p readwise-web
exec cargo run --locked -p readwise-web
