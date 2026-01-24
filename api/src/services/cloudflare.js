import { exec } from 'child_process';
import { promisify } from 'util';
import { appendFile, mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import cron from 'node-cron';

const execAsync = promisify(exec);
const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DATA_DIR = path.join(__dirname, '../../data');
const LOG_FILE = path.join(DATA_DIR, 'ddns.log');

// In-memory logs (dernières 50 entrées)
let logs = [];
const MAX_LOGS = 50;

// Scheduler instance
let schedulerTask = null;

// Dernier statut connu
let lastUpdate = null;
let lastIp = null;
let lastError = null;

function getConfig() {
  return {
    apiToken: process.env.CF_API_TOKEN,
    zoneId: process.env.CF_ZONE_ID,
    recordName: process.env.CF_RECORD_NAME,
    interface: process.env.CF_INTERFACE || 'enp5s0',
    cronExpression: process.env.DDNS_CRON || '*/2 * * * *'
  };
}

async function log(message, level = 'INFO') {
  const timestamp = new Date().toISOString();
  const entry = `${timestamp} [${level}] ${message}`;

  logs.unshift(entry);
  if (logs.length > MAX_LOGS) {
    logs = logs.slice(0, MAX_LOGS);
  }

  // Écrire dans le fichier
  try {
    if (!existsSync(DATA_DIR)) {
      await mkdir(DATA_DIR, { recursive: true });
    }
    await appendFile(LOG_FILE, entry + '\n');
  } catch (err) {
    console.error('Erreur écriture log DDNS:', err.message);
  }

  console.log(`[DDNS] ${entry}`);
}

async function getCurrentIPv6(interfaceName) {
  try {
    const { stdout } = await execAsync(
      `ip -6 addr show ${interfaceName} scope global | grep -oP '2a0d:[0-9a-f:]+(?=/)' | head -1`
    );
    return stdout.trim() || null;
  } catch {
    return null;
  }
}

async function getCloudflareRecord(config) {
  const response = await fetch(
    `https://api.cloudflare.com/client/v4/zones/${config.zoneId}/dns_records?type=AAAA&name=${config.recordName}`,
    {
      headers: {
        'Authorization': `Bearer ${config.apiToken}`,
        'Content-Type': 'application/json'
      }
    }
  );

  if (!response.ok) {
    throw new Error(`Cloudflare API error: ${response.status}`);
  }

  const data = await response.json();

  if (!data.success) {
    throw new Error(`Cloudflare API error: ${data.errors?.[0]?.message || 'Unknown error'}`);
  }

  const record = data.result?.[0];
  return record ? { id: record.id, content: record.content } : null;
}

async function createDnsRecord(config, ip) {
  const response = await fetch(
    `https://api.cloudflare.com/client/v4/zones/${config.zoneId}/dns_records`,
    {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${config.apiToken}`,
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        type: 'AAAA',
        name: config.recordName,
        content: ip,
        ttl: 1,
        proxied: true
      })
    }
  );

  const data = await response.json();

  if (!data.success) {
    throw new Error(`Failed to create record: ${data.errors?.[0]?.message || 'Unknown error'}`);
  }

  return data.result;
}

async function updateDnsRecord(config, recordId, ip) {
  const response = await fetch(
    `https://api.cloudflare.com/client/v4/zones/${config.zoneId}/dns_records/${recordId}`,
    {
      method: 'PUT',
      headers: {
        'Authorization': `Bearer ${config.apiToken}`,
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        type: 'AAAA',
        name: config.recordName,
        content: ip,
        ttl: 1,
        proxied: true
      })
    }
  );

  const data = await response.json();

  if (!data.success) {
    throw new Error(`Failed to update record: ${data.errors?.[0]?.message || 'Unknown error'}`);
  }

  return data.result;
}

export async function runUpdate() {
  const config = getConfig();

  if (!config.apiToken || !config.zoneId || !config.recordName) {
    const msg = 'Configuration DDNS incomplète (CF_API_TOKEN, CF_ZONE_ID, CF_RECORD_NAME requis)';
    await log(msg, 'ERROR');
    lastError = msg;
    return { success: false, error: msg };
  }

  try {
    // Récupérer l'IPv6 actuelle
    const currentIp = await getCurrentIPv6(config.interface);

    if (!currentIp) {
      const msg = `Pas d'IPv6 globale sur ${config.interface}`;
      await log(msg, 'ERROR');
      lastError = msg;
      return { success: false, error: msg };
    }

    // Récupérer l'enregistrement actuel
    const record = await getCloudflareRecord(config);

    // Vérifier si mise à jour nécessaire
    if (record && record.content === currentIp) {
      lastError = null;
      return { success: true, message: 'IP unchanged', ip: currentIp };
    }

    // Créer ou mettre à jour
    if (!record) {
      await createDnsRecord(config, currentIp);
      await log(`CREE - ${config.recordName} -> ${currentIp}`);
    } else {
      await updateDnsRecord(config, record.id, currentIp);
      await log(`MAJ - ${config.recordName}: ${record.content} -> ${currentIp}`);
    }

    lastUpdate = new Date().toISOString();
    lastIp = currentIp;
    lastError = null;

    return { success: true, message: 'Updated', ip: currentIp, previousIp: record?.content };
  } catch (error) {
    const msg = `Erreur mise à jour: ${error.message}`;
    await log(msg, 'ERROR');
    lastError = error.message;
    return { success: false, error: error.message };
  }
}

export async function getStatus() {
  const config = getConfig();

  try {
    const currentIpv6 = await getCurrentIPv6(config.interface);

    // Récupérer l'enregistrement actuel depuis Cloudflare
    let cloudflareRecord = null;
    if (config.apiToken && config.zoneId && config.recordName) {
      try {
        cloudflareRecord = await getCloudflareRecord(config);
      } catch (err) {
        // Ignorer les erreurs API ici
      }
    }

    return {
      success: true,
      status: {
        config: {
          recordName: config.recordName,
          zoneId: config.zoneId ? config.zoneId.substring(0, 8) + '...' : null,
          apiToken: config.apiToken ? '***masked***' : null,
          interface: config.interface,
          cronExpression: config.cronExpression
        },
        currentIpv6,
        cloudflareIp: cloudflareRecord?.content || null,
        lastUpdate,
        lastIp,
        lastError,
        schedulerActive: schedulerTask !== null,
        logs
      }
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function forceUpdate() {
  const result = await runUpdate();
  const status = await getStatus();

  return {
    ...result,
    status: status.success ? status.status : null
  };
}

export function startScheduler() {
  const config = getConfig();

  if (!config.apiToken || !config.zoneId || !config.recordName) {
    console.log('[DDNS] Scheduler non démarré: configuration incomplète');
    return false;
  }

  if (schedulerTask) {
    console.log('[DDNS] Scheduler déjà actif');
    return true;
  }

  if (!cron.validate(config.cronExpression)) {
    console.error(`[DDNS] Expression cron invalide: ${config.cronExpression}`);
    return false;
  }

  schedulerTask = cron.schedule(config.cronExpression, async () => {
    await runUpdate();
  });

  console.log(`[DDNS] Scheduler démarré avec cron: ${config.cronExpression}`);

  // Exécuter immédiatement au démarrage
  runUpdate();

  return true;
}

export function stopScheduler() {
  if (schedulerTask) {
    schedulerTask.stop();
    schedulerTask = null;
    console.log('[DDNS] Scheduler arrêté');
    return true;
  }
  return false;
}
