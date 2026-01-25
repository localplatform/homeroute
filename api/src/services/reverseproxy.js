import { readFile, writeFile, mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import path from 'path';
import crypto from 'crypto';
import http from 'http';
import tls from 'tls';

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
  environments: [
    { id: 'prod', name: 'Production', prefix: '', apiPrefix: 'api', isDefault: true },
    { id: 'dev', name: 'Development', prefix: 'dev', apiPrefix: 'api.dev', isDefault: false }
  ],
  applications: [],
  hosts: [],
  cloudflare: {
    enabled: false,
    wildcardDomains: []
  }
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

// Migrate old application structure (frontend/api global) to new structure (endpoints per env)
function migrateApplicationStructure(app) {
  // If already has endpoints structure, no migration needed
  if (app.endpoints && typeof app.endpoints === 'object') {
    // Check if we need to migrate api → apis[] within endpoints
    let needsMigration = false;
    for (const envEndpoints of Object.values(app.endpoints)) {
      if (envEndpoints && envEndpoints.api && !envEndpoints.apis) {
        needsMigration = true;
        break;
      }
    }

    if (needsMigration) {
      // Migrate api to apis[] for each environment
      for (const [envId, envEndpoints] of Object.entries(app.endpoints)) {
        if (envEndpoints && envEndpoints.api && !envEndpoints.apis) {
          envEndpoints.apis = [{
            slug: '',
            targetHost: envEndpoints.api.targetHost,
            targetPort: envEndpoints.api.targetPort,
            localOnly: !!envEndpoints.api.localOnly,
            requireAuth: !!envEndpoints.api.requireAuth
          }];
          delete envEndpoints.api;
        }
      }
    }
    return app;
  }

  // Convert old structure to new structure
  const endpoints = {};
  const envIds = app.environments || ['prod'];

  for (const envId of envIds) {
    endpoints[envId] = {
      frontend: app.frontend ? {
        targetHost: app.frontend.targetHost || 'localhost',
        targetPort: app.frontend.targetPort || 3000,
        localOnly: !!app.frontend.localOnly,
        requireAuth: !!app.frontend.requireAuth
      } : null,
      apis: app.api ? [{
        slug: '',
        targetHost: app.api.targetHost || 'localhost',
        targetPort: app.api.targetPort || 3001,
        localOnly: !!app.api.localOnly,
        requireAuth: !!app.api.requireAuth
      }] : []
    };
  }

  // Return new structure (remove old fields)
  const { frontend, api, environments, ...rest } = app;
  return {
    ...rest,
    endpoints,
    enabled: app.enabled !== false
  };
}

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

    // Merge with defaults to ensure all fields exist
    const defaultConfig = getDefaultConfig();
    const config = {
      ...defaultConfig,
      ...saved,
      // Ensure nested objects have defaults
      environments: saved.environments || defaultConfig.environments,
      applications: saved.applications || defaultConfig.applications,
      cloudflare: { ...defaultConfig.cloudflare, ...(saved.cloudflare || {}) }
    };

    // Migrate applications to new structure if needed
    if (config.applications && config.applications.length > 0) {
      config.applications = config.applications.map(migrateApplicationStructure);
    }

    return config;
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

// ========== Environment Management ==========

export async function getEnvironments() {
  try {
    const config = await loadConfig();
    return { success: true, environments: config.environments || [] };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function addEnvironment(envConfig) {
  try {
    const { name, prefix, apiPrefix } = envConfig;

    if (!name || typeof prefix !== 'string' || typeof apiPrefix !== 'string') {
      return { success: false, error: 'Name, prefix and apiPrefix are required' };
    }

    const config = await loadConfig();

    // Check for duplicate id
    const id = name.toLowerCase().replace(/[^a-z0-9]/g, '-');
    if (config.environments.some(e => e.id === id)) {
      return { success: false, error: 'Environment with this name already exists' };
    }

    const newEnv = {
      id,
      name,
      prefix: prefix.toLowerCase(),
      apiPrefix: apiPrefix.toLowerCase(),
      isDefault: false
    };

    config.environments.push(newEnv);
    await saveConfigFile(config);

    return { success: true, environment: newEnv };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function updateEnvironment(envId, updates) {
  try {
    const config = await loadConfig();
    const envIndex = config.environments.findIndex(e => e.id === envId);

    if (envIndex === -1) {
      return { success: false, error: 'Environment not found' };
    }

    const allowedUpdates = ['name', 'prefix', 'apiPrefix', 'isDefault'];
    for (const key of Object.keys(updates)) {
      if (allowedUpdates.includes(key)) {
        if (key === 'isDefault' && updates[key]) {
          // Unset other defaults
          config.environments.forEach(e => e.isDefault = false);
        }
        config.environments[envIndex][key] = updates[key];
      }
    }

    await saveConfigFile(config);
    await applyCaddyConfig();

    return { success: true, environment: config.environments[envIndex] };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function deleteEnvironment(envId) {
  try {
    const config = await loadConfig();
    const envIndex = config.environments.findIndex(e => e.id === envId);

    if (envIndex === -1) {
      return { success: false, error: 'Environment not found' };
    }

    // Check if any apps use this environment
    const appsUsingEnv = config.applications.filter(a => a.environments.includes(envId));
    if (appsUsingEnv.length > 0) {
      return { success: false, error: `Cannot delete: ${appsUsingEnv.length} application(s) use this environment` };
    }

    const deletedEnv = config.environments.splice(envIndex, 1)[0];
    await saveConfigFile(config);

    return { success: true, message: 'Environment deleted', environment: deletedEnv };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ========== Application Management ==========

export async function getApplications() {
  try {
    const config = await loadConfig();
    return { success: true, applications: config.applications || [] };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function addApplication(appConfig) {
  try {
    const { name, slug, endpoints } = appConfig;

    if (!name || !slug) {
      return { success: false, error: 'Name and slug are required' };
    }

    // Validate slug format
    const slugRegex = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;
    if (!slugRegex.test(slug.toLowerCase())) {
      return { success: false, error: 'Invalid slug format' };
    }

    if (!endpoints || typeof endpoints !== 'object' || Object.keys(endpoints).length === 0) {
      return { success: false, error: 'At least one environment endpoint is required' };
    }

    const config = await loadConfig();

    // Check for duplicate slug
    if (config.applications.some(a => a.slug === slug.toLowerCase())) {
      return { success: false, error: 'Application with this slug already exists' };
    }

    // Validate and sanitize endpoints for each environment
    const validEndpoints = {};
    for (const [envId, envEndpoints] of Object.entries(endpoints)) {
      // Check if environment exists
      if (!config.environments.some(e => e.id === envId)) {
        continue; // Skip invalid environments
      }

      // Validate frontend is required
      if (!envEndpoints.frontend || !envEndpoints.frontend.targetHost || !envEndpoints.frontend.targetPort) {
        return { success: false, error: `Frontend target is required for environment ${envId}` };
      }

      // Validate frontend port
      const frontendPort = parseInt(envEndpoints.frontend.targetPort);
      if (isNaN(frontendPort) || frontendPort < 1 || frontendPort > 65535) {
        return { success: false, error: `Invalid frontend port for environment ${envId}` };
      }

      // Process APIs (new format: apis[] array)
      let apis = [];
      if (envEndpoints.apis && Array.isArray(envEndpoints.apis)) {
        apis = envEndpoints.apis.map(api => ({
          slug: (api.slug || '').toLowerCase().replace(/[^a-z0-9-]/g, ''),
          targetHost: api.targetHost || 'localhost',
          targetPort: parseInt(api.targetPort) || 3001,
          localOnly: !!api.localOnly,
          requireAuth: !!api.requireAuth
        }));
      } else if (envEndpoints.api) {
        // Backward compatibility: convert single api to apis[]
        const apiPort = parseInt(envEndpoints.api.targetPort);
        if (envEndpoints.api.targetPort && (isNaN(apiPort) || apiPort < 1 || apiPort > 65535)) {
          return { success: false, error: `Invalid API port for environment ${envId}` };
        }
        apis = [{
          slug: '',
          targetHost: envEndpoints.api.targetHost || 'localhost',
          targetPort: apiPort || 3001,
          localOnly: !!envEndpoints.api.localOnly,
          requireAuth: !!envEndpoints.api.requireAuth
        }];
      }

      // Validate API ports
      for (const api of apis) {
        if (isNaN(api.targetPort) || api.targetPort < 1 || api.targetPort > 65535) {
          return { success: false, error: `Invalid API port for environment ${envId}` };
        }
      }

      validEndpoints[envId] = {
        frontend: {
          targetHost: envEndpoints.frontend.targetHost,
          targetPort: frontendPort,
          localOnly: !!envEndpoints.frontend.localOnly,
          requireAuth: !!envEndpoints.frontend.requireAuth
        },
        apis
      };
    }

    if (Object.keys(validEndpoints).length === 0) {
      return { success: false, error: 'No valid environment endpoints provided' };
    }

    const newApp = {
      id: crypto.randomUUID(),
      name,
      slug: slug.toLowerCase(),
      endpoints: validEndpoints,
      enabled: true,
      createdAt: new Date().toISOString()
    };

    config.applications.push(newApp);
    await saveConfigFile(config);
    await applyCaddyConfig();

    return { success: true, application: newApp };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function updateApplication(appId, updates) {
  try {
    const config = await loadConfig();
    const appIndex = config.applications.findIndex(a => a.id === appId);

    if (appIndex === -1) {
      return { success: false, error: 'Application not found' };
    }

    const app = config.applications[appIndex];

    // Update name if provided
    if (updates.name) {
      app.name = updates.name;
    }

    // Update slug if provided
    if (updates.slug) {
      const newSlug = updates.slug.toLowerCase();

      // Validate slug format
      const slugRegex = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;
      if (!slugRegex.test(newSlug)) {
        return { success: false, error: 'Invalid slug format' };
      }

      // Check for duplicate slug (excluding current app)
      if (config.applications.some(a => a.id !== appId && a.slug === newSlug)) {
        return { success: false, error: 'Application with this slug already exists' };
      }

      app.slug = newSlug;
    }

    // Update enabled if provided
    if (typeof updates.enabled === 'boolean') {
      app.enabled = updates.enabled;
    }

    // Update endpoints if provided
    if (updates.endpoints && typeof updates.endpoints === 'object') {
      // Initialize endpoints if not exists
      if (!app.endpoints) {
        app.endpoints = {};
      }

      for (const [envId, envEndpoints] of Object.entries(updates.endpoints)) {
        // Check if environment exists
        if (!config.environments.some(e => e.id === envId)) {
          continue; // Skip invalid environments
        }

        if (envEndpoints === null) {
          // Remove environment
          delete app.endpoints[envId];
        } else {
          // Validate frontend port if frontend is being updated
          if (envEndpoints.frontend && envEndpoints.frontend.targetPort) {
            const frontendPort = parseInt(envEndpoints.frontend.targetPort);
            if (isNaN(frontendPort) || frontendPort < 1 || frontendPort > 65535) {
              return { success: false, error: `Invalid frontend port for environment ${envId}` };
            }
          }

          // Process APIs (new format: apis[] array)
          let apis;
          if (envEndpoints.apis !== undefined) {
            if (Array.isArray(envEndpoints.apis)) {
              apis = envEndpoints.apis.map(api => ({
                slug: (api.slug || '').toLowerCase().replace(/[^a-z0-9-]/g, ''),
                targetHost: api.targetHost || 'localhost',
                targetPort: parseInt(api.targetPort) || 3001,
                localOnly: !!api.localOnly,
                requireAuth: !!api.requireAuth
              }));
            } else {
              apis = [];
            }
          } else if (envEndpoints.api !== undefined) {
            // Backward compatibility: convert single api to apis[]
            if (envEndpoints.api) {
              const apiPort = parseInt(envEndpoints.api.targetPort);
              if (envEndpoints.api.targetPort && (isNaN(apiPort) || apiPort < 1 || apiPort > 65535)) {
                return { success: false, error: `Invalid API port for environment ${envId}` };
              }
              apis = [{
                slug: '',
                targetHost: envEndpoints.api.targetHost || 'localhost',
                targetPort: apiPort || 3001,
                localOnly: !!envEndpoints.api.localOnly,
                requireAuth: !!envEndpoints.api.requireAuth
              }];
            } else {
              apis = [];
            }
          } else {
            // Keep existing apis
            apis = app.endpoints[envId]?.apis || [];
          }

          // Validate API ports
          for (const api of apis) {
            if (isNaN(api.targetPort) || api.targetPort < 1 || api.targetPort > 65535) {
              return { success: false, error: `Invalid API port for environment ${envId}` };
            }
          }

          // Update or add environment endpoints
          app.endpoints[envId] = {
            frontend: envEndpoints.frontend ? {
              targetHost: envEndpoints.frontend.targetHost || 'localhost',
              targetPort: parseInt(envEndpoints.frontend.targetPort) || 3000,
              localOnly: !!envEndpoints.frontend.localOnly,
              requireAuth: !!envEndpoints.frontend.requireAuth
            } : app.endpoints[envId]?.frontend || null,
            apis
          };
        }
      }
    }

    await saveConfigFile(config);
    await applyCaddyConfig();

    return { success: true, application: app };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function deleteApplication(appId) {
  try {
    const config = await loadConfig();
    const appIndex = config.applications.findIndex(a => a.id === appId);

    if (appIndex === -1) {
      return { success: false, error: 'Application not found' };
    }

    const deletedApp = config.applications.splice(appIndex, 1)[0];
    await saveConfigFile(config);
    await applyCaddyConfig();

    return { success: true, message: 'Application deleted', application: deletedApp };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function toggleApplication(appId, enabled) {
  try {
    return await updateApplication(appId, { enabled: !!enabled });
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ========== Migration ==========

// ========== Cloudflare Configuration ==========

export async function getCloudflareConfig() {
  try {
    const config = await loadConfig();
    return {
      success: true,
      cloudflare: {
        enabled: config.cloudflare?.enabled || false,
        wildcardDomains: config.cloudflare?.wildcardDomains || [],
        hasToken: !!process.env.CF_API_TOKEN
      }
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function updateCloudflareConfig(cfConfig) {
  try {
    const config = await loadConfig();

    config.cloudflare = {
      ...config.cloudflare,
      enabled: !!cfConfig.enabled,
      wildcardDomains: cfConfig.wildcardDomains || config.cloudflare?.wildcardDomains || []
    };

    // Auto-generate wildcard domains based on environments
    if (cfConfig.enabled && config.baseDomain) {
      const wildcards = new Set();
      for (const env of config.environments) {
        if (env.prefix) {
          wildcards.add(`*.${env.prefix}.${config.baseDomain}`);
          wildcards.add(`*.${env.apiPrefix}.${config.baseDomain}`);
        } else {
          wildcards.add(`*.${config.baseDomain}`);
          wildcards.add(`*.${env.apiPrefix}.${config.baseDomain}`);
        }
      }
      config.cloudflare.wildcardDomains = Array.from(wildcards);
    }

    await saveConfigFile(config);
    await applyCaddyConfig();

    return { success: true, cloudflare: config.cloudflare };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ========== Caddy Configuration Generation ==========

// Generate domain for an application endpoint
function getAppDomain(app, endpointType, env, baseDomain, apiSlug = '') {
  // endpointType: 'frontend' or 'api'
  // env: environment object with prefix and apiPrefix
  // apiSlug: optional slug for additional APIs (e.g., 'cdn', 'ws')

  if (endpointType === 'api') {
    // API domain format: {app}-{slug}.{apiPrefix}.{baseDomain} or {app}.{apiPrefix}.{baseDomain}
    // e.g., www.api.dev.example.com (default) or www-cdn.api.dev.example.com
    const hostPart = apiSlug ? `${app.slug}-${apiSlug}` : app.slug;
    return `${hostPart}.${env.apiPrefix}.${baseDomain}`;
  } else {
    // Frontend: {slug}.{prefix}.{baseDomain} or {slug}.{baseDomain}
    if (env.prefix) {
      return `${app.slug}.${env.prefix}.${baseDomain}`;
    }
    return `${app.slug}.${baseDomain}`;
  }
}

// Generate CSP header handler
function getCspHeaderHandler(baseDomain) {
  return {
    handler: 'headers',
    response: {
      set: {
        'Content-Security-Policy': [
          `default-src 'self'; connect-src 'self' https://auth.${baseDomain}; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: https:; font-src 'self' data:`
        ]
      }
    }
  };
}

// Generate routes for an application across all its environments
function generateAppRoutes(app, environments, baseDomain) {
  const { DASHBOARD_PORT } = getEnv();
  const authServiceUrl = `http://localhost:${DASHBOARD_PORT}`;
  const routes = [];

  // New structure: app.endpoints is an object { envId: { frontend, apis: [] } }
  if (!app.endpoints || typeof app.endpoints !== 'object') {
    return routes;
  }

  for (const [envId, envEndpoints] of Object.entries(app.endpoints)) {
    const env = environments.find(e => e.id === envId);
    if (!env || !envEndpoints) continue;

    // Frontend route
    if (envEndpoints.frontend) {
      const frontendDomain = getAppDomain(app, 'frontend', env, baseDomain);
      routes.push(generateEndpointRoute(
        `${app.id}-frontend-${envId}`,
        frontendDomain,
        envEndpoints.frontend,
        baseDomain,
        authServiceUrl,
        envId
      ));
    }

    // API routes (multiple APIs supported via apis[])
    const apis = envEndpoints.apis || [];
    for (const api of apis) {
      const apiSlug = api.slug || '';
      const apiDomain = getAppDomain(app, 'api', env, baseDomain, apiSlug);
      const routeId = apiSlug
        ? `${app.id}-api-${apiSlug}-${envId}`
        : `${app.id}-api-${envId}`;
      routes.push(generateEndpointRoute(
        routeId,
        apiDomain,
        api,
        baseDomain,
        authServiceUrl,
        envId
      ));
    }

    // Backward compatibility: support old 'api' field if apis[] not present
    if (envEndpoints.api && (!envEndpoints.apis || envEndpoints.apis.length === 0)) {
      const apiDomain = getAppDomain(app, 'api', env, baseDomain);
      routes.push(generateEndpointRoute(
        `${app.id}-api-${envId}`,
        apiDomain,
        envEndpoints.api,
        baseDomain,
        authServiceUrl,
        envId
      ));
    }
  }

  return routes;
}

// Generate a single endpoint route
function generateEndpointRoute(id, domain, endpoint, baseDomain, authServiceUrl, envId = null) {
  const cspHandler = getCspHeaderHandler(baseDomain);
  const reverseProxyHandler = {
    handler: 'reverse_proxy',
    upstreams: [{
      dial: `${endpoint.targetHost}:${endpoint.targetPort}`
    }]
  };

  // Detect if this is a development environment
  const isDevelopment = envId && envId.toLowerCase().includes('dev');

  if (endpoint.requireAuth) {
    // Build routes array for subroute
    const routes = [];

    // Add WebSocket bypass ONLY for development environments
    if (isDevelopment) {
      routes.push({
        // WebSocket bypass in dev: pass directly without auth check (for Vite HMR)
        match: [{ header: { 'Upgrade': ['websocket'] } }],
        handle: [reverseProxyHandler]
      });
    }

    // Add normal auth check route
    routes.push({
      // Normal requests: auth check
      handle: [{
        handler: 'reverse_proxy',
        upstreams: [{ dial: authServiceUrl.replace('http://', '') }],
        rewrite: { uri: '/api/authz/forward-auth' },
        handle_response: [
          {
            match: { status_code: [401, 403] },
            routes: [{
              handle: [{
                handler: 'static_response',
                status_code: 302,
                headers: {
                  'Location': [`https://auth.${baseDomain}/login?rd=https://${domain}{http.request.uri}`]
                }
              }]
            }]
          },
          {
            match: { status_code: [200] },
            routes: [{ handle: [reverseProxyHandler] }]
          }
        ]
      }]
    });

    const authCheckRoute = {
      '@id': id,
      match: [{ host: [domain] }],
      handle: [
        cspHandler,
        {
          handler: 'subroute',
          routes
        }
      ],
      terminal: true
    };

    if (endpoint.localOnly) {
      authCheckRoute.handle = [{
        handler: 'subroute',
        routes: [
          { match: [{ remote_ip: { ranges: LOCAL_NETWORKS } }], handle: [cspHandler, ...authCheckRoute.handle.slice(1)] },
          { handle: [{ handler: 'error', status_code: 403 }] }
        ]
      }];
    }

    return authCheckRoute;
  }

  // Without requireAuth: direct proxy
  if (endpoint.localOnly) {
    return {
      '@id': id,
      match: [{ host: [domain] }],
      handle: [{
        handler: 'subroute',
        routes: [
          { match: [{ remote_ip: { ranges: LOCAL_NETWORKS } }], handle: [cspHandler, reverseProxyHandler] },
          { handle: [{ handler: 'error', status_code: 403 }] }
        ]
      }],
      terminal: true
    };
  }

  return {
    '@id': id,
    match: [{ host: [domain] }],
    handle: [cspHandler, reverseProxyHandler],
    terminal: true
  };
}

function generateCaddyRoute(host, baseDomain) {
  const domain = host.customDomain || `${host.subdomain}.${baseDomain}`;
  const { DASHBOARD_PORT } = getEnv();
  const authServiceUrl = `http://localhost:${DASHBOARD_PORT}`;
  const cspHandler = getCspHeaderHandler(baseDomain);

  // Proxy direct vers la cible
  const reverseProxyHandler = {
    handler: 'reverse_proxy',
    upstreams: [{
      dial: `${host.targetHost}:${host.targetPort}`
    }]
  };

  // Detect if this is a development environment (domain contains .dev.)
  const isDevelopment = domain.includes('.dev.');

  // Si requireAuth est activé, on utilise intercept pour vérifier l'auth
  // via une sous-requête à l'auth-service
  if (host.requireAuth) {
    // Build routes array for subroute
    const routes = [];

    // Add WebSocket bypass ONLY for development environments
    if (isDevelopment) {
      routes.push({
        // WebSocket bypass in dev: pass directly without auth check (for Vite HMR)
        match: [{ header: { 'Upgrade': ['websocket'] } }],
        handle: [reverseProxyHandler]
      });
    }

    // Add normal auth check route
    routes.push({
      // Normal requests: auth check
      // Faire une sous-requête à l'auth-service pour vérifier le cookie
      handle: [{
        handler: 'reverse_proxy',
        upstreams: [{ dial: authServiceUrl.replace('http://', '') }],
        rewrite: {
          uri: '/api/authz/forward-auth'
        },
        handle_response: [
          {
            // Si auth-service retourne 401, rediriger vers login
            match: { status_code: [401, 403] },
            routes: [{
              handle: [{
                handler: 'static_response',
                status_code: 302,
                headers: {
                  'Location': [`https://auth.${baseDomain}/login?rd=https://${domain}{http.request.uri}`]
                }
              }]
            }]
          },
          {
            // Si auth OK (200), proxy vers l'app cible
            match: { status_code: [200] },
            routes: [{
              handle: [reverseProxyHandler]
            }]
          }
        ]
      }]
    });

    // Route avec vérification d'auth via intercept
    const authCheckRoute = {
      '@id': host.id,
      match: [{ host: [domain] }],
      handle: [
        cspHandler,
        {
          handler: 'subroute',
          routes
        }
      ],
      terminal: true
    };

    // Ajouter restriction IP si localOnly
    if (host.localOnly) {
      authCheckRoute.handle = [{
        handler: 'subroute',
        routes: [
          {
            match: [{ remote_ip: { ranges: LOCAL_NETWORKS } }],
            handle: [cspHandler, ...authCheckRoute.handle.slice(1)]
          },
          {
            handle: [{
              handler: 'error',
              status_code: 403
            }]
          }
        ]
      }];
    }

    return authCheckRoute;
  }

  // Sans requireAuth : proxy direct
  if (host.localOnly) {
    return {
      '@id': host.id,
      match: [{ host: [domain] }],
      handle: [{
        handler: 'subroute',
        routes: [
          {
            match: [{ remote_ip: { ranges: LOCAL_NETWORKS } }],
            handle: [cspHandler, reverseProxyHandler]
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
    handle: [cspHandler, reverseProxyHandler],
    terminal: true
  };
}

function generateCaddyConfig(config) {
  const { DASHBOARD_PORT } = getEnv();
  const authServiceUrl = `http://localhost:${DASHBOARD_PORT}`;

  const routes = [];

  // Generate routes for applications
  const enabledApps = (config.applications || []).filter(a => a.enabled);
  for (const app of enabledApps) {
    const appRoutes = generateAppRoutes(app, config.environments || [], config.baseDomain);
    routes.push(...appRoutes);
  }

  // Generate routes for standalone hosts
  const enabledHosts = config.hosts.filter(h => h.enabled);
  routes.push(...enabledHosts.map(h => generateCaddyRoute(h, config.baseDomain)));

  // Add system route for dashboard (proxy.<baseDomain>)
  // Note: L'authentification est gérée côté API via le middleware (cookie auth_session)
  if (config.baseDomain) {
    const cspHandler = getCspHeaderHandler(config.baseDomain);

    routes.unshift({
      '@id': 'system-dashboard',
      match: [{ host: [`proxy.${config.baseDomain}`] }],
      handle: [
        cspHandler,
        {
          handler: 'reverse_proxy',
          upstreams: [{ dial: `localhost:${DASHBOARD_PORT}` }]
        }
      ],
      terminal: true
    });

    // Add auth portal route (auth.<baseDomain>) - custom auth service (no auth required)
    routes.unshift({
      '@id': 'system-auth',
      match: [{ host: [`auth.${config.baseDomain}`] }],
      handle: [
        cspHandler,
        {
          handler: 'reverse_proxy',
          upstreams: [{ dial: authServiceUrl.replace('http://', '').replace('https://', '') }]
        }
      ],
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
            routes,
            logs: {
              default_logger_name: 'homeroute_access'
            }
          }
        }
      }
    }
  };

  // TLS configuration
  if (routes.length > 0) {
    if (config.cloudflare?.enabled && process.env.CF_API_TOKEN) {
      // Cloudflare DNS challenge for wildcard certificates
      caddyConfig.apps.tls = {
        automation: {
          policies: [{
            subjects: config.cloudflare.wildcardDomains || [],
            issuers: [{
              module: 'acme',
              challenges: {
                dns: {
                  provider: {
                    name: 'cloudflare',
                    api_token: process.env.CF_API_TOKEN
                  }
                }
              }
            }]
          }]
        }
      };
    } else {
      // Default: Let's Encrypt with HTTP challenge (individual certs)
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
  }

  // Add access logging for traffic analytics
  caddyConfig.logging = {
    logs: {
      homeroute_access: {
        writer: {
          output: 'file',
          filename: process.env.CADDY_ACCESS_LOG || '/var/log/caddy/homeroute-access.json'
        },
        encoder: {
          format: 'json'
        }
      }
    }
  };

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

    // Check all applications
    for (const app of config.applications || []) {
      if (!app.enabled || !app.endpoints) continue;

      for (const [envId, envEndpoints] of Object.entries(app.endpoints)) {
        const env = config.environments.find(e => e.id === envId);
        if (!env || !envEndpoints) continue;

        // Check frontend certificate
        if (envEndpoints.frontend) {
          const frontendDomain = getAppDomain(app, 'frontend', env, config.baseDomain);
          const key = `${app.id}-frontend-${envId}`;
          statuses[key] = await checkCertificate(frontendDomain);
        }

        // Check API certificates (multiple APIs supported)
        const apis = envEndpoints.apis || [];
        for (const api of apis) {
          const apiSlug = api.slug || '';
          const apiDomain = getAppDomain(app, 'api', env, config.baseDomain, apiSlug);
          const key = apiSlug
            ? `${app.id}-api-${apiSlug}-${envId}`
            : `${app.id}-api-${envId}`;
          statuses[key] = await checkCertificate(apiDomain);
        }

        // Backward compatibility: check old 'api' field if apis[] not present
        if (envEndpoints.api && (!envEndpoints.apis || envEndpoints.apis.length === 0)) {
          const apiDomain = getAppDomain(app, 'api', env, config.baseDomain);
          const key = `${app.id}-api-${envId}`;
          statuses[key] = await checkCertificate(apiDomain);
        }
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
