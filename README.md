# Server Dashboard

A comprehensive web-based dashboard for managing home server infrastructure. Monitor and configure network services, DNS/DHCP, ad-blocking, reverse proxy, and file sharing from a single unified interface.

## Features

- **Network Management** - Monitor interfaces, IPv4/IPv6 routing tables, NAT and firewall rules
- **DNS/DHCP** - Dnsmasq integration with DHCP lease tracking
- **Ad-Blocking** - Host-based ad-blocking with whitelist management and blocklist updates
- **Dynamic DNS** - Cloudflare DDNS integration for automatic IPv6 updates
- **Reverse Proxy** - Caddy management with HTTPS certificates and authentication

## Tech Stack

**Frontend**
- React 18 with React Router
- Vite build tool
- Tailwind CSS
- Socket.IO client
- Recharts for data visualization

**Backend**
- Express.js
- MongoDB (optional)
- Socket.IO
- Session-based authentication

**Infrastructure**
- Caddy (reverse proxy)
- Dnsmasq (DNS/DHCP)
- Cloudflare (DDNS)

## Installation

### Prerequisites

- Node.js 22+
- MongoDB (optional)
- Linux server with:
  - Dnsmasq configured
  - Caddy installed

### Setup

1. Clone the repository
   ```bash
   git clone <repository-url>
   cd server-dashboard
   ```

2. Install dependencies
   ```bash
   npm run install:all
   ```

3. Configure environment variables
   ```bash
   cp .env.example .env
   # Edit .env with your configuration
   ```

4. Build the frontend
   ```bash
   npm run build
   ```

## Configuration

Key environment variables in `.env`:

```bash
# API Server
PORT=4000
MONGODB_URI=mongodb://localhost:27017/server-dashboard

# System paths
DNSMASQ_CONFIG=/etc/dnsmasq.d/lan.conf
DNSMASQ_LEASES=/var/lib/misc/dnsmasq.leases

# Ad-blocking
ADBLOCK_HOSTS=/var/lib/dnsmasq/adblock-hosts.txt
ADBLOCK_WHITELIST=/var/lib/dnsmasq/adblock-whitelist.txt

# Cloudflare DDNS
DDNS_CONFIG=/etc/cloudflare-ddns.conf

# Reverse Proxy
CADDY_API_URL=http://localhost:2019
REVERSEPROXY_CONFIG=/var/lib/server-dashboard/reverseproxy-config.json
```

## NPM Scripts

```bash
# Development
npm run dev          # Run API and frontend concurrently
npm run dev:api      # Run API only (with nodemon)
npm run dev:web      # Run frontend only (Vite dev server)

# Production
npm run build        # Build frontend for production

# Code quality
npm run lint         # Run ESLint on both packages
```

## Project Structure

```
server-dashboard/
├── api/                    # Express.js backend
│   └── src/
│       ├── routes/         # API endpoints
│       └── services/       # Business logic
├── web/                    # React frontend
│   └── src/
│       ├── pages/          # Page components
│       └── components/     # Reusable components
└── .env                    # Environment configuration
```

## API Endpoints

| Route | Description |
|-------|-------------|
| `/api/auth` | Authentication (login, logout, session check) |
| `/api/network` | Network interfaces and routing |
| `/api/dns` | DNS/DHCP configuration and leases |
| `/api/nat` | NAT and firewall rules |
| `/api/adblock` | Ad-blocking stats and whitelist |
| `/api/ddns` | Dynamic DNS status and updates |
| `/api/reverseproxy` | Caddy reverse proxy management |

## License

Private project.
