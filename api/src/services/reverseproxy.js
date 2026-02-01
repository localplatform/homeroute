import { readFile, writeFile, mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import path from 'path';
import crypto from 'crypto';
import tls from 'tls';
import { syncRustProxyRoutes, reloadRustProxy, getRustProxyStatus as getRustProxyStatusFromRoute } from '../routes/rust-proxy.js';

// Environment configuration
const getEnv = () => ({
  CONFIG_FILE: process.env.REVERSEPROXY_CONFIG || '/var/lib/server-dashboard/reverseproxy-config.json',
  DASHBOARD_PORT: process.env.PORT || '4000'
});

// Default config structure
const getDefaultConfig = () => ({
  baseDomain: '',
  environments: [
    { id: 'prod', name: 'Production', prefix: '', apiPrefix: 'api', isDefault: true },
    { id: 'dev', name: 'Development', prefix: 'dev', apiPrefix: 'api.dev', isDefault: false }
  ],
  applications: [],
  hosts: []
});

async function ensureConfigDir() {
  const { CONFIG_FILE } = getEnv();
  const configDir = path.dirname(CONFIG_FILE);
  if (!existsSync(configDir)) {
    await mkdir(configDir, { recursive: true });
  }
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
    // Migration: remove old fields
    delete saved.wildcardCert;
    delete saved.cloudflare;

    // Merge with defaults to ensure all fields exist
    const defaultConfig = getDefaultConfig();
    const config = {
      ...defaultConfig,
      ...saved,
      // Ensure nested objects have defaults
      environments: saved.environments || defaultConfig.environments,
      applications: saved.applications || defaultConfig.applications
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

    // Sync all routes to Rust proxy
    await syncAllRoutes(config);

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
    await syncAllRoutes(config);

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
    await syncAllRoutes(config);

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
    await syncAllRoutes(config);

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
    await syncAllRoutes(config);

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
    const appsUsingEnv = config.applications.filter(a => a.environments?.includes(envId));
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
    await syncAllRoutes(config);

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
    await syncAllRoutes(config);

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
    await syncAllRoutes(config);

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

// ========== Domain Generation ==========

// Generate domain for an application endpoint
function getAppDomain(app, endpointType, env, baseDomain, apiSlug = '') {
  if (endpointType === 'api') {
    const hostPart = apiSlug ? `${app.slug}-${apiSlug}` : app.slug;
    return `${hostPart}.${env.apiPrefix}.${baseDomain}`;
  } else {
    if (env.prefix) {
      return `${app.slug}.${env.prefix}.${baseDomain}`;
    }
    return `${app.slug}.${baseDomain}`;
  }
}

// ========== Proxy Status & Control ==========

export async function getProxyStatus() {
  try {
    return await getRustProxyStatusFromRoute();
  } catch (error) {
    return {
      success: true,
      running: false,
      error: error.message
    };
  }
}

export async function reloadProxy() {
  try {
    await syncAllRoutes();
    return { success: true, message: 'Proxy configuration reloaded' };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function renewCertificates() {
  try {
    // With local CA, renewal means re-syncing routes which triggers ca-cli
    await syncAllRoutes();
    return { success: true, message: 'Certificate renewal triggered via local CA' };
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

// ========== Route Sync (Rust Proxy) ==========

/**
 * Collect ALL routes from hosts and applications,
 * add system routes, and sync them to the rust-proxy config.
 */
async function syncAllRoutes(config) {
  if (!config) config = await loadConfig();
  const { DASHBOARD_PORT } = getEnv();
  const routes = [];

  // System routes (proxy.<baseDomain> and auth.<baseDomain>)
  if (config.baseDomain) {
    routes.push({
      id: 'system-dashboard',
      domain: `proxy.${config.baseDomain}`,
      target_host: 'localhost',
      target_port: parseInt(DASHBOARD_PORT),
      local_only: false,
      require_auth: false,
      enabled: true
    });

    routes.push({
      id: 'system-auth',
      domain: `auth.${config.baseDomain}`,
      target_host: 'localhost',
      target_port: parseInt(DASHBOARD_PORT),
      local_only: false,
      require_auth: false,
      enabled: true
    });
  }

  // Collect from standalone hosts
  for (const host of config.hosts || []) {
    if (host.enabled) {
      const domain = host.customDomain || `${host.subdomain}.${config.baseDomain}`;
      routes.push({
        id: host.id,
        domain,
        target_host: host.targetHost,
        target_port: host.targetPort,
        local_only: !!host.localOnly,
        require_auth: !!host.requireAuth,
        enabled: true
      });
    }
  }

  // Collect from applications
  for (const app of config.applications || []) {
    if (!app.enabled || !app.endpoints) continue;

    for (const [envId, envEndpoints] of Object.entries(app.endpoints)) {
      const env = (config.environments || []).find(e => e.id === envId);
      if (!env || !envEndpoints) continue;

      // Frontend
      if (envEndpoints.frontend) {
        const domain = getAppDomain(app, 'frontend', env, config.baseDomain);
        routes.push({
          id: `${app.id}-frontend-${envId}`,
          domain,
          target_host: envEndpoints.frontend.targetHost,
          target_port: envEndpoints.frontend.targetPort,
          local_only: !!envEndpoints.frontend.localOnly,
          require_auth: !!envEndpoints.frontend.requireAuth,
          enabled: true
        });
      }

      // APIs
      for (const api of envEndpoints.apis || []) {
        const apiDomain = getAppDomain(app, 'api', env, config.baseDomain, api.slug || '');
        routes.push({
          id: api.slug ? `${app.id}-api-${api.slug}-${envId}` : `${app.id}-api-${envId}`,
          domain: apiDomain,
          target_host: api.targetHost,
          target_port: api.targetPort,
          local_only: !!api.localOnly,
          require_auth: !!api.requireAuth,
          enabled: true
        });
      }
    }
  }

  await syncRustProxyRoutes(routes);
}

// Export for use in routes and startup
export { syncAllRoutes };

// ========== Certificate Status ==========

async function checkCertificate(domain) {
  return new Promise((resolve) => {
    const socket = tls.connect({
      host: domain,
      port: 443,
      servername: domain,
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
