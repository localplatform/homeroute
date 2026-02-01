# Notes pour Claude Code

## Gestion du serveur

- L'API backend est gérée avec PM2
- Le redémarrage de l'API via PM2 est autorisé après modifications du code backend
- Le build frontend (`npm run build`) peut être lancé

## Architecture

- **Frontend**: React + Vite dans `/web`
- **Backend**: Express.js dans `/api`
- **Reverse Proxy**: Rust custom dans `/rust-proxy`
- Les fichiers buildés du frontend vont dans `/web/dist`

## Reverse Proxy (Rust)

- Proxy Rust custom sur le port 443 (HTTPS) et 80 (redirection HTTP→HTTPS)
- Certificats TLS via CA locale (`/var/lib/server-dashboard/ca`)
- Configuration : `/var/lib/server-dashboard/rust-proxy-config.json`
- Hot-reload via SIGHUP (`systemctl reload rust-proxy`)
- Service systemd : `rust-proxy.service`
- **IMPORTANT** : Utiliser l'API du projet (`/api/reverseproxy/*` ou `/api/rust-proxy/*`) pour gérer les routes et recharger le proxy

## Commandes utiles

```bash
# Build frontend
cd /opt/homeroute/web && npm run build

# Build Rust proxy
cd /opt/homeroute/rust-proxy && cargo build --release

# Test import backend
cd /opt/homeroute/api && node -e "import('./src/index.js')"

# Rust proxy tests
cd /opt/homeroute/rust-proxy && cargo test
```
