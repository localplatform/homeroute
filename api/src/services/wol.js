import wol from 'wake_on_lan';
import { promisify } from 'util';
import { getServerById } from './servers.js';
import { executeSSHCommand } from './sshKeys.js';

const wake = promisify(wol.wake);

/**
 * Send Wake-on-LAN magic packet to server
 */
export async function sendWakeOnLan(serverId) {
  const server = await getServerById(serverId);

  if (!server) {
    throw new Error('Server not found');
  }

  if (!server.mac) {
    throw new Error('Server does not have a MAC address configured');
  }

  try {
    console.log(`Sending WOL magic packet to ${server.name} (${server.mac})...`);
    await wake(server.mac);
    console.log(`WOL magic packet sent successfully to ${server.name}`);

    return {
      success: true,
      message: `Wake-on-LAN magic packet sent to ${server.name}`,
      server: {
        id: server.id,
        name: server.name,
        mac: server.mac
      }
    };
  } catch (error) {
    console.error(`Failed to send WOL packet to ${server.name}:`, error);
    throw new Error(`Failed to send WOL packet: ${error.message}`);
  }
}

/**
 * Shutdown server via SSH
 */
export async function shutdownServer(serverId) {
  const server = await getServerById(serverId);

  if (!server) {
    throw new Error('Server not found');
  }

  try {
    console.log(`Shutting down server ${server.name}...`);

    // Try poweroff first (systemd), fallback to shutdown
    // No sudo needed since we connect as root
    const result = await executeSSHCommand(
      server.host,
      server.port,
      server.username,
      'poweroff || shutdown -h now'
    );

    console.log(`Shutdown command sent to ${server.name}`);

    return {
      success: true,
      message: `Shutdown command sent to ${server.name}`,
      server: {
        id: server.id,
        name: server.name,
        host: server.host
      },
      output: result.stdout || result.stderr
    };
  } catch (error) {
    console.error(`Failed to shutdown ${server.name}:`, error);
    throw new Error(`Failed to shutdown server: ${error.message}`);
  }
}

/**
 * Reboot server via SSH
 */
export async function rebootServer(serverId) {
  const server = await getServerById(serverId);

  if (!server) {
    throw new Error('Server not found');
  }

  try {
    console.log(`Rebooting server ${server.name}...`);

    // No sudo needed since we connect as root
    const result = await executeSSHCommand(
      server.host,
      server.port,
      server.username,
      'reboot'
    );

    console.log(`Reboot command sent to ${server.name}`);

    return {
      success: true,
      message: `Reboot command sent to ${server.name}`,
      server: {
        id: server.id,
        name: server.name,
        host: server.host
      },
      output: result.stdout || result.stderr
    };
  } catch (error) {
    console.error(`Failed to reboot ${server.name}:`, error);
    throw new Error(`Failed to reboot server: ${error.message}`);
  }
}

/**
 * Send WOL to multiple servers
 */
export async function sendWakeOnLanBulk(serverIds) {
  const results = [];

  for (const serverId of serverIds) {
    try {
      const result = await sendWakeOnLan(serverId);
      results.push({ serverId, success: true, ...result });
    } catch (error) {
      results.push({
        serverId,
        success: false,
        error: error.message
      });
    }
  }

  return results;
}

/**
 * Shutdown multiple servers
 */
export async function shutdownServersBulk(serverIds) {
  const results = [];

  for (const serverId of serverIds) {
    try {
      const result = await shutdownServer(serverId);
      results.push({ serverId, success: true, ...result });
    } catch (error) {
      results.push({
        serverId,
        success: false,
        error: error.message
      });
    }
  }

  return results;
}
