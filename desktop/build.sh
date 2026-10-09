#!/usr/bin/env bash
# Builds the Linux desktop app: a .deb and an AppImage under target/release/bundle/.
# Needs: cargo install tauri-cli --version '^2' --locked, and the WebKitGTK 4.1 dev packages.
set -euo pipefail
cd "$(dirname "$0")"
cargo tauri build
echo
ls -1 ../target/release/bundle/deb/*.deb ../target/release/bundle/appimage/*.AppImage
