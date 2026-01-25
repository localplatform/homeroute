# Notes pour Claude Code

## Gestion du serveur

- L'API backend est gérée avec PM2
- Le redémarrage de l'API via PM2 est autorisé après modifications du code backend
- Le build frontend (`npm run build`) peut être lancé

## Architecture

- **Frontend**: React + Vite dans `/web`
- **Backend**: Express.js dans `/api`
- Les fichiers buildés du frontend vont dans `/web/dist`

## Reverse Proxy (Caddy)

- Utilise uniquement des certificats individuels Let's Encrypt (HTTP challenge)
- Pas de wildcard certificate
- Le domaine de base sert uniquement de raccourci pour les sous-domaines
- Caddy API sur `localhost:2019`

## Commandes utiles

```bash
# Build frontend
cd /ssd_pool/server-dashboard/web && npm run build

# Test import backend
cd /ssd_pool/server-dashboard/api && node -e "import('./src/index.js')"
```
