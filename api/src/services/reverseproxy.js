import { readFile, writeFile, mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import path from 'path';
import crypto from 'crypto';
import http from 'http';
import tls from 'tls';
import bcrypt from 'bcrypt';

// Environment configuration
const getEnv = () => ({
  CONFIG_FILE: process.env.REVERSEPROXY_CONFIG || '/var/lib/server-dashboard/reverseproxy-config.json',
  CADDY_API: process.env.CADDY_API_URL || 'http://localhost:2019',
  DASHBOARD_PORT: process.env.PORT || '4000'
});

// RFC 1918 private networks + localhost
const LOCAL_NETWORKS = [
  '192.168.0.0/16',
  '10.0.0.0/8',
  '172.16.0.0/12',
  '127.0.0.0/8'
];

// Default config structure
const getDefaultConfig = () => ({
  baseDomain: '',
  authAccounts: [],
  hosts: []
});

async function ensureConfigDir() {
  const { CONFIG_FILE } = getEnv();
  const configDir = path.dirname(CONFIG_FILE);
  if (!existsSync(configDir)) {
    await mkdir(configDir, { recursive: true });
  }
}

// ========== Caddy API Interaction ==========

async function caddyApiRequest(method, apiPath, data = null) {
  const { CADDY_API } = getEnv();
  const url = new URL(apiPath, CADDY_API);

  return new Promise((resolve, reject) => {
    const options = {
      hostname: url.hostname,
      port: url.port || 2019,
      path: url.pathname,
      method,
      headers: { 'Content-Type': 'application/json' }
    };

    const req = http.request(options, (res) => {
      let body = '';
      res.on('data', chunk => body += chunk);
      res.on('end', () => {
        if (res.statusCode >= 200 && res.statusCode < 300) {
          let data = null;
          if (body) {
            try {
              data = JSON.parse(body);
            } catch {
              data = body;
            }
          }
          resolve({ success: true, data, statusCode: res.statusCode });
        } else {
          resolve({ success: false, error: `Caddy API error: ${res.statusCode} - ${body}`, statusCode: res.statusCode });
        }
      });
    });

    req.on('error', (err) => {
      reject(new Error(`Caddy API connection failed: ${err.message}`));
    });

    req.setTimeout(10000, () => {
      req.destroy();
      reject(new Error('Caddy API request timeout'));
    });

    if (data) req.write(JSON.stringify(data));
    req.end();
  });
}

// ========== Configuration Management ==========

// Export loadConfig for auth routes
export async function loadConfig() {
  const { CONFIG_FILE } = getEnv();

  if (!existsSync(CONFIG_FILE)) {
    return getDefaultConfig();
  }

  try {
    const content = await readFile(CONFIG_FILE, 'utf-8');
    const saved = JSON.parse(content);
    // Migration: remove old wildcardCert field
    delete saved.wildcardCert;
    delete saved.cloudflare;
    return { ...getDefaultConfig(), ...saved };
  } catch {
    return getDefaultConfig();
  }
}

// Get base domain for session cookie configuration
export async function getBaseDomain() {
  try {
    const config = await loadConfig();
    return config.baseDomain || null;
  } catch {
    return null;
  }
}

async function saveConfigFile(config) {
  const { CONFIG_FILE } = getEnv();
  await ensureConfigDir();
  await writeFile(CONFIG_FILE, JSON.stringify(config, null, 2));
}

export async function getConfig() {
  try {
    const config = await loadConfig();
    return {
      success: true,
      config: {
        baseDomain: config.baseDomain
      }
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function updateBaseDomain(baseDomain) {
  try {
    if (!baseDomain || typeof baseDomain !== 'string') {
      return { success: false, error: 'Invalid base domain' };
    }

    // Validate domain format
    const domainRegex = /^([a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}$/;
    if (!domainRegex.test(baseDomain)) {
      return { success: false, error: 'Invalid domain format' };
    }

    const config = await loadConfig();
    config.baseDomain = baseDomain.toLowerCase();
    await saveConfigFile(config);

    // Regenerate Caddy config with new base domain
    await applyCaddyConfig();

    return { success: true, message: 'Base domain updated', baseDomain: config.baseDomain };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ========== Host Management ==========

export async function getHosts() {
  try {
    const config = await loadConfig();
    return { success: true, hosts: config.hosts || [] };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function addHost(hostConfig) {
  try {
    const { subdomain, customDomain, targetHost, targetPort } = hostConfig;

    // Validation
    if (!targetHost || !targetPort) {
      return { success: false, error: 'Target host and port are required' };
    }
    if (!subdomain && !customDomain) {
      return { success: false, error: 'Subdomain or custom domain is required' };
    }

    const port = parseInt(targetPort);
    if (isNaN(port) || port < 1 || port > 65535) {
      return { success: false, error: 'Invalid port number' };
    }

    // Validate subdomain format
    if (subdomain) {
      const subdomainRegex = /^[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?$/;
      if (!subdomainRegex.test(subdomain)) {
        return { success: false, error: 'Invalid subdomain format' };
      }
    }

    // Validate custom domain format
    if (customDomain) {
      const domainRegex = /^([a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?\.)+[a-zA-Z]{2,}$/;
      if (!domainRegex.test(customDomain)) {
        return { success: false, error: 'Invalid custom domain format' };
      }
    }

    const config = await loadConfig();

    // Check for duplicates
    const fullDomain = customDomain || `${subdomain}.${config.baseDomain}`;
    const exists = config.hosts.some(h => {
      const existingDomain = h.customDomain || `${h.subdomain}.${config.baseDomain}`;
      return existingDomain.toLowerCase() === fullDomain.toLowerCase();
    });
    if (exists) {
      return { success: false, error: 'Host with this domain already exists' };
    }

    const newHost = {
      id: crypto.randomUUID(),
      subdomain: subdomain?.toLowerCase() || null,
      customDomain: customDomain?.toLowerCase() || null,
      targetHost,
      targetPort: port,
      localOnly: !!hostConfig.localOnly,
      requireAuth: !!hostConfig.requireAuth,
      enabled: true,
      createdAt: new Date().toISOString()
    };

    config.hosts.push(newHost);
    await saveConfigFile(config);

    // Apply to Caddy
    await applyCaddyConfig();

    return { success: true, host: newHost };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function updateHost(hostId, updates) {
  try {
    const config = await loadConfig();
    const hostIndex = config.hosts.findIndex(h => h.id === hostId);

    if (hostIndex === -1) {
      return { success: false, error: 'Host not found' };
    }

    const allowedUpdates = ['targetHost', 'targetPort', 'enabled', 'localOnly', 'requireAuth'];
    for (const key of Object.keys(updates)) {
      if (allowedUpdates.includes(key)) {
        if (key === 'targetPort') {
          const port = parseInt(updates[key]);
          if (isNaN(port) || port < 1 || port > 65535) {
            return { success: false, error: 'Invalid port number' };
          }
          config.hosts[hostIndex][key] = port;
        } else {
          config.hosts[hostIndex][key] = updates[key];
        }
      }
    }

    await saveConfigFile(config);
    await applyCaddyConfig();

    return { success: true, host: config.hosts[hostIndex] };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function deleteHost(hostId) {
  try {
    const config = await loadConfig();
    const hostIndex = config.hosts.findIndex(h => h.id === hostId);

    if (hostIndex === -1) {
      return { success: false, error: 'Host not found' };
    }

    const deletedHost = config.hosts.splice(hostIndex, 1)[0];
    await saveConfigFile(config);
    await applyCaddyConfig();

    return { success: true, message: 'Host deleted', host: deletedHost };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function toggleHost(hostId, enabled) {
  try {
    return await updateHost(hostId, { enabled: !!enabled });
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ========== Auth Accounts Management ==========

export async function getAuthAccounts() {
  try {
    const config = await loadConfig();
    // Return accounts without password hashes
    const accounts = (config.authAccounts || []).map(a => ({
      id: a.id,
      username: a.username,
      createdAt: a.createdAt
    }));
    return { success: true, accounts };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function addAuthAccount(username, password) {
  try {
    if (!username || !password) {
      return { success: false, error: 'Username and password are required' };
    }

    if (username.length < 3) {
      return { success: false, error: 'Username must be at least 3 characters' };
    }

    if (password.length < 6) {
      return { success: false, error: 'Password must be at least 6 characters' };
    }

    const config = await loadConfig();
    if (!config.authAccounts) config.authAccounts = [];

    // Check for duplicate username
    if (config.authAccounts.some(a => a.username.toLowerCase() === username.toLowerCase())) {
      return { success: false, error: 'Username already exists' };
    }

    // Hash password with bcrypt
    const passwordHash = await bcrypt.hash(password, 10);

    const newAccount = {
      id: crypto.randomUUID(),
      username: username.toLowerCase(),
      passwordHash,
      createdAt: new Date().toISOString()
    };

    config.authAccounts.push(newAccount);
    await saveConfigFile(config);

    // Reload Caddy to apply auth changes
    await applyCaddyConfig();

    return {
      success: true,
      account: { id: newAccount.id, username: newAccount.username, createdAt: newAccount.createdAt }
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function updateAuthAccount(accountId, updates) {
  try {
    const config = await loadConfig();
    if (!config.authAccounts) config.authAccounts = [];

    const accountIndex = config.authAccounts.findIndex(a => a.id === accountId);
    if (accountIndex === -1) {
      return { success: false, error: 'Account not found' };
    }

    // Update username if provided
    if (updates.username) {
      if (updates.username.length < 3) {
        return { success: false, error: 'Username must be at least 3 characters' };
      }
      // Check for duplicate
      const duplicate = config.authAccounts.some(
        (a, i) => i !== accountIndex && a.username.toLowerCase() === updates.username.toLowerCase()
      );
      if (duplicate) {
        return { success: false, error: 'Username already exists' };
      }
      config.authAccounts[accountIndex].username = updates.username.toLowerCase();
    }

    // Update password if provided
    if (updates.password) {
      if (updates.password.length < 6) {
        return { success: false, error: 'Password must be at least 6 characters' };
      }
      config.authAccounts[accountIndex].passwordHash = await bcrypt.hash(updates.password, 10);
    }

    await saveConfigFile(config);
    await applyCaddyConfig();

    const account = config.authAccounts[accountIndex];
    return {
      success: true,
      account: { id: account.id, username: account.username, createdAt: account.createdAt }
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function deleteAuthAccount(accountId) {
  try {
    const config = await loadConfig();
    if (!config.authAccounts) config.authAccounts = [];

    const accountIndex = config.authAccounts.findIndex(a => a.id === accountId);
    if (accountIndex === -1) {
      return { success: false, error: 'Account not found' };
    }

    const deleted = config.authAccounts.splice(accountIndex, 1)[0];
    await saveConfigFile(config);
    await applyCaddyConfig();

    return {
      success: true,
      message: 'Account deleted',
      account: { id: deleted.id, username: deleted.username }
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ========== Caddy Configuration Generation ==========

// HTML splash page shown for 2 seconds before proxying
function getSplashHtml(targetDomain) {
  return `<!DOCTYPE html>
<html lang="fr">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Connexion securisee...</title>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body {
      min-height: 100vh;
      background: #111827;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
      color: white;
    }
    .container { text-align: center; }
    .flow {
      display: flex;
      align-items: center;
      justify-content: center;
      gap: 16px;
      margin-bottom: 32px;
    }
    .icon-box {
      width: 64px;
      height: 64px;
      background: #1f2937;
      border-radius: 12px;
      display: flex;
      align-items: center;
      justify-content: center;
      border: 1px solid #374151;
    }
    .shield-box {
      width: 80px;
      height: 80px;
      background: rgba(37, 99, 235, 0.2);
      border: 2px solid #3b82f6;
      border-radius: 12px;
      display: flex;
      align-items: center;
      justify-content: center;
      position: relative;
    }
    .ping {
      position: absolute;
      top: -4px;
      right: -4px;
      width: 16px;
      height: 16px;
      background: #22c55e;
      border-radius: 50%;
      animation: ping 1s infinite;
    }
    @keyframes ping {
      0% { transform: scale(1); opacity: 1; }
      75%, 100% { transform: scale(1.5); opacity: 0; }
    }
    .arrows {
      display: flex;
      gap: 4px;
    }
    .arrow {
      color: #3b82f6;
      animation: pulse 1s infinite;
    }
    .arrow:nth-child(2) { animation-delay: 0.15s; }
    .arrow:nth-child(3) { animation-delay: 0.3s; }
    .arrows-green .arrow { color: #22c55e; }
    .arrows-green .arrow:nth-child(1) { animation-delay: 0.45s; }
    .arrows-green .arrow:nth-child(2) { animation-delay: 0.6s; }
    .arrows-green .arrow:nth-child(3) { animation-delay: 0.75s; }
    @keyframes pulse {
      0%, 100% { opacity: 0.3; }
      50% { opacity: 1; }
    }
    h1 { font-size: 1.5rem; margin-bottom: 8px; }
    .subtitle { color: #9ca3af; margin-bottom: 24px; }
    .progress-bar {
      width: 256px;
      height: 6px;
      background: #1f2937;
      border-radius: 9999px;
      overflow: hidden;
      margin: 0 auto;
    }
    .progress-fill {
      height: 100%;
      background: linear-gradient(to right, #3b82f6, #22c55e);
      border-radius: 9999px;
      animation: progress 2s ease-out forwards;
    }
    @keyframes progress {
      0% { width: 0%; }
      100% { width: 100%; }
    }
    .tech { margin-top: 32px; font-size: 12px; color: #4b5563; font-family: monospace; }
    .domain { margin-top: 16px; font-size: 13px; color: #6b7280; }
    svg { width: 32px; height: 32px; }
    .shield svg { width: 40px; height: 40px; color: #60a5fa; }
    .user svg { color: #6b7280; }
    .server svg { color: #9ca3af; }
  </style>
</head>
<body>
  <div class="container">
    <div class="flow">
      <div class="icon-box user">
        <svg fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
          <circle cx="12" cy="8" r="4"/><path d="M4 20c0-4 4-6 8-6s8 2 8 6"/>
        </svg>
      </div>
      <div class="arrows">
        <span class="arrow">&#x2192;</span>
        <span class="arrow">&#x2192;</span>
        <span class="arrow">&#x2192;</span>
      </div>
      <div class="shield-box">
        <div class="ping"></div>
        <div class="shield">
          <svg fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
          </svg>
        </div>
      </div>
      <div class="arrows arrows-green">
        <span class="arrow">&#x2192;</span>
        <span class="arrow">&#x2192;</span>
        <span class="arrow">&#x2192;</span>
      </div>
      <div class="icon-box server">
        <svg fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
          <rect x="2" y="2" width="20" height="8" rx="2"/>
          <rect x="2" y="14" width="20" height="8" rx="2"/>
          <circle cx="6" cy="6" r="1" fill="currentColor"/>
          <circle cx="6" cy="18" r="1" fill="currentColor"/>
        </svg>
      </div>
    </div>
    <h1>Reverse Proxy</h1>
    <p class="subtitle">Connexion securisee en cours...</p>
    <div class="progress-bar"><div class="progress-fill"></div></div>
    <p class="tech">TLS 1.3 | HTTP/2 | Caddy Server</p>
    <p class="domain">${targetDomain}</p>
  </div>
  <script>
    document.cookie = "proxy_splash=1; path=/; max-age=3600; SameSite=Lax";
    setTimeout(function() {
      window.location.reload();
    }, 2000);
  </script>
</body>
</html>`;
}

// Splash HTML for auth routes - uses short-lived cookie (10 seconds, shows every fresh visit)
function getAuthSplashHtml(targetDomain) {
  // Same visual as regular splash, but with very short cookie (10s for slow connections)
  const baseHtml = getSplashHtml(targetDomain);
  // Replace the long cookie with a 10-second cookie (using regex for flexible matching)
  return baseHtml.replace(
    /document\.cookie\s*=\s*"proxy_splash=1;[^"]*"/,
    'document.cookie = "proxy_auth_splash=1; path=/; max-age=10; SameSite=Lax"'
  );
}

function generateCaddyRoute(host, baseDomain) {
  const { DASHBOARD_PORT } = getEnv();
  const domain = host.customDomain || `${host.subdomain}.${baseDomain}`;

  const reverseProxyHandler = {
    handler: 'reverse_proxy',
    upstreams: [{
      dial: `${host.targetHost}:${host.targetPort}`
    }]
  };

  // Splash screen handler for auth routes (short-lived cookie, shows every fresh visit)
  const authSplashHandler = {
    handler: 'static_response',
    status_code: 200,
    headers: {
      'Content-Type': ['text/html; charset=utf-8'],
      'Cache-Control': ['no-cache, no-store, must-revalidate']
    },
    body: getAuthSplashHtml(domain)
  };

  // Build subroute logic based on auth and splash requirements
  const routes = [];

  if (host.requireAuth) {
    // For auth routes: splash shown on EVERY fresh page visit (10s cookie)

    // Route 1: Authenticated -> proxy to target (skip splash for all requests)
    routes.push({
      match: [{
        header: { Cookie: ['*dashboard.sid=*'] }
      }],
      handle: [reverseProxyHandler]
    });

    // Route 2: API/assets/WebSocket requests -> proxy to dashboard without splash
    // (These are not page navigations, shouldn't get splash)
    routes.push({
      match: [{
        path_regexp: {
          pattern: '^/(api/|socket\\.io/|assets/|.*\\.(js|css|svg|png|jpg|jpeg|gif|ico|woff|woff2|ttf|eot|map|json)$)'
        }
      }],
      handle: [{
        handler: 'reverse_proxy',
        upstreams: [{ dial: `localhost:${DASHBOARD_PORT}` }]
      }]
    });

    // Route 3: Page navigation without splash cookie -> show splash
    routes.push({
      match: [{ not: [{ header: { Cookie: ['*proxy_auth_splash=1*'] } }] }],
      handle: [authSplashHandler]
    });

    // Route 4: Splash seen but not authenticated -> proxy to dashboard (shows login)
    routes.push({
      handle: [{
        handler: 'reverse_proxy',
        upstreams: [{ dial: `localhost:${DASHBOARD_PORT}` }]
      }]
    });
  } else {
    // For non-auth routes: no splash, just proxy directly
    routes.push({
      handle: [reverseProxyHandler]
    });
  }

  const subrouteHandler = {
    handler: 'subroute',
    routes: routes
  };

  // If localOnly, wrap everything in IP restriction
  if (host.localOnly) {
    return {
      '@id': host.id,
      match: [{ host: [domain] }],
      handle: [{
        handler: 'subroute',
        routes: [
          {
            match: [{ remote_ip: { ranges: LOCAL_NETWORKS } }],
            handle: [subrouteHandler]
          },
          {
            handle: [{
              handler: 'error',
              status_code: 403
            }]
          }
        ]
      }],
      terminal: true
    };
  }

  return {
    '@id': host.id,
    match: [{ host: [domain] }],
    handle: [subrouteHandler],
    terminal: true
  };
}

function generateCaddyConfig(config) {
  const { DASHBOARD_PORT } = getEnv();
  const enabledHosts = config.hosts.filter(h => h.enabled);
  const routes = enabledHosts.map(h => generateCaddyRoute(h, config.baseDomain));

  // Add system route for dashboard (proxy.<baseDomain>)
  if (config.baseDomain) {
    routes.unshift({
      '@id': 'system-dashboard',
      match: [{ host: [`proxy.${config.baseDomain}`] }],
      handle: [{
        handler: 'reverse_proxy',
        upstreams: [{ dial: `localhost:${DASHBOARD_PORT}` }]
      }],
      terminal: true
    });
  }

  const caddyConfig = {
    admin: {
      listen: 'localhost:2019'
    },
    apps: {
      http: {
        servers: {
          srv0: {
            listen: [':80', ':443'],
            routes
          }
        }
      }
    }
  };

  // Add default TLS automation (Let's Encrypt with HTTP challenge)
  if (routes.length > 0) {
    caddyConfig.apps.tls = {
      automation: {
        policies: [{
          issuers: [{
            module: 'acme'
          }]
        }]
      }
    };
  }

  return caddyConfig;
}

async function applyCaddyConfig() {
  try {
    const config = await loadConfig();

    // Skip if no hosts and no baseDomain (system route)
    if (config.hosts.length === 0 && !config.baseDomain) {
      return { success: true, message: 'No configuration to apply' };
    }

    const caddyConfig = generateCaddyConfig(config);
    const result = await caddyApiRequest('POST', '/load', caddyConfig);

    if (!result.success) {
      console.error('Failed to apply Caddy config:', result.error);
      return result;
    }

    return { success: true, message: 'Caddy configuration applied' };
  } catch (error) {
    console.error('Error applying Caddy config:', error);
    return { success: false, error: error.message };
  }
}

// ========== Caddy Status ==========

export async function getCaddyStatus() {
  try {
    const result = await caddyApiRequest('GET', '/config/');

    return {
      success: true,
      running: result.success,
      hasConfig: result.data !== null,
      error: result.success ? null : result.error
    };
  } catch (error) {
    return {
      success: true,
      running: false,
      hasConfig: false,
      error: error.message
    };
  }
}

export async function reloadCaddy() {
  try {
    const result = await applyCaddyConfig();
    return result;
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function renewCertificates() {
  try {
    // Caddy handles renewal automatically, but we can force it by reloading config
    const result = await applyCaddyConfig();

    if (result.success) {
      return { success: true, message: 'Certificate renewal triggered' };
    }

    return result;
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ========== System Route Status ==========

export async function getSystemRouteStatus() {
  try {
    const config = await loadConfig();
    const { DASHBOARD_PORT } = getEnv();

    if (!config.baseDomain) {
      return {
        success: true,
        configured: false,
        domain: null,
        port: DASHBOARD_PORT
      };
    }

    return {
      success: true,
      configured: true,
      domain: `proxy.${config.baseDomain}`,
      port: DASHBOARD_PORT
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ========== Certificate Status ==========

async function checkCertificate(domain) {
  return new Promise((resolve) => {
    const socket = tls.connect({
      host: domain,
      port: 443,
      servername: domain,  // SNI - important pour les certificats multi-domaines
      rejectUnauthorized: false,
      timeout: 5000
    }, () => {
      const cert = socket.getPeerCertificate();
      socket.destroy();

      if (cert && cert.valid_to) {
        const expiresAt = new Date(cert.valid_to);
        const now = new Date();
        const daysRemaining = Math.floor((expiresAt - now) / (1000 * 60 * 60 * 24));

        resolve({
          valid: daysRemaining > 0,
          expiresAt: expiresAt.toISOString(),
          daysRemaining,
          issuer: cert.issuer?.O || 'Unknown',
          subject: cert.subject?.CN || domain
        });
      } else {
        resolve({ valid: false, error: 'No certificate found' });
      }
    });

    socket.on('error', (err) => {
      resolve({ valid: false, error: err.message || 'Connection failed' });
    });

    socket.on('timeout', () => {
      socket.destroy();
      resolve({ valid: false, error: 'Timeout' });
    });
  });
}

export async function getCertificatesStatus() {
  try {
    const config = await loadConfig();
    const statuses = {};

    // Check all hosts
    for (const host of config.hosts) {
      const domain = host.customDomain || `${host.subdomain}.${config.baseDomain}`;
      if (host.enabled) {
        statuses[host.id] = await checkCertificate(domain);
      } else {
        statuses[host.id] = { valid: false, error: 'Host disabled' };
      }
    }

    // Check system route
    if (config.baseDomain) {
      statuses['system-dashboard'] = await checkCertificate(`proxy.${config.baseDomain}`);
    }

    return { success: true, certificates: statuses };
  } catch (error) {
    return { success: false, error: error.message };
  }
}
