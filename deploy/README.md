# Deploying rust-radar

rust-radar runs on the droplet as a systemd service listening on
`127.0.0.1:8090`. nginx sits in front of it, asks for a password, and adds HTTPS
through certbot. The app is never exposed directly to the internet.

| File | Purpose |
|---|---|
| `deploy.sh` | Run from your machine: builds a static binary, uploads it, restarts the service |
| `remote.sh` | The server half of `deploy.sh` (runs as root on the droplet) |
| `rust-radar.prod.toml` | Server config, tuned for a 1 GB droplet shared with other sites |
| `rust-radar.service` | systemd unit: throwaway user, 400 MB memory cap, low CPU priority |
| `nginx-rust-radar.conf` | nginx site template: password prompt plus proxy |
| `.env.example` | Template for `deploy/.env`, which holds your host and domain and is never committed |

## One-time setup

1. **DNS.** Add an `A` record for your subdomain (e.g. `radar.yourdomain.com`)
   that points at the droplet's IP.
2. **Swap**, on the droplet. A 1 GB droplet has no swap by default; this
   gives it a safety net:
   ```sh
   sudo fallocate -l 1G /swapfile && sudo chmod 600 /swapfile
   sudo mkswap /swapfile && sudo swapon /swapfile
   echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab
   ```
3. **Password tool**, on the droplet: `sudo apt install apache2-utils`
4. **Your settings**, on your machine:
   ```sh
   cp deploy/.env.example deploy/.env    # then edit it
   ```
5. **Install**, from your machine: `deploy/deploy.sh install`. This:
   - builds and uploads rust-radar;
   - asks you to choose the map's login password;
   - installs and starts the service;
   - adds the nginx site, after checking with `nginx -t` that your other
     sites' configuration is still valid.
6. **HTTPS**, on the droplet. Do this before logging in, so your password
   never goes over plain HTTP:
   ```sh
   sudo certbot --nginx -d radar.yourdomain.com
   ```
   Choose the redirect option when asked.

Then open `https://radar.yourdomain.com`. The first radar frames arrive
within a minute. The past 2 hours fill in during the first minute or so, and
the archive grows to 72 hours over the following days.

## Updating

```sh
deploy/deploy.sh
```

This rebuilds, uploads and restarts. Your radar archive and nginx config are
left alone.

## Day to day

| Task | Command (on the droplet) |
|---|---|
| Logs | `journalctl -u rust-radar -f` |
| Status and memory | `systemctl status rust-radar` |
| Change the password | `sudo htpasswd /etc/nginx/rust-radar.htpasswd radar` |
| Add another viewer | `sudo htpasswd /etc/nginx/rust-radar.htpasswd theirname` |
| Disk used by the archive | `sudo du -sh /var/lib/private/rust-radar` |

## Sizing

Measured with these settings:

| Phase | Memory |
|---|---|
| Startup backfill (peak) | about 210 MB |
| Idle | about 55 MB |

An uncached replay frame takes about 0.25 s to decode. If you move to a 2 GB
droplet, raise `decoded_cache_frames` to 10 and `backfill_hours` to 6 in
`rust-radar.prod.toml`, then run `deploy/deploy.sh`.
