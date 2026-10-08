#!/usr/bin/env bash
# Server half of deploy.sh. Runs as root on the droplet from the upload directory.
set -euo pipefail
cd "$(dirname "$0")"
mode=$1 domain=$2 radar_user=$3

site=/etc/nginx/sites-available/rust-radar
passwords=/etc/nginx/rust-radar.htpasswd

install_files() {
    install -d /opt/rust-radar
    # Copy then rename, so the running binary is never overwritten in place.
    install -m 755 rust-radar /opt/rust-radar/rust-radar.new
    mv /opt/rust-radar/rust-radar.new /opt/rust-radar/rust-radar
    rsync -a --delete web/ /opt/rust-radar/web/
    install -m 644 rust-radar.prod.toml /opt/rust-radar/rust-radar.toml
}

if [[ $mode == install ]]; then
    if [[ ! -f $passwords ]]; then
        if ! command -v htpasswd > /dev/null; then
            echo "htpasswd not found: run 'sudo apt install apache2-utils' and try again" >&2
            exit 1
        fi
        echo "Choose the password for logging in to the map as '$radar_user':"
        htpasswd -c "$passwords" "$radar_user"
        chown root:www-data "$passwords"
        chmod 640 "$passwords"
    fi

    install_files
    install -m 644 rust-radar.service /etc/systemd/system/rust-radar.service
    systemctl daemon-reload
    systemctl enable rust-radar
    systemctl restart rust-radar

    if [[ -e $site ]]; then
        echo "$site already exists; leaving it alone (certbot may have added HTTPS to it)"
    else
        sed "s/__DOMAIN__/$domain/" nginx-rust-radar.conf > "$site"
        ln -sf "$site" /etc/nginx/sites-enabled/rust-radar
    fi
    if nginx -t; then
        systemctl reload nginx
    else
        echo "nginx config test failed; disabling the rust-radar site so your other sites keep working" >&2
        rm -f /etc/nginx/sites-enabled/rust-radar
        exit 1
    fi
    echo
    echo "Installed. Next, add HTTPS before logging in: sudo certbot --nginx -d $domain"
else
    install_files
    systemctl restart rust-radar
fi

systemctl --no-pager --lines=5 status rust-radar
