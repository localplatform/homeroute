import { exec } from 'child_process';
import { promisify } from 'util';
import { getCollection, COLLECTIONS } from './mongodb.js';
import { trafficEvents } from './traffic.js';

const execAsync = promisify(exec);

const POLLING_INTERVAL_MS = parseInt(process.env.NETWORK_POLLING_INTERVAL_MS || '10000');
const CHAINS = ['INPUT', 'OUTPUT', 'FORWARD'];

let pollingInterval = null;
let prevStats = null;
let networkMetrics = {
  bytesPerSecond: 0,
  packetsPerSecond: 0
};

/**
 * Start network traffic capture from iptables
 */
export function startNetworkCapture() {
  console.log('Starting network traffic capture...');

  // Initial capture
  captureNetworkStats();

  // Start polling
  pollingInterval = setInterval(captureNetworkStats, POLLING_INTERVAL_MS);

  console.log(`✓ Network traffic capture started (polling every ${POLLING_INTERVAL_MS}ms)`);
}

/**
 * Stop network traffic capture
 */
export function stopNetworkCapture() {
  if (pollingInterval) {
    clearInterval(pollingInterval);
    pollingInterval = null;
  }

  console.log('Network traffic capture stopped');
}

/**
 * Capture network stats from iptables
 */
async function captureNetworkStats() {
  try {
    const stats = await parseIptablesStats();

    // Calculate deltas if we have previous stats
    if (prevStats) {
      const deltaTime = (Date.now() - prevStats.timestamp) / 1000; // seconds

      const events = [];

      for (const chain of CHAINS) {
        if (!stats[chain] || !prevStats[chain]) continue;

        const deltaBytes = stats[chain].bytes - prevStats[chain].bytes;
        const deltaPackets = stats[chain].packets - prevStats[chain].packets;

        // Avoid negative deltas (can happen if counters reset)
        if (deltaBytes < 0 || deltaPackets < 0) continue;

        const bytesPerSecond = deltaBytes / deltaTime;
        const packetsPerSecond = deltaPackets / deltaTime;

        events.push({
          timestamp: new Date(),
          meta: {
            chain,
            interface: null // Could be enhanced to track per-interface
          },
          metrics: {
            bytesPerSecond,
            packetsPerSecond,
            totalBytes: stats[chain].bytes,
            totalPackets: stats[chain].packets
          }
        });

        // Update metrics for real-time updates (aggregate all chains)
        if (chain === 'FORWARD') {
          // Use FORWARD chain as primary metric (router traffic)
          networkMetrics.bytesPerSecond = bytesPerSecond;
          networkMetrics.packetsPerSecond = packetsPerSecond;
        }
      }

      // Insert events to MongoDB
      if (events.length > 0) {
        const collection = getCollection(COLLECTIONS.TRAFFIC_NETWORK);
        await collection.insertMany(events);
        console.log(`✓ Inserted ${events.length} network traffic events`);

        // Emit real-time update
        emitNetworkUpdate();
      }
    }

    // Update previous stats
    prevStats = {
      ...stats,
      timestamp: Date.now()
    };
  } catch (error) {
    console.error('Error capturing network stats:', error);
  }
}

/**
 * Parse iptables statistics
 */
async function parseIptablesStats() {
  try {
    const { stdout } = await execAsync('sudo iptables -L -n -v -x');

    const stats = {};

    for (const chain of CHAINS) {
      // Match: Chain FORWARD (policy ACCEPT 1720000 packets, 2311000000 bytes)
      const regex = new RegExp(`Chain ${chain} \\(policy \\w+ (\\d+) packets, (\\d+) bytes\\)`, 'i');
      const match = stdout.match(regex);

      if (match) {
        stats[chain] = {
          packets: parseInt(match[1]),
          bytes: parseInt(match[2])
        };
      } else {
        // Try to get from first line of chain
        const chainRegex = new RegExp(`Chain ${chain}[^\\n]*\\n\\s*(\\d+)\\s+(\\d+)`, 'i');
        const chainMatch = stdout.match(chainRegex);

        if (chainMatch) {
          stats[chain] = {
            packets: parseInt(chainMatch[1]),
            bytes: parseInt(chainMatch[2])
          };
        } else {
          stats[chain] = { packets: 0, bytes: 0 };
        }
      }
    }

    return stats;
  } catch (error) {
    console.error('Error parsing iptables stats:', error);
    throw error;
  }
}

/**
 * Emit real-time network update
 */
function emitNetworkUpdate() {
  try {
    const bandwidthMbps = (networkMetrics.bytesPerSecond / (1024 * 1024)) * 8; // Convert to Mbps

    trafficEvents.emit('networkUpdate', {
      bandwidthMbps: parseFloat(bandwidthMbps.toFixed(3)),
      packetsPerSecond: Math.round(networkMetrics.packetsPerSecond),
      timestamp: new Date().toISOString()
    });
  } catch (error) {
    console.error('Error emitting network update:', error);
  }
}

/**
 * Get current network metrics
 */
export function getCurrentNetworkMetrics() {
  return {
    ...networkMetrics,
    bandwidthMbps: (networkMetrics.bytesPerSecond / (1024 * 1024)) * 8
  };
}

// Graceful shutdown
process.on('SIGINT', stopNetworkCapture);
process.on('SIGTERM', stopNetworkCapture);
