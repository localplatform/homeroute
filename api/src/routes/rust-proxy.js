import { Router } from 'express';
import { readFile, writeFile } from 'fs/promises';
import { existsSync } from 'fs';
import { execSync, spawn } from 'child_process';

const router = Router();

const RUST_PROXY_CONFIG = process.env.RUST_PROXY_CONFIG || '/var/lib/server-dashboard/rust-proxy-config.json';

// ========== Helpers ==========

async function loadRustProxyConfig() {
  if (!existsSync(RUST_PROXY_CONFIG)) {
    return null;
  }
  const content = await readFile(RUST_PROXY_CONFIG, 'utf-8');
  return JSON.parse(content);
}

async function saveRustProxyConfig(config) {
  await writeFile(RUST_PROXY_CONFIG, JSON.stringify(config, null, 2));
}

function getRustProxyPid() {
  try {
    const output = execSync('systemctl show rust-proxy --property=MainPID --value', { encoding: 'utf-8' }).trim();
    const pid = parseInt(output);
    return pid > 0 ? pid : null;
  } catch {
    return null;
  }
}

function reloadRustProxy() {
  try {
    execSync('systemctl reload rust-proxy', { encoding: 'utf-8' });
    return true;
  } catch (e) {
    console.error('Failed to reload rust-proxy:', e.message);
    return false;
  }
}

function isRustProxyRunning() {
  try {
    const output = execSync('systemctl is-active rust-proxy', { encoding: 'utf-8' }).trim();
    return output === 'active';
  } catch {
    return false;
  }
}

// ========== Routes ==========

// GET /api/rust-proxy/status
router.get('/status', async (req, res) => {
  try {
    const running = isRustProxyRunning();
    const config = await loadRustProxyConfig();
    const routes = config?.routes?.filter(r => r.enabled) || [];

    res.json({
      success: true,
      running,
      activeRoutes: routes.length,
      httpsPort: config?.https_port || 443,
      pid: getRustProxyPid()
    });
  } catch (error) {
    res.json({ success: false, error: error.message });
  }
});

// POST /api/rust-proxy/reload - Send SIGHUP to reload config + certs
router.post('/reload', async (req, res) => {
  try {
    const success = reloadRustProxy();
    res.json({
      success,
      message: success ? 'Rust proxy reloaded' : 'Failed to reload'
    });
  } catch (error) {
    res.json({ success: false, error: error.message });
  }
});

// GET /api/rust-proxy/routes - List all routes in rust-proxy config
router.get('/routes', async (req, res) => {
  try {
    const config = await loadRustProxyConfig();
    res.json({
      success: true,
      routes: config?.routes || []
    });
  } catch (error) {
    res.json({ success: false, error: error.message });
  }
});

export default router;

// ========== Exported helper functions ==========

export { reloadRustProxy };

export async function getRustProxyStatus() {
  const running = isRustProxyRunning();
  const config = await loadRustProxyConfig();
  const routes = config?.routes?.filter(r => r.enabled) || [];

  return {
    success: true,
    running,
    activeRoutes: routes.length,
    httpsPort: config?.https_port || 443,
    pid: getRustProxyPid()
  };
}

// ========== Sync function (used by reverseproxy service) ==========

/**
 * Synchronize routes from the main reverseproxy config to rust-proxy config.
 * Called whenever hosts/apps with backend='rust' are modified.
 * Issues CA certificates for new domains automatically.
 */
export async function syncRustProxyRoutes(rustRoutes) {
  try {
    let config = await loadRustProxyConfig();
    if (!config) {
      config = {
        http_port: 80,
        https_port: 443,
        base_domain: 'mynetwk.biz',
        tls_mode: 'local-ca',
        ca_storage_path: '/var/lib/server-dashboard/ca',
        auth_service_url: 'http://localhost:4000',
        routes: [],
        local_networks: [
          '192.168.0.0/16',
          '10.0.0.0/8',
          '172.16.0.0/12',
          '127.0.0.0/8'
        ]
      };
    }

    // Ensure correct ports and access log
    config.https_port = 443;
    config.http_port = 80;
    config.access_log_path = '/var/log/rust-proxy/access.json';

    // Build a map of existing routes by domain (to preserve cert_id)
    const existingByDomain = {};
    for (const route of config.routes || []) {
      existingByDomain[route.domain] = route;
    }

    // Build new routes list
    const newRoutes = [];
    for (const route of rustRoutes) {
      const existing = existingByDomain[route.domain];
      let certId = existing?.cert_id || null;

      // Issue certificate if none exists
      if (!certId) {
        try {
          certId = await issueCertForDomain(route.domain);
        } catch (e) {
          console.error(`Failed to issue cert for ${route.domain}:`, e.message);
        }
      }

      newRoutes.push({
        id: existing?.id || route.id,
        domain: route.domain,
        target_host: route.target_host,
        target_port: route.target_port,
        local_only: route.local_only || false,
        require_auth: route.require_auth || false,
        enabled: route.enabled !== false,
        cert_id: certId
      });
    }

    config.routes = newRoutes;
    await saveRustProxyConfig(config);

    // Reload rust-proxy if running
    if (isRustProxyRunning()) {
      reloadRustProxy();
    }

    return { success: true, routeCount: newRoutes.length };
  } catch (error) {
    console.error('Failed to sync rust-proxy routes:', error);
    return { success: false, error: error.message };
  }
}

/**
 * Issue a CA certificate for a domain using ca-cli
 */
async function issueCertForDomain(domain) {
  return new Promise((resolve, reject) => {
    const caCli = '/opt/homeroute/ca-service/target/release/ca-cli';
    if (!existsSync(caCli)) {
      reject(new Error('ca-cli binary not found'));
      return;
    }

    const child = spawn(caCli, ['issue', '--domains', domain], {
      env: { ...process.env, CA_STORAGE_PATH: '/var/lib/server-dashboard/ca' }
    });

    let stdout = '';
    let stderr = '';
    child.stdout.on('data', data => stdout += data);
    child.stderr.on('data', data => stderr += data);

    child.on('close', code => {
      if (code !== 0) {
        reject(new Error(`ca-cli failed: ${stderr}`));
        return;
      }
      try {
        const result = JSON.parse(stdout);
        if (result.success && result.certificate?.id) {
          resolve(result.certificate.id);
        } else {
          reject(new Error(result.error || 'Unknown error'));
        }
      } catch (e) {
        reject(new Error(`Failed to parse ca-cli output: ${stdout}`));
      }
    });
  });
}
