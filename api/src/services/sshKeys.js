import { promises as fs } from 'fs';
import path from 'path';
import { exec } from 'child_process';
import { promisify } from 'util';
import { Client } from 'ssh2';

const execAsync = promisify(exec);

// Paths for SSH keys
const SSH_DIR = '/data/ssh';
const PRIVATE_KEY_PATH = path.join(SSH_DIR, 'id_rsa');
const PUBLIC_KEY_PATH = path.join(SSH_DIR, 'id_rsa.pub');

/**
 * Ensure SSH directory exists
 */
async function ensureSSHDir() {
  try {
    await fs.access(SSH_DIR);
  } catch (error) {
    await fs.mkdir(SSH_DIR, { recursive: true, mode: 0o700 });
  }
}

/**
 * Generate SSH key pair if it doesn't exist
 */
export async function ensureKeyPair() {
  await ensureSSHDir();

  try {
    await fs.access(PRIVATE_KEY_PATH);
    await fs.access(PUBLIC_KEY_PATH);
    console.log('SSH key pair already exists');
    return true;
  } catch (error) {
    console.log('Generating new SSH key pair...');

    try {
      await execAsync(
        `ssh-keygen -t rsa -b 4096 -f ${PRIVATE_KEY_PATH} -N "" -C "homeroute@server-dashboard"`
      );

      // Set correct permissions
      await fs.chmod(PRIVATE_KEY_PATH, 0o600);
      await fs.chmod(PUBLIC_KEY_PATH, 0o644);

      console.log('SSH key pair generated successfully');
      return true;
    } catch (genError) {
      console.error('Failed to generate SSH key pair:', genError);
      throw new Error('Failed to generate SSH key pair');
    }
  }
}

/**
 * Get the public key content
 */
export async function getPublicKey() {
  await ensureKeyPair();
  return await fs.readFile(PUBLIC_KEY_PATH, 'utf-8');
}

/**
 * Get the private key content
 */
async function getPrivateKey() {
  await ensureKeyPair();
  return await fs.readFile(PRIVATE_KEY_PATH, 'utf-8');
}

/**
 * Test SSH connection with password
 */
export async function testSSHConnection(host, port, username, password) {
  return new Promise((resolve, reject) => {
    const conn = new Client();

    const timeout = setTimeout(() => {
      conn.end();
      reject(new Error('Connection timeout'));
    }, 15000);

    conn.on('ready', () => {
      clearTimeout(timeout);
      conn.end();
      resolve({ success: true, message: 'Connection successful' });
    });

    conn.on('error', (err) => {
      clearTimeout(timeout);
      reject(err);
    });

    conn.connect({
      host,
      port: port || 22,
      username,
      password,
      readyTimeout: 15000,
    });
  });
}

/**
 * Test SSH connection with private key (always connects as root)
 */
export async function testSSHKeyConnection(host, port, username) {
  const privateKey = await getPrivateKey();

  return new Promise((resolve, reject) => {
    const conn = new Client();

    const timeout = setTimeout(() => {
      conn.end();
      reject(new Error('Connection timeout'));
    }, 15000);

    conn.on('ready', () => {
      clearTimeout(timeout);
      conn.end();
      resolve({ success: true, message: 'Key-based connection successful' });
    });

    conn.on('error', (err) => {
      clearTimeout(timeout);
      reject(err);
    });

    conn.connect({
      host,
      port: port || 22,
      username: 'root',  // Always connect as root with the key
      privateKey,
      readyTimeout: 15000,
    });
  });
}

/**
 * Add public key to remote server's authorized_keys
 */
export async function addPublicKeyToServer(host, port, username, password) {
  const publicKey = (await getPublicKey()).trim();

  return new Promise((resolve, reject) => {
    const conn = new Client();

    const timeout = setTimeout(() => {
      conn.end();
      reject(new Error('Connection timeout'));
    }, 30000);

    conn.on('ready', () => {
      // Create .ssh directory and add public key to root
      const commands = [
        'sudo mkdir -p /root/.ssh',
        'sudo chmod 700 /root/.ssh',
        'sudo touch /root/.ssh/authorized_keys',
        'sudo chmod 600 /root/.ssh/authorized_keys',
        `sudo grep -qF '${publicKey}' /root/.ssh/authorized_keys || echo '${publicKey}' | sudo tee -a /root/.ssh/authorized_keys > /dev/null`
      ].join(' && ');

      conn.exec(commands, (err, stream) => {
        if (err) {
          clearTimeout(timeout);
          conn.end();
          return reject(err);
        }

        let stdout = '';
        let stderr = '';

        stream.on('close', (code) => {
          clearTimeout(timeout);
          conn.end();

          if (code === 0) {
            resolve({
              success: true,
              message: 'Public key added successfully',
              stdout,
              stderr
            });
          } else {
            reject(new Error(`Command failed with code ${code}: ${stderr}`));
          }
        });

        stream.on('data', (data) => {
          stdout += data.toString();
        });

        stream.stderr.on('data', (data) => {
          stderr += data.toString();
        });
      });
    });

    conn.on('error', (err) => {
      clearTimeout(timeout);
      reject(err);
    });

    conn.connect({
      host,
      port: port || 22,
      username,
      password,
      readyTimeout: 15000,
    });
  });
}

/**
 * Execute command on remote server using SSH key (always as root)
 */
export async function executeSSHCommand(host, port, username, command) {
  const privateKey = await getPrivateKey();

  return new Promise((resolve, reject) => {
    const conn = new Client();

    const timeout = setTimeout(() => {
      conn.end();
      reject(new Error('Connection timeout'));
    }, 30000);

    conn.on('ready', () => {
      conn.exec(command, (err, stream) => {
        if (err) {
          clearTimeout(timeout);
          conn.end();
          return reject(err);
        }

        let stdout = '';
        let stderr = '';

        stream.on('close', (code) => {
          clearTimeout(timeout);
          conn.end();

          resolve({
            code,
            stdout: stdout.trim(),
            stderr: stderr.trim(),
            success: code === 0
          });
        });

        stream.on('data', (data) => {
          stdout += data.toString();
        });

        stream.stderr.on('data', (data) => {
          stderr += data.toString();
        });
      });
    });

    conn.on('error', (err) => {
      clearTimeout(timeout);
      reject(err);
    });

    conn.connect({
      host,
      port: port || 22,
      username: 'root',  // Always connect as root with the key
      privateKey,
      readyTimeout: 15000,
    });
  });
}

/**
 * Get network interfaces from remote server
 */
export async function getRemoteNetworkInterfaces(host, port, username) {
  try {
    // Get interface list with IPs and MACs
    const result = await executeSSHCommand(
      host,
      port,
      username,
      "ip -j addr show | grep -v '127.0.0.1' || ip addr show | awk '/^[0-9]+:/ {iface=$2; gsub(/:/, \"\", iface)} /inet / && !/127.0.0.1/ {print iface, $2} /link\\/ether/ {mac=$2; print iface, mac}'"
    );

    if (!result.success) {
      throw new Error(`Failed to get network interfaces: ${result.stderr}`);
    }

    // Try parsing JSON output first (modern systems)
    try {
      const interfaces = JSON.parse(result.stdout);
      return interfaces
        .filter(iface => !iface.ifname.startsWith('lo') && iface.address)
        .map(iface => {
          const ipv4 = iface.addr_info?.find(a => a.family === 'inet')?.local || '';
          const ipv6 = iface.addr_info?.find(a => a.family === 'inet6' && !a.local.startsWith('fe80'))?.local || '';

          return {
            name: iface.ifname,
            mac: iface.address || '',
            ipv4,
            ipv6,
            state: iface.operstate || 'unknown'
          };
        });
    } catch {
      // Fallback to parsing text output
      const lines = result.stdout.split('\n').filter(l => l.trim());
      const interfaces = {};

      lines.forEach(line => {
        const parts = line.trim().split(/\s+/);
        if (parts.length >= 2) {
          const iface = parts[0];
          const value = parts[1];

          if (!interfaces[iface]) {
            interfaces[iface] = { name: iface, mac: '', ipv4: '', ipv6: '', state: 'unknown' };
          }

          if (value.includes(':') && value.split(':').length === 6) {
            interfaces[iface].mac = value;
          } else if (value.includes('.')) {
            interfaces[iface].ipv4 = value.split('/')[0];
          } else if (value.includes('::')) {
            interfaces[iface].ipv6 = value.split('/')[0];
          }
        }
      });

      return Object.values(interfaces).filter(i => i.mac && !i.name.startsWith('lo'));
    }
  } catch (error) {
    console.error('Failed to get remote network interfaces:', error);
    throw error;
  }
}

/**
 * Setup SSH key authentication on a remote server
 * This is the main function to call when adding a new server
 */
export async function setupServerSSHKey(host, port, username, password) {
  try {
    // Ensure we have a key pair
    await ensureKeyPair();

    // Test initial connection with password
    await testSSHConnection(host, port, username, password);

    // Add public key to server
    await addPublicKeyToServer(host, port, username, password);

    // Test key-based connection
    await testSSHKeyConnection(host, port, username);

    // Get network interfaces
    const interfaces = await getRemoteNetworkInterfaces(host, port, username);

    return {
      success: true,
      message: 'SSH key authentication setup successfully',
      interfaces
    };
  } catch (error) {
    console.error('Failed to setup SSH key:', error);
    throw error;
  }
}
