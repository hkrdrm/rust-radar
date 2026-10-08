#!/usr/bin/env bash
# Build rust-radar as a static binary and ship it to the droplet.
#   deploy/deploy.sh install   first time: also sets up the systemd service and nginx site
#   deploy/deploy.sh           every time after: upload the new build and restart
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ ! -f deploy/.env ]]; then
    echo "deploy/.env is missing: copy deploy/.env.example to deploy/.env and fill it in" >&2
    exit 1
fi
# shellcheck source=/dev/null
source deploy/.env
: "${DEPLOY_HOST:?set DEPLOY_HOST in deploy/.env}"
: "${DOMAIN:?set DOMAIN in deploy/.env}"
RADAR_USER=${RADAR_USER:-radar}

mode=${1:-update}
case $mode in
    install | update) ;;
    *) echo "usage: $0 [install|update]" >&2; exit 2 ;;
esac

target=x86_64-unknown-linux-musl
stage=rust-radar-release # upload directory in the remote user's home

rustup target list --installed | grep -qx "$target" || rustup target add "$target"
# ring compiles a little C; the host gcc builds it fine for a static x86_64 binary.
CC_x86_64_unknown_linux_musl=${CC_x86_64_unknown_linux_musl:-gcc} \
    cargo build --release --locked --target "$target"

rsync -az --delete \
    "target/$target/release/rust-radar" web \
    deploy/rust-radar.prod.toml deploy/rust-radar.service \
    deploy/nginx-rust-radar.conf deploy/remote.sh \
    "$DEPLOY_HOST:$stage/"

ssh -t "$DEPLOY_HOST" "sudo bash $stage/remote.sh $mode '$DOMAIN' '$RADAR_USER'"
