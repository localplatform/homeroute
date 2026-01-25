import { readFile } from 'fs/promises';
import { existsSync } from 'fs';

const DNSMASQ_CONFIG = process.env.DNSMASQ_CONFIG || '/etc/dnsmasq.d/lan.conf';
const DNSMASQ_LEASES = process.env.DNSMASQ_LEASES || '/var/lib/misc/dnsmasq.leases';

export async function getDnsConfig() {
  try {
    const content = await readFile(DNSMASQ_CONFIG, 'utf-8');
    const config = parseConfig(content);
    return { success: true, config, raw: content };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

function parseConfig(content) {
  const lines = content.split('\n');
  const config = {
    interface: null,
    dhcpRange: null,
    dhcpOptions: [],
    dnsServers: [],
    domain: null,
    cacheSize: null,
    ipv6: {},
    wildcardAddress: null,
    comments: []
  };

  for (const line of lines) {
    const trimmed = line.trim();

    if (trimmed.startsWith('#') || trimmed === '') {
      if (trimmed.startsWith('#') && !trimmed.startsWith('# ')) {
        config.comments.push(trimmed);
      }
      continue;
    }

    if (trimmed.startsWith('interface=')) {
      config.interface = trimmed.split('=')[1];
    } else if (trimmed.startsWith('dhcp-range=')) {
      const value = trimmed.split('=')[1];
      if (value.startsWith('::')) {
        config.ipv6.dhcpRange = value;
      } else {
        config.dhcpRange = value;
      }
    } else if (trimmed.startsWith('dhcp-option=')) {
      const value = trimmed.split('=')[1];
      if (value.startsWith('option6:')) {
        config.ipv6.options = config.ipv6.options || [];
        config.ipv6.options.push(value);
      } else {
        config.dhcpOptions.push(value);
      }
    } else if (trimmed.startsWith('server=')) {
      config.dnsServers.push(trimmed.split('=')[1]);
    } else if (trimmed.startsWith('domain=')) {
      config.domain = trimmed.split('=')[1];
    } else if (trimmed.startsWith('cache-size=')) {
      config.cacheSize = parseInt(trimmed.split('=')[1]);
    } else if (trimmed.startsWith('address=')) {
      const parts = trimmed.split('=')[1].split('/');
      config.wildcardAddress = { domain: parts[1], ip: parts[2] };
    } else if (trimmed === 'enable-ra') {
      config.ipv6.raEnabled = true;
    }
  }

  return config;
}

export async function getDhcpLeases() {
  try {
    if (!existsSync(DNSMASQ_LEASES)) {
      return { success: true, leases: [] };
    }

    const content = await readFile(DNSMASQ_LEASES, 'utf-8');
    const leases = parseLeases(content);
    return { success: true, leases };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

function parseLeases(content) {
  const lines = content.split('\n').filter(l => l.trim());
  const leases = [];

  for (const line of lines) {
    try {
      const parts = line.split(' ');

      // Validate minimum required fields (timestamp, MAC, IP)
      if (parts.length < 3) {
        console.warn(`[DHCP] Skipping invalid lease line (not enough parts): ${line}`);
        continue;
      }

      leases.push({
        expiration: new Date(parseInt(parts[0]) * 1000).toISOString(),
        expirationTimestamp: parseInt(parts[0]),
        mac: parts[1],
        ip: parts[2],
        hostname: parts[3] && parts[3] !== '*' ? parts[3] : null,
        clientId: parts[4] || null
      });
    } catch (error) {
      console.warn(`[DHCP] Skipping invalid lease line: ${line}`, error.message);
    }
  }

  return leases.sort((a, b) => {
    // Sort by IP address
    const ipA = a.ip.split('.').map(Number);
    const ipB = b.ip.split('.').map(Number);
    for (let i = 0; i < 4; i++) {
      if (ipA[i] !== ipB[i]) return ipA[i] - ipB[i];
    }
    return 0;
  });
}
