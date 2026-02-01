import { promises as fs } from 'fs';
import path from 'path';
import { v4 as uuidv4 } from 'uuid';
import {
  setupServerSSHKey,
  testSSHKeyConnection,
  getRemoteNetworkInterfaces,
  executeSSHCommand
} from './sshKeys.js';

const SERVERS_FILE = '/data/servers.json';

/**
 * Ensure servers file exists
 */
async function ensureServersFile() {
  try {
    await fs.access(SERVERS_FILE);
  } catch (error) {
    await fs.writeFile(SERVERS_FILE, JSON.stringify({ servers: [] }, null, 2));
  }
}

/**
 * Read servers from file
 */
export async function readServers() {
  await ensureServersFile();
  const data = await fs.readFile(SERVERS_FILE, 'utf-8');
  return JSON.parse(data);
}

/**
 * Write servers to file
 */
async function writeServers(data) {
  await fs.writeFile(SERVERS_FILE, JSON.stringify(data, null, 2));
}

/**
 * Get all servers
 */
export async function listServers() {
  const data = await readServers();
  return data.servers || [];
}

/**
 * Get server by ID
 */
export async function getServerById(id) {
  const servers = await listServers();
  return servers.find(s => s.id === id);
}

/**
 * Add a new server
 * This will:
 * 1. Test SSH connection with password
 * 2. Add SSH key to server
 * 3. Get network interfaces
 * 4. Save server to database
 */
export async function addServer(serverData) {
  const { name, host, port = 22, username, password, groups = [], wolInterface } = serverData;

  if (!name || !host || !username || !password) {
    throw new Error('Missing required fields: name, host, username, password');
  }

  try {
    // Setup SSH key and get network interfaces
    console.log(`Setting up SSH key for server ${name} at ${host}...`);
    const setupResult = await setupServerSSHKey(host, port, username, password);

    // Find the selected interface or use the first one
    let selectedInterface;
    if (wolInterface) {
      selectedInterface = setupResult.interfaces.find(i => i.name === wolInterface);
    }
    if (!selectedInterface && setupResult.interfaces.length > 0) {
      selectedInterface = setupResult.interfaces[0];
    }

    if (!selectedInterface || !selectedInterface.mac) {
      throw new Error('No valid network interface with MAC address found');
    }

    // Create server object
    const server = {
      id: uuidv4(),
      name,
      host,
      port,
      username,
      interface: selectedInterface.name,
      mac: selectedInterface.mac,
      ipv4: selectedInterface.ipv4 || '',
      ipv6: selectedInterface.ipv6 || '',
      groups: groups || [],
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      status: 'unknown'
    };

    // Save to file
    const data = await readServers();
    data.servers.push(server);
    await writeServers(data);

    console.log(`Server ${name} added successfully with ID ${server.id}`);

    return {
      server,
      interfaces: setupResult.interfaces
    };
  } catch (error) {
    console.error(`Failed to add server ${name}:`, error);
    throw error;
  }
}

/**
 * Update server
 */
export async function updateServer(id, updates) {
  const data = await readServers();
  const index = data.servers.findIndex(s => s.id === id);

  if (index === -1) {
    throw new Error('Server not found');
  }

  // Update allowed fields
  const allowedFields = ['name', 'host', 'port', 'username', 'interface', 'mac', 'groups', 'ipv4', 'ipv6'];
  allowedFields.forEach(field => {
    if (updates[field] !== undefined) {
      data.servers[index][field] = updates[field];
    }
  });

  data.servers[index].updatedAt = new Date().toISOString();

  await writeServers(data);

  return data.servers[index];
}

/**
 * Delete server
 */
export async function deleteServer(id) {
  const data = await readServers();
  const index = data.servers.findIndex(s => s.id === id);

  if (index === -1) {
    throw new Error('Server not found');
  }

  const deleted = data.servers.splice(index, 1)[0];
  await writeServers(data);

  return deleted;
}

/**
 * Test server connection
 */
export async function testServerConnection(id) {
  const server = await getServerById(id);

  if (!server) {
    throw new Error('Server not found');
  }

  try {
    await testSSHKeyConnection(server.host, server.port, server.username);
    return { success: true, online: true, message: 'Connection successful' };
  } catch (error) {
    return { success: false, online: false, message: error.message };
  }
}

/**
 * Get network interfaces from server
 */
export async function getServerInterfaces(id) {
  const server = await getServerById(id);

  if (!server) {
    throw new Error('Server not found');
  }

  try {
    const interfaces = await getRemoteNetworkInterfaces(server.host, server.port, server.username);
    return interfaces;
  } catch (error) {
    console.error(`Failed to get interfaces for server ${id}:`, error);
    throw error;
  }
}

/**
 * Execute command on server
 */
export async function executeCommandOnServer(id, command) {
  const server = await getServerById(id);

  if (!server) {
    throw new Error('Server not found');
  }

  try {
    const result = await executeSSHCommand(server.host, server.port, server.username, command);
    return result;
  } catch (error) {
    console.error(`Failed to execute command on server ${id}:`, error);
    throw error;
  }
}

/**
 * Get server uptime
 */
export async function getServerUptime(id) {
  try {
    const result = await executeCommandOnServer(id, 'uptime -p 2>/dev/null || uptime');
    return result.stdout;
  } catch (error) {
    return null;
  }
}

/**
 * Get server hostname
 */
export async function getServerHostname(id) {
  try {
    const result = await executeCommandOnServer(id, 'hostname');
    return result.stdout;
  } catch (error) {
    return null;
  }
}

/**
 * Get server system info
 */
export async function getServerInfo(id) {
  const server = await getServerById(id);

  if (!server) {
    throw new Error('Server not found');
  }

  try {
    const commands = {
      hostname: 'hostname',
      kernel: 'uname -r',
      os: 'cat /etc/os-release | grep PRETTY_NAME | cut -d= -f2 | tr -d \'"\'',
      uptime: 'uptime -p 2>/dev/null || uptime',
      load: 'cat /proc/loadavg | awk \'{print $1, $2, $3}\'',
      memory: 'free -h | grep Mem | awk \'{print $3 "/" $2}\'',
      disk: 'df -h / | tail -1 | awk \'{print $3 "/" $2 " (" $5 ")"}\'',
    };

    const results = {};

    for (const [key, cmd] of Object.entries(commands)) {
      try {
        const result = await executeCommandOnServer(id, cmd);
        results[key] = result.success ? result.stdout : null;
      } catch (error) {
        results[key] = null;
      }
    }

    return {
      ...server,
      info: results
    };
  } catch (error) {
    console.error(`Failed to get server info for ${id}:`, error);
    throw error;
  }
}

/**
 * Refresh server network interfaces
 * Useful when server's network configuration has changed
 */
export async function refreshServerInterfaces(id) {
  const server = await getServerById(id);

  if (!server) {
    throw new Error('Server not found');
  }

  try {
    const interfaces = await getRemoteNetworkInterfaces(server.host, server.port, server.username);

    // Find current interface
    const currentInterface = interfaces.find(i => i.name === server.interface);

    if (currentInterface) {
      // Update server with new interface info
      await updateServer(id, {
        mac: currentInterface.mac,
        ipv4: currentInterface.ipv4 || '',
        ipv6: currentInterface.ipv6 || ''
      });
    }

    return interfaces;
  } catch (error) {
    console.error(`Failed to refresh interfaces for server ${id}:`, error);
    throw error;
  }
}

/**
 * Update server status
 */
export async function updateServerStatus(id, status, latency = null) {
  const data = await readServers();
  const server = data.servers.find(s => s.id === id);

  if (server) {
    server.status = status;
    server.lastSeen = new Date().toISOString();
    if (latency !== null) {
      server.latency = latency;
    }
    await writeServers(data);
  }
}

/**
 * Get servers by group
 */
export async function getServersByGroup(group) {
  const servers = await listServers();
  return servers.filter(s => s.groups && s.groups.includes(group));
}

/**
 * Get all groups
 */
export async function getAllGroups() {
  const servers = await listServers();
  const groupsSet = new Set();

  servers.forEach(server => {
    if (server.groups) {
      server.groups.forEach(group => groupsSet.add(group));
    }
  });

  return Array.from(groupsSet).sort();
}
