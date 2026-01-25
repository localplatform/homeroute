import { exec } from 'child_process';
import { promisify } from 'util';

const execAsync = promisify(exec);

export async function getNatRules() {
  try {
    const { stdout } = await execAsync('iptables -t nat -L -n -v --line-numbers 2>/dev/null || echo "[]"');
    const rules = parseIptablesOutput(stdout);
    return { success: true, rules };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getFilterRules() {
  try {
    const { stdout } = await execAsync('iptables -L -n -v --line-numbers 2>/dev/null || echo "[]"');
    const rules = parseIptablesOutput(stdout);
    return { success: true, rules };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

function parseIptablesOutput(output) {
  const chains = {};
  let currentChain = null;

  const lines = output.split('\n');

  for (const line of lines) {
    // Chain header
    const chainMatch = line.match(/^Chain (\S+) \(policy (\S+).*\)/);
    if (chainMatch) {
      currentChain = chainMatch[1];
      chains[currentChain] = {
        policy: chainMatch[2],
        rules: []
      };
      continue;
    }

    // Rule line (starts with number)
    if (currentChain && /^\d+/.test(line.trim())) {
      const parts = line.trim().split(/\s+/);
      if (parts.length >= 9) {
        chains[currentChain].rules.push({
          num: parseInt(parts[0]),
          pkts: parts[1],
          bytes: parts[2],
          target: parts[3],
          prot: parts[4],
          opt: parts[5],
          in: parts[6],
          out: parts[7],
          source: parts[8],
          destination: parts[9] || '*',
          extra: parts.slice(10).join(' ')
        });
      }
    }
  }

  return chains;
}

export async function getMasqueradeRules() {
  try {
    const { stdout } = await execAsync('iptables -t nat -L POSTROUTING -n -v 2>/dev/null');
    const lines = stdout.split('\n').filter(l => l.includes('MASQUERADE'));

    const rules = lines.map(line => {
      const parts = line.trim().split(/\s+/);
      return {
        pkts: parts[0],
        bytes: parts[1],
        source: parts[7],
        destination: parts[8],
        outInterface: parts[6] !== '*' ? parts[6] : null
      };
    });

    return { success: true, rules };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getPortForwards() {
  try {
    const { stdout } = await execAsync('iptables -t nat -L PREROUTING -n -v 2>/dev/null');
    const lines = stdout.split('\n').filter(l => l.includes('DNAT'));

    const rules = lines.map(line => {
      const parts = line.trim().split(/\s+/);
      const dnatMatch = line.match(/to:([0-9.:]+)/);
      const dptMatch = line.match(/dpt:(\d+)/);

      return {
        pkts: parts[0],
        bytes: parts[1],
        protocol: parts[3],
        inInterface: parts[5] !== '*' ? parts[5] : null,
        destinationPort: dptMatch ? dptMatch[1] : null,
        forwardTo: dnatMatch ? dnatMatch[1] : null
      };
    });

    return { success: true, rules };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getFirewallStatus() {
  try {
    // Detect firewall framework
    let framework = 'iptables';
    try {
      await execAsync('nft list tables 2>/dev/null');
      framework = 'nftables';
    } catch {
      // nftables not available, use iptables
    }

    // Check if firewall is active by listing rules
    const { stdout } = await execAsync('iptables -L -n 2>/dev/null | head -5');
    const active = stdout.includes('Chain');

    return {
      success: true,
      status: {
        active,
        framework,
        timestamp: new Date().toISOString()
      }
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getRoutingRules() {
  try {
    const { stdout } = await execAsync('ip -j rule show 2>/dev/null');
    const rules = JSON.parse(stdout);

    // Format rules for display
    const formatted = rules.map(rule => ({
      priority: rule.priority,
      src: rule.src || 'all',
      dst: rule.dst || 'all',
      table: rule.table || 'main',
      fwmark: rule.fwmark || null,
      action: rule.action || 'lookup'
    }));

    return { success: true, rules: formatted };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getChainStats() {
  try {
    const { stdout } = await execAsync('iptables -L -n -v -x 2>/dev/null');
    const stats = {};
    let currentChain = null;
    let totalPackets = 0;
    let totalBytes = 0;

    const lines = stdout.split('\n');
    for (const line of lines) {
      // Parse chain header with packet/byte counts
      const chainMatch = line.match(/^Chain (\S+) \(policy (\S+) (\d+) packets, (\d+) bytes\)/);
      if (chainMatch) {
        currentChain = chainMatch[1];
        const packets = parseInt(chainMatch[3]);
        const bytes = parseInt(chainMatch[4]);
        stats[currentChain] = { policy: chainMatch[2], packets, bytes };
        totalPackets += packets;
        totalBytes += bytes;
        continue;
      }

      // Also handle chains without policy stats (user-defined)
      const userChainMatch = line.match(/^Chain (\S+) \((\d+) references\)/);
      if (userChainMatch) {
        currentChain = userChainMatch[1];
        stats[currentChain] = { references: parseInt(userChainMatch[2]), packets: 0, bytes: 0 };
      }
    }

    return {
      success: true,
      stats: {
        chains: stats,
        total: { packets: totalPackets, bytes: totalBytes }
      }
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}
