import { readFile, writeFile, appendFile } from 'fs/promises';
import { existsSync } from 'fs';
import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

const ADBLOCK_HOSTS = process.env.ADBLOCK_HOSTS || '/var/lib/dnsmasq/adblock-hosts.txt';
const ADBLOCK_WHITELIST = process.env.ADBLOCK_WHITELIST || '/var/lib/dnsmasq/adblock-whitelist.txt';
const ADBLOCK_LOG = process.env.ADBLOCK_LOG || '/var/log/homeroute/adblock.log';
const ADBLOCK_SCRIPT = process.env.ADBLOCK_SCRIPT || '/usr/local/bin/update-adblock-lists.sh';

export async function getStats() {
  try {
    let domainCount = 0;
    let lastUpdate = null;
    let logs = [];

    // Count domains
    if (existsSync(ADBLOCK_HOSTS)) {
      const { stdout } = await execAsync(`wc -l < ${ADBLOCK_HOSTS}`);
      domainCount = parseInt(stdout.trim());
    }

    // Get last modification time
    if (existsSync(ADBLOCK_HOSTS)) {
      const { stdout } = await execAsync(`stat -c %Y ${ADBLOCK_HOSTS}`);
      lastUpdate = new Date(parseInt(stdout.trim()) * 1000).toISOString();
    }

    // Get recent logs
    if (existsSync(ADBLOCK_LOG)) {
      const { stdout } = await execAsync(`tail -50 ${ADBLOCK_LOG}`);
      logs = stdout.split('\n').filter(l => l.trim());
    }

    // Sources (hardcoded from script)
    const sources = [
      { name: 'StevenBlack', url: 'https://raw.githubusercontent.com/StevenBlack/hosts/master/hosts' },
      { name: 'AdAway', url: 'https://adaway.org/hosts.txt' },
      { name: 'OISD Basic', url: 'https://small.oisd.nl/domainswild' },
      { name: 'URLhaus', url: 'https://urlhaus.abuse.ch/downloads/hostfile/' }
    ];

    return {
      success: true,
      stats: {
        domainCount,
        lastUpdate,
        sources,
        logs
      }
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getWhitelist() {
  try {
    if (!existsSync(ADBLOCK_WHITELIST)) {
      return { success: true, domains: [] };
    }

    const content = await readFile(ADBLOCK_WHITELIST, 'utf-8');
    const domains = content
      .split('\n')
      .map(l => l.trim())
      .filter(l => l && !l.startsWith('#'));

    return { success: true, domains };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function addToWhitelist(domain) {
  try {
    // Validate domain
    if (!/^[a-zA-Z0-9][a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$/.test(domain)) {
      return { success: false, error: 'Invalid domain format' };
    }

    // Check if already exists
    const { domains } = await getWhitelist();
    if (domains.includes(domain)) {
      return { success: false, error: 'Domain already in whitelist' };
    }

    // Append to file
    await appendFile(ADBLOCK_WHITELIST, domain + '\n');

    return { success: true, message: `Added ${domain} to whitelist` };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function removeFromWhitelist(domain) {
  try {
    const { domains } = await getWhitelist();
    const newDomains = domains.filter(d => d !== domain);

    if (domains.length === newDomains.length) {
      return { success: false, error: 'Domain not found in whitelist' };
    }

    await writeFile(ADBLOCK_WHITELIST, newDomains.join('\n') + '\n');

    return { success: true, message: `Removed ${domain} from whitelist` };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function updateLists() {
  try {
    if (!existsSync(ADBLOCK_SCRIPT)) {
      return { success: false, error: 'Update script not found' };
    }

    const { stdout, stderr } = await execAsync(`sudo ${ADBLOCK_SCRIPT}`, { timeout: 300000 });

    return {
      success: true,
      message: 'Update completed',
      output: stdout + stderr
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function searchBlocked(query) {
  try {
    if (!existsSync(ADBLOCK_HOSTS)) {
      return { success: true, results: [] };
    }

    const { stdout } = await execAsync(`grep -i "${query}" ${ADBLOCK_HOSTS} | head -100`);
    const results = stdout
      .split('\n')
      .filter(l => l.trim())
      .map(l => l.split(' ')[1])
      .filter(Boolean);

    return { success: true, results };
  } catch (error) {
    // grep returns exit 1 if no matches
    if (error.code === 1) {
      return { success: true, results: [] };
    }
    return { success: false, error: error.message };
  }
}
