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

    const allowedUpdates = ['targetHost', 'targetPort', 'enabled', 'localOnly', 'requireAuth', 'authBackend'];
    for (const key of Object.keys(updates)) {
      if (allowedUpdates.includes(key)) {
        if (key === 'targetPort') {
          const port = parseInt(updates[key]);
          if (isNaN(port) || port < 1 || port > 65535) {
            return { success: false, error: 'Invalid port number' };
          }
          config.hosts[hostIndex][key] = port;
        } else if (key === 'authBackend') {
          // Validate authBackend value
          const validBackends = ['none', 'legacy', 'authelia'];
          if (!validBackends.includes(updates[key])) {
            return { success: false, error: 'Invalid authBackend value' };
          }
          // PROTECTION: Ne pas permettre authelia sur code-server
          if (updates[key] === 'authelia' && config.hosts[hostIndex].subdomain === 'code') {
            console.warn('REFUSÉ: code-server ne peut pas utiliser Authelia');
            return { success: false, error: 'code-server ne peut pas utiliser Authelia pour des raisons de sécurité' };
          }
          config.hosts[hostIndex][key] = updates[key];
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

function generateCaddyRoute(host, baseDomain) {
  const domain = host.customDomain || `${host.subdomain}.${baseDomain}`;
  const authServiceUrl = process.env.AUTH_SERVICE_URL || 'http://localhost:9100';

  // Build handlers array
  const handlers = [];

  // Toujours ajouter forward_auth pour injecter les headers d'authentification
  // - Par défaut : endpoint optionnel (ne bloque pas, retourne 200 même si non connecté)
  // - Si requireAuth : endpoint bloquant (retourne 401 si non connecté)
  const authEndpoint = host.requireAuth
    ? '/api/authz/forward-auth'           // Bloque si non authentifié
    : '/api/authz/forward-auth-optional'; // Ne bloque jamais, injecte headers si connecté

  handlers.push({
    handler: 'forward_auth',
    uri: `${authServiceUrl}${authEndpoint}`,
    copy_headers: ['Remote-User', 'Remote-Email', 'Remote-Name', 'Remote-Groups']
  });

  // Proxy to target
  handlers.push({
    handler: 'reverse_proxy',
    upstreams: [{
      dial: `${host.targetHost}:${host.targetPort}`
    }]
  });

  const subrouteHandler = {
    handler: 'subroute',
    routes: [{
      handle: handlers
    }]
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
  const authServiceUrl = process.env.AUTH_SERVICE_URL || 'http://localhost:9100';

  // Filtrer les hosts activés
  const enabledHosts = config.hosts.filter(h => h.enabled);

  const routes = enabledHosts.map(h => generateCaddyRoute(h, config.baseDomain));

  // Add system route for dashboard (proxy.<baseDomain>)
  // Note: L'authentification est gérée côté API via le middleware (cookie auth_session)
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

    // Add auth portal route (auth.<baseDomain>) - custom auth service (no auth required)
    routes.unshift({
      '@id': 'system-auth',
      match: [{ host: [`auth.${config.baseDomain}`] }],
      handle: [{
        handler: 'reverse_proxy',
        upstreams: [{ dial: authServiceUrl.replace('http://', '').replace('https://', '') }]
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
