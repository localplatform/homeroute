# Notes pour Claude Code

## Architecture

HomeRoute est un **binaire Rust unifié** qui gère tous les services réseau.

- **Frontend**: React + Vite dans `/web`, servi statiquement par le proxy
- **Backend**: Binaire Rust unique (Cargo workspace) dans `/opt/homeroute/crates/`
- **Service systemd**: `homeroute.service`

### Cargo Workspace

```
crates/
├── homeroute/       # Binaire principal (supervisor + main)
├── hr-common/       # Types partagés, config, EventBus
├── hr-auth/         # Auth (SQLite sessions, YAML users, Argon2id)
├── hr-proxy/        # Reverse proxy HTTPS (TLS/SNI, WebSocket, forward-auth)
├── hr-dns/          # Serveur DNS (UDP/TCP port 53, cache, upstream)
├── hr-dhcp/         # Serveur DHCP (DHCPv4, leases, DORA)
├── hr-ipv6/         # IPv6 RA + DHCPv6 stateless
├── hr-adblock/      # Moteur adblock (FxHashSet, sources, whitelist)
├── hr-ca/           # Autorité de certification locale
├── hr-analytics/    # Capture trafic + agrégation (SQLite)
├── hr-servers/      # Gestion serveurs (monitoring, WoL, scheduler)
├── hr-system/       # Système (énergie, updates, réseau, DDNS Cloudflare)
└── hr-api/          # Routeur API HTTP (axum, routes /api/*, WebSocket)
```

## Gestion du serveur

- Le service systemd `homeroute.service` gère le binaire Rust
- `systemctl restart homeroute` pour redémarrer après modifications
- `systemctl reload homeroute` (SIGHUP) pour hot-reload de la config proxy
- Le build frontend (`cd /opt/homeroute/web && npm run build`) peut être lancé

## Stockage

| Données | Format | Chemin |
|---------|--------|--------|
| Sessions | SQLite | `/opt/homeroute/data/auth.db` |
| Users | YAML | `/opt/homeroute/data/users.yml` |
| Analytics | SQLite | `/opt/homeroute/data/analytics.db` |
| Serveurs | JSON | `/data/servers.json` |
| WoL schedules | JSON | `/data/wol-schedules.json` |
| Config proxy | JSON | `/var/lib/server-dashboard/rust-proxy-config.json` |
| Config DNS/DHCP | JSON | `/var/lib/server-dashboard/dns-dhcp-config.json` |
| Config reverseproxy | JSON | `/var/lib/server-dashboard/reverseproxy-config.json` |
| Certificats CA | PEM | `/var/lib/server-dashboard/ca/` |
| DHCP leases | JSON | `/var/lib/server-dashboard/dhcp-leases` |
| Env config | dotenv | `/opt/homeroute/.env` |

## Ports

| Port | Service |
|------|---------|
| 443 | HTTPS reverse proxy (hr-proxy) |
| 80 | HTTP→HTTPS redirect |
| 53 | DNS (hr-dns, UDP+TCP) |
| 67 | DHCP (hr-dhcp) |
| 3017 | API management (hr-api, interne) |

## Commandes utiles

```bash
# Build tout
cd /opt/homeroute && cargo build --release

# Build frontend
cd /opt/homeroute/web && npm run build

# Tests
cd /opt/homeroute && cargo test

# Redémarrer le service
systemctl restart homeroute

# Hot-reload config proxy
systemctl reload homeroute

# Logs
journalctl -u homeroute -f
```
