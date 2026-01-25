import { createReadStream, existsSync } from 'fs';
import { stat } from 'fs/promises';
import { createInterface } from 'readline';
import { getCollection, COLLECTIONS } from './mongodb.js';
import { getDhcpLeases } from './dnsmasq.js';
import { trafficEvents } from './traffic.js';

const DNS_LOG_PATH = process.env.DNS_LOG_PATH || '/var/log/dnsmasq.log';
const POLL_INTERVAL_MS = 2000; // Check for new lines every 2 seconds
const BATCH_SIZE = parseInt(process.env.TRAFFIC_BATCH_SIZE || '100');
const BATCH_INTERVAL_MS = parseInt(process.env.TRAFFIC_BATCH_INTERVAL_MS || '5000');

let lastPosition = 0;
let eventBatch = [];
let batchTimer = null;
let pollingInterval = null;
let isProcessing = false;

// Cache for DHCP leases
let dhcpLeasesCache = [];
let lastCacheUpdate = 0;
const CACHE_TTL_MS = 60000; // 1 minute

// Domain categorization
const DOMAIN_CATEGORIES = {
  'youtube.com': 'Video Streaming',
  'googlevideo.com': 'Video Streaming',
  'netflix.com': 'Video Streaming',
  'twitch.tv': 'Video Streaming',
  'google.com': 'Search & Web',
  'googleapis.com': 'Cloud Services',
  'cloudflare.com': 'CDN',
  'facebook.com': 'Social Media',
  'instagram.com': 'Social Media',
  'twitter.com': 'Social Media',
  'microsoft.com': 'Cloud Services',
  'windows.com': 'Operating System',
  'apple.com': 'Operating System',
  'icloud.com': 'Cloud Services',
  'amazon.com': 'E-commerce',
  'amazonaws.com': 'Cloud Services',
  'spotify.com': 'Music Streaming',
  'github.com': 'Development',
  'gitlab.com': 'Development',
  'mynetwk.biz': 'Local Services'
};

/**
 * Start DNS traffic capture
 */
export async function startDnsCapture() {
  console.log('Starting DNS traffic capture...');

  if (!existsSync(DNS_LOG_PATH)) {
    console.warn(`DNS log not found at ${DNS_LOG_PATH}. DNS capture will start when log file is created.`);
  } else {
    try {
      const stats = await stat(DNS_LOG_PATH);
      lastPosition = stats.size;
      console.log(`Starting DNS capture from position ${lastPosition} in ${DNS_LOG_PATH}`);
    } catch (error) {
      console.error('Error getting DNS log file stats:', error);
    }
  }

  pollingInterval = setInterval(pollLogFile, POLL_INTERVAL_MS);
  console.log('✓ DNS traffic capture started');
}

/**
 * Stop DNS traffic capture
 */
export function stopDnsCapture() {
  if (pollingInterval) {
    clearInterval(pollingInterval);
    pollingInterval = null;
  }

  if (batchTimer) {
    clearTimeout(batchTimer);
    batchTimer = null;
  }

  if (eventBatch.length > 0) {
    flushBatch();
  }

  console.log('DNS traffic capture stopped');
}

/**
 * Poll log file for new lines
 */
async function pollLogFile() {
  if (isProcessing) return;
  if (!existsSync(DNS_LOG_PATH)) return;

  isProcessing = true;

  try {
    const stats = await stat(DNS_LOG_PATH);
    const currentSize = stats.size;

    // File was truncated or rotated
    if (currentSize < lastPosition) {
      console.log('DNS log file rotated, resetting position');
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
    console.error('Error polling DNS log file:', error);
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
    const fileStream = createReadStream(DNS_LOG_PATH, {
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
 * Process a single DNS log line
 */
async function processLogLine(line) {
  try {
    // Parse dnsmasq log format: "Jan 25 16:44:47 dnsmasq[17278]: query[A] example.com from 10.0.0.50"
    const queryMatch = line.match(/query\[(\w+)\]\s+([\w\.\-]+)\s+from\s+([\w\.:]+)/);

    if (!queryMatch) return; // Skip non-query lines

    const [, queryType, domain, sourceIp] = queryMatch;

    // Skip reverse DNS queries
    if (domain.includes('.in-addr.arpa') || domain.includes('.ip6.arpa')) return;

    // Extract timestamp
    const timestampMatch = line.match(/^(\w+\s+\d+\s+\d+:\d+:\d+)/);
    const timestamp = timestampMatch ? parseTimestamp(timestampMatch[1]) : new Date();

    // Enrich with metadata
    const enrichedEvent = await enrichDnsEvent({
      timestamp,
      queryType,
      domain,
      sourceIp
    });

    // Add to batch
    eventBatch.push(enrichedEvent);

    // Flush if batch is full
    if (eventBatch.length >= BATCH_SIZE) {
      await flushBatch();
    } else if (!batchTimer) {
      batchTimer = setTimeout(flushBatch, BATCH_INTERVAL_MS);
    }
  } catch (error) {
    // Skip invalid lines
  }
}

/**
 * Parse dnsmasq timestamp (Jan 25 16:44:47)
 */
function parseTimestamp(timestampStr) {
  const now = new Date();
  const [month, day, time] = timestampStr.split(' ');
  const [hour, minute, second] = time.split(':');

  const months = {
    Jan: 0, Feb: 1, Mar: 2, Apr: 3, May: 4, Jun: 5,
    Jul: 6, Aug: 7, Sep: 8, Oct: 9, Nov: 10, Dec: 11
  };

  const date = new Date(
    now.getFullYear(),
    months[month],
    parseInt(day),
    parseInt(hour),
    parseInt(minute),
    parseInt(second)
  );

  // Handle year rollover
  if (date > now) {
    date.setFullYear(date.getFullYear() - 1);
  }

  return date;
}

/**
 * Enrich DNS event with metadata
 */
async function enrichDnsEvent({ timestamp, queryType, domain, sourceIp }) {
  // Update cache if needed
  await updateCaches();

  // Find device from DHCP leases
  const lease = dhcpLeasesCache.find(l => l.ip === sourceIp);

  // Categorize domain
  const category = categorizeDomain(domain);
  const baseDomain = extractBaseDomain(domain);

  return {
    timestamp,
    meta: {
      deviceMac: lease?.mac || null,
      deviceIp: sourceIp,
      deviceHostname: lease?.hostname || null,
      domain: baseDomain,
      fullDomain: domain,
      queryType,
      category
    },
    metrics: {
      queryCount: 1
    }
  };
}

/**
 * Categorize domain
 */
function categorizeDomain(domain) {
  for (const [key, category] of Object.entries(DOMAIN_CATEGORIES)) {
    if (domain.includes(key)) {
      return category;
    }
  }
  return 'Other';
}

/**
 * Extract base domain (e.g., youtube.com from www.youtube.com)
 */
function extractBaseDomain(domain) {
  const parts = domain.split('.');
  if (parts.length >= 2) {
    return parts.slice(-2).join('.');
  }
  return domain;
}

/**
 * Update DHCP leases cache
 */
async function updateCaches() {
  const now = Date.now();

  if (now - lastCacheUpdate > CACHE_TTL_MS) {
    try {
      const leasesResult = await getDhcpLeases();
      if (leasesResult.success) {
        dhcpLeasesCache = leasesResult.leases || [];
      }
      lastCacheUpdate = now;
    } catch (error) {
      console.error('Error updating DHCP leases cache:', error);
    }
  }
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
    // Store in a new collection for DNS queries
    const db = getCollection(COLLECTIONS.TRAFFIC_HTTP).s.db;
    const dnsCollection = db.collection('traffic_dns');

    await dnsCollection.insertMany(events);

    // Emit real-time update
    emitDnsUpdate(events);

    console.log(`✓ Inserted ${events.length} DNS query events`);
  } catch (error) {
    console.error('Error flushing DNS events to MongoDB:', error);
  }
}

/**
 * Emit real-time DNS update
 */
function emitDnsUpdate(events) {
  if (!events.length) return;

  try {
    // Count queries per category
    const categories = {};
    events.forEach(e => {
      const category = e.meta.category || 'Other';
      categories[category] = (categories[category] || 0) + 1;
    });

    trafficEvents.emit('dnsUpdate', {
      queriesPerSecond: events.length / (BATCH_INTERVAL_MS / 1000),
      categories,
      timestamp: new Date().toISOString()
    });
  } catch (error) {
    console.error('Error emitting DNS update:', error);
  }
}

// Graceful shutdown
process.on('SIGINT', stopDnsCapture);
process.on('SIGTERM', stopDnsCapture);
