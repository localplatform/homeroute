import { readFile, stat, open } from 'fs/promises';
import { existsSync } from 'fs';
import { createReadStream } from 'fs';
import { createInterface } from 'readline';
import { getCollection, COLLECTIONS } from './mongodb.js';
import { getDhcpLeases } from './dnsmasq.js';
import { getConfig as getReverseProxyConfig } from './reverseproxy.js';
import { trafficEvents } from './traffic.js';

const ACCESS_LOG_PATH = process.env.PROXY_ACCESS_LOG || '/var/log/rust-proxy/access.json';
const BATCH_SIZE = parseInt(process.env.TRAFFIC_BATCH_SIZE || '100');
const BATCH_INTERVAL_MS = parseInt(process.env.TRAFFIC_BATCH_INTERVAL_MS || '5000');
const POLL_INTERVAL_MS = 2000; // Check for new lines every 2 seconds

let lastPosition = 0;
let eventBatch = [];
let batchTimer = null;
let pollingInterval = null;
let isProcessing = false;

// Cache for DHCP leases and reverse proxy config
let dhcpLeasesCache = [];
let reverseProxyConfigCache = null;
let lastCacheUpdate = 0;
const CACHE_TTL_MS = 60000; // 1 minute

/**
 * Start HTTP traffic capture from reverse proxy access logs
 */
export async function startHttpCapture() {
  console.log('Starting HTTP traffic capture...');

  // Check if log file exists
  if (!existsSync(ACCESS_LOG_PATH)) {
    console.warn(`Access log not found at ${ACCESS_LOG_PATH}. HTTP capture will start when log file is created.`);
    // Start polling anyway - file might be created later
  } else {
    // Get initial file size to skip existing logs
    try {
      const stats = await stat(ACCESS_LOG_PATH);
      lastPosition = stats.size;
      console.log(`Starting HTTP capture from position ${lastPosition} in ${ACCESS_LOG_PATH}`);
    } catch (error) {
      console.error('Error getting log file stats:', error);
    }
  }

  // Start polling for new log lines
  pollingInterval = setInterval(pollLogFile, POLL_INTERVAL_MS);

  console.log('✓ HTTP traffic capture started');
}

/**
 * Stop HTTP traffic capture
 */
export function stopHttpCapture() {
  if (pollingInterval) {
    clearInterval(pollingInterval);
    pollingInterval = null;
  }

  if (batchTimer) {
    clearTimeout(batchTimer);
    batchTimer = null;
  }

  // Flush remaining events
  if (eventBatch.length > 0) {
    flushBatch();
  }

  console.log('HTTP traffic capture stopped');
}

/**
 * Poll log file for new lines
 */
async function pollLogFile() {
  if (isProcessing) return; // Skip if already processing
  if (!existsSync(ACCESS_LOG_PATH)) return; // File doesn't exist yet

  isProcessing = true;

  try {
    const stats = await stat(CADDY_LOG_PATH);
    const currentSize = stats.size;

    // File was truncated or rotated
    if (currentSize < lastPosition) {
      console.log('Log file rotated, resetting position');
      lastPosition = 0;
    }

    // No new data
    if (currentSize === lastPosition) {
      isProcessing = false;
      return;
    }

    // Read new lines
    const newLines = await readNewLines(lastPosition, currentSize);
    lastPosition = currentSize;

    // Process each line
    for (const line of newLines) {
      if (line.trim()) {
        await processLogLine(line);
      }
    }
  } catch (error) {
    console.error('Error polling log file:', error);
  } finally {
    isProcessing = false;
  }
}

/**
 * Read new lines from log file
 */
async function readNewLines(start, end) {
  return new Promise((resolve, reject) => {
    const lines = [];
    const fileStream = createReadStream(ACCESS_LOG_PATH, {
      start,
      end: end - 1,
      encoding: 'utf8'
    });

    const rl = createInterface({
      input: fileStream,
      crlfDelay: Infinity
    });

    rl.on('line', (line) => {
      lines.push(line);
    });

    rl.on('close', () => {
      resolve(lines);
    });

    rl.on('error', (error) => {
      reject(error);
    });
  });
}

/**
 * Process a single log line
 */
async function processLogLine(line) {
  try {
    const logEntry = JSON.parse(line);

    // Enrich with metadata
    const enrichedEvent = await enrichHttpEvent(logEntry);

    // Add to batch
    eventBatch.push(enrichedEvent);

    // Flush if batch is full
    if (eventBatch.length >= BATCH_SIZE) {
      await flushBatch();
    } else if (!batchTimer) {
      // Set timer to flush after interval
      batchTimer = setTimeout(flushBatch, BATCH_INTERVAL_MS);
    }
  } catch (error) {
    console.error('Error processing log line:', error.message);
    // Skip invalid JSON lines
  }
}

/**
 * Enrich HTTP event with metadata
 * Rust proxy log format: {"timestamp":"...","client_ip":"...","host":"...","method":"GET","path":"/...","status":200,"duration_ms":12,"user_agent":"..."}
 */
async function enrichHttpEvent(logEntry) {
  // Update cache if needed
  await updateCaches();

  const clientIp = logEntry.client_ip || '';

  // Find device from DHCP leases
  const lease = dhcpLeasesCache.find(l => l.ip === clientIp);

  // Find application from reverse proxy config
  const app = findApplicationByHost(logEntry.host);

  return {
    timestamp: new Date(logEntry.timestamp || Date.now()),
    meta: {
      deviceMac: lease?.mac || null,
      deviceIp: clientIp,
      deviceHostname: lease?.hostname || null,
      userId: null,
      endpoint: logEntry.host || '',
      application: app?.name || null,
      environment: app?.environment || null,
      path: logEntry.path || '/',
      method: logEntry.method || 'GET',
      statusCode: logEntry.status || 0
    },
    metrics: {
      requestCount: 1,
      responseBytes: 0,
      requestBytes: 0,
      responseTimeMs: logEntry.duration_ms || 0
    }
  };
}

/**
 * Update caches (DHCP leases and reverse proxy config)
 */
async function updateCaches() {
  const now = Date.now();

  if (now - lastCacheUpdate > CACHE_TTL_MS) {
    try {
      // Update DHCP leases
      const leasesResult = await getDhcpLeases();
      if (leasesResult.success) {
        dhcpLeasesCache = leasesResult.leases || [];
      }

      // Update reverse proxy config
      const configResult = await getReverseProxyConfig();
      if (configResult.success) {
        reverseProxyConfigCache = configResult.config;
      }

      lastCacheUpdate = now;
    } catch (error) {
      console.error('Error updating caches:', error);
    }
  }
}

/**
 * Find application by host from reverse proxy config
 */
function findApplicationByHost(host) {
  if (!reverseProxyConfigCache) return null;

  const { applications = [], environments = [] } = reverseProxyConfigCache;

  // Check each application
  for (const app of applications) {
    if (!app.endpoints) continue;

    for (const [envId, endpoints] of Object.entries(app.endpoints)) {
      const env = environments.find(e => e.id === envId);
      if (!env) continue;

      // Check frontend domain
      if (endpoints.frontend) {
        const frontendDomain = getAppDomain(app, 'frontend', env);
        if (frontendDomain === host) {
          return { name: app.name, environment: env.name || env.id };
        }
      }

      // Check API domains
      const apis = endpoints.apis || [];
      for (const api of apis) {
        const apiDomain = getAppDomain(app, 'api', env, api.slug);
        if (apiDomain === host) {
          return { name: app.name, environment: env.name || env.id };
        }
      }
    }
  }

  return null;
}

/**
 * Get application domain (mirrored from reverseproxy.js)
 */
function getAppDomain(app, type, env, apiSlug = '') {
  if (!reverseProxyConfigCache) return null;

  const { baseDomain } = reverseProxyConfigCache;
  const prefix = env.prefix || '';

  if (type === 'frontend') {
    return prefix ? `${app.id}-${prefix}.${baseDomain}` : `${app.id}.${baseDomain}`;
  } else if (type === 'api') {
    const slug = apiSlug ? `-${apiSlug}` : '';
    return prefix ? `${app.id}${slug}-api-${prefix}.${baseDomain}` : `${app.id}${slug}-api.${baseDomain}`;
  }

  return null;
}

/**
 * Flush event batch to MongoDB
 */
async function flushBatch() {
  if (batchTimer) {
    clearTimeout(batchTimer);
    batchTimer = null;
  }

  if (eventBatch.length === 0) return;

  const events = [...eventBatch];
  eventBatch = [];

  try {
    const collection = getCollection(COLLECTIONS.TRAFFIC_HTTP);
    await collection.insertMany(events);

    // Emit real-time update
    emitRealtimeUpdate(events);

    console.log(`✓ Inserted ${events.length} HTTP traffic events`);
  } catch (error) {
    console.error('Error flushing HTTP events to MongoDB:', error);
    // Don't re-add to batch to avoid infinite loop
  }
}

/**
 * Emit real-time traffic update
 */
function emitRealtimeUpdate(events) {
  if (!events.length) return;

  try {
    // Calculate RPS (events per second)
    const rps = events.length / (BATCH_INTERVAL_MS / 1000);

    // Calculate bandwidth (bytes per second)
    const totalBytes = events.reduce((sum, e) => sum + (e.metrics.responseBytes || 0), 0);
    const bandwidthMbps = (totalBytes / (BATCH_INTERVAL_MS / 1000)) / (1024 * 1024);

    // Find top endpoint
    const endpointCounts = {};
    events.forEach(e => {
      const endpoint = e.meta.endpoint || 'unknown';
      endpointCounts[endpoint] = (endpointCounts[endpoint] || 0) + 1;
    });

    const topEndpoint = Object.entries(endpointCounts)
      .sort((a, b) => b[1] - a[1])[0]?.[0] || null;

    trafficEvents.emit('trafficUpdate', {
      rps: Math.round(rps),
      bandwidthMbps: parseFloat(bandwidthMbps.toFixed(3)),
      topEndpoint,
      timestamp: new Date().toISOString()
    });
  } catch (error) {
    console.error('Error emitting real-time update:', error);
  }
}

// Graceful shutdown
process.on('SIGINT', stopHttpCapture);
process.on('SIGTERM', stopHttpCapture);
