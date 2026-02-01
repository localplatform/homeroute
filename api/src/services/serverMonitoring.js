import ping from 'ping';
import { listServers, updateServerStatus } from './servers.js';
import { testSSHKeyConnection } from './sshKeys.js';
import { getIO } from '../socket.js';

// Monitoring interval in milliseconds (30 seconds)
const MONITORING_INTERVAL = 30000;

// Store monitoring interval ID
let monitoringIntervalId = null;

/**
 * Ping a server and return latency
 */
async function pingServer(host) {
  try {
    const result = await ping.promise.probe(host, {
      timeout: 5,
      min_reply: 1
    });

    return {
      online: result.alive,
      latency: result.alive ? Math.round(result.time) : null
    };
  } catch (error) {
    console.error(`Failed to ping ${host}:`, error);
    return {
      online: false,
      latency: null
    };
  }
}

/**
 * Test SSH connection to server
 */
async function testServerSSH(server) {
  try {
    await testSSHKeyConnection(server.host, server.port, server.username);
    return true;
  } catch (error) {
    return false;
  }
}

/**
 * Check server status (ping + optional SSH)
 */
export async function checkServerStatus(server) {
  try {
    // First, try ping
    const pingResult = await pingServer(server.host);

    let status = 'offline';
    let latency = null;

    if (pingResult.online) {
      status = 'online';
      latency = pingResult.latency;

      // Optionally test SSH connection (commented out to avoid too many SSH connections)
      // const sshOnline = await testServerSSH(server);
      // if (!sshOnline) {
      //   status = 'reachable'; // Ping works but SSH doesn't
      // }
    }

    return {
      serverId: server.id,
      online: pingResult.online,
      status,
      latency,
      lastSeen: new Date().toISOString()
    };
  } catch (error) {
    console.error(`Failed to check status for ${server.name}:`, error);
    return {
      serverId: server.id,
      online: false,
      status: 'offline',
      latency: null,
      lastSeen: new Date().toISOString()
    };
  }
}

/**
 * Monitor all servers
 */
async function monitorServers() {
  try {
    const servers = await listServers();

    if (servers.length === 0) {
      return;
    }

    console.log(`Monitoring ${servers.length} server(s)...`);

    // Check all servers in parallel
    const results = await Promise.all(
      servers.map(server => checkServerStatus(server))
    );

    // Update server statuses in database
    for (const result of results) {
      await updateServerStatus(result.serverId, result.status, result.latency);
    }

    // Emit WebSocket events
    const io = getIO();
    if (io) {
      for (const result of results) {
        io.emit('servers:status', result);
      }
    }

    console.log(`Monitoring complete: ${results.filter(r => r.online).length}/${results.length} online`);
  } catch (error) {
    console.error('Failed to monitor servers:', error);
  }
}

/**
 * Start monitoring service
 */
export function startMonitoring() {
  if (monitoringIntervalId) {
    console.warn('Server monitoring already running');
    return;
  }

  console.log('Starting server monitoring service...');

  // Run initial check immediately
  monitorServers();

  // Schedule periodic checks
  monitoringIntervalId = setInterval(() => {
    monitorServers();
  }, MONITORING_INTERVAL);

  console.log(`Server monitoring started (interval: ${MONITORING_INTERVAL / 1000}s)`);
}

/**
 * Stop monitoring service
 */
export function stopMonitoring() {
  if (monitoringIntervalId) {
    clearInterval(monitoringIntervalId);
    monitoringIntervalId = null;
    console.log('Server monitoring stopped');
  }
}

/**
 * Get monitoring status
 */
export function getMonitoringStatus() {
  return {
    running: monitoringIntervalId !== null,
    interval: MONITORING_INTERVAL
  };
}

/**
 * Force an immediate monitoring check
 */
export async function forceMonitoringCheck() {
  console.log('Forcing monitoring check...');
  await monitorServers();
  return { success: true, message: 'Monitoring check completed' };
}
