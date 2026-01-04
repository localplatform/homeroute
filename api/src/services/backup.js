import { readFile, writeFile, mkdir } from 'fs/promises';
import { existsSync } from 'fs';
import { exec, spawn } from 'child_process';
import { promisify } from 'util';
import path from 'path';
import dgram from 'dgram';
import { getIO } from '../socket.js';

const execAsync = promisify(exec);

// Wake-on-LAN: envoie un magic packet UDP
export function sendWakeOnLan(macAddress) {
  return new Promise((resolve, reject) => {
    // Valider et parser l'adresse MAC
    const macClean = macAddress.replace(/[:-]/g, '').toLowerCase();
    if (!/^[0-9a-f]{12}$/.test(macClean)) {
      return reject(new Error('Invalid MAC address format'));
    }

    // Créer le magic packet: 6x 0xFF + 16x MAC address
    const macBytes = Buffer.from(macClean, 'hex');
    const magicPacket = Buffer.alloc(6 + 16 * 6);

    // 6 bytes de 0xFF
    for (let i = 0; i < 6; i++) {
      magicPacket[i] = 0xff;
    }

    // 16 répétitions de l'adresse MAC
    for (let i = 0; i < 16; i++) {
      macBytes.copy(magicPacket, 6 + i * 6);
    }

    // Envoyer en broadcast UDP sur port 9
    const socket = dgram.createSocket('udp4');
    socket.once('error', (err) => {
      socket.close();
      reject(err);
    });

    socket.bind(() => {
      socket.setBroadcast(true);
      socket.send(magicPacket, 0, magicPacket.length, 9, '255.255.255.255', (err) => {
        socket.close();
        if (err) {
          reject(err);
        } else {
          resolve({ success: true, message: `WOL packet sent to ${macAddress}` });
        }
      });
    });
  });
}

// Ping un serveur pour vérifier s'il est en ligne
export async function pingServer(ip, timeoutMs = 2000) {
  try {
    const timeoutSec = Math.ceil(timeoutMs / 1000);
    const startTime = Date.now();
    await execAsync(`ping -c 1 -W ${timeoutSec} ${ip}`, { timeout: timeoutMs + 1000 });
    const pingMs = Date.now() - startTime;
    return { online: true, pingMs };
  } catch {
    return { online: false, pingMs: null };
  }
}

// Attendre que le serveur soit prêt (ping + SMB accessible) - pas de timeout
export async function waitForServer() {
  const { SMB_SERVER } = getEnv();
  const io = getIO();
  const startTime = Date.now();
  let attempt = 0;

  // Étape 1: Attendre le ping (indéfiniment)
  while (true) {
    attempt++;
    io.emit('backup:wol-ping-waiting', { attempt, elapsed: Math.floor((Date.now() - startTime) / 1000) });

    const result = await pingServer(SMB_SERVER, 1000);
    if (result.online) {
      io.emit('backup:wol-ping-ok', { pingMs: result.pingMs });
      break;
    }
  }

  // Étape 2: Vérifier accès SMB (indéfiniment)
  io.emit('backup:wol-smb-waiting');

  while (true) {
    try {
      await mountSmb();
      await unmountSmb();
      io.emit('backup:wol-ready');
      return { success: true };
    } catch {
      // Attendre avant la prochaine tentative
      await new Promise(resolve => setTimeout(resolve, 2000));
    }
  }
}

// État du backup actif
let activeBackupProcess = null;
let backupCancelled = false;

// Getters pour lire les variables après dotenv.config()
const getEnv = () => ({
  SMB_SERVER: process.env.SMB_SERVER || '',
  SMB_SHARE: process.env.SMB_SHARE || '',
  SMB_USERNAME: process.env.SMB_USERNAME || '',
  SMB_PASSWORD: process.env.SMB_PASSWORD || '',
  SMB_MOUNT_POINT: process.env.SMB_MOUNT_POINT || '/mnt/smb_backup',
  BACKUP_CONFIG_FILE: process.env.BACKUP_CONFIG_FILE || '/var/lib/server-dashboard/backup-config.json',
  BACKUP_HISTORY_FILE: process.env.BACKUP_HISTORY_FILE || '/var/lib/server-dashboard/backup-history.json'
});

async function ensureConfigDir() {
  const { BACKUP_CONFIG_FILE } = getEnv();
  const configDir = path.dirname(BACKUP_CONFIG_FILE);
  if (!existsSync(configDir)) {
    await mkdir(configDir, { recursive: true });
  }
}

export async function getConfig() {
  try {
    const env = getEnv();
    let sources = [];
    let wolMacAddress = '';

    if (existsSync(env.BACKUP_CONFIG_FILE)) {
      const content = await readFile(env.BACKUP_CONFIG_FILE, 'utf-8');
      const config = JSON.parse(content);
      sources = config.sources || [];
      wolMacAddress = config.wolMacAddress || '';
    }

    return {
      success: true,
      config: {
        smbServer: env.SMB_SERVER,
        smbShare: env.SMB_SHARE,
        smbUsername: env.SMB_USERNAME,
        smbPasswordSet: !!env.SMB_PASSWORD,
        mountPoint: env.SMB_MOUNT_POINT,
        sources,
        wolMacAddress
      }
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function saveConfig({ sources, wolMacAddress }) {
  try {
    const { BACKUP_CONFIG_FILE } = getEnv();
    await ensureConfigDir();

    // Charger la config existante pour préserver les autres champs
    let existingConfig = {};
    if (existsSync(BACKUP_CONFIG_FILE)) {
      const content = await readFile(BACKUP_CONFIG_FILE, 'utf-8');
      existingConfig = JSON.parse(content);
    }

    const newConfig = {
      ...existingConfig,
      sources: sources !== undefined ? sources : existingConfig.sources || [],
      wolMacAddress: wolMacAddress !== undefined ? wolMacAddress : existingConfig.wolMacAddress || ''
    };

    await writeFile(BACKUP_CONFIG_FILE, JSON.stringify(newConfig, null, 2));
    return { success: true, message: 'Configuration saved' };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

async function isMounted() {
  try {
    const { SMB_MOUNT_POINT } = getEnv();
    const { stdout } = await execAsync(`mount | grep "${SMB_MOUNT_POINT}"`);
    return !!stdout.trim();
  } catch {
    return false;
  }
}

async function mountSmb() {
  const env = getEnv();

  if (!env.SMB_SERVER || !env.SMB_SHARE) {
    throw new Error('SMB server or share not configured');
  }

  if (!existsSync(env.SMB_MOUNT_POINT)) {
    await execAsync(`sudo mkdir -p "${env.SMB_MOUNT_POINT}"`);
  }

  if (await isMounted()) {
    return { success: true, message: 'Already mounted' };
  }

  // Utiliser les options directes avec single quotes pour le mot de passe
  try {
    const mountOptions = env.SMB_USERNAME
      ? `'username=${env.SMB_USERNAME},password=${env.SMB_PASSWORD},vers=3.0,uid=0,gid=0'`
      : `'guest,vers=3.0,uid=0,gid=0'`;

    const mountCmd = `sudo mount -t cifs "//${env.SMB_SERVER}/${env.SMB_SHARE}" "${env.SMB_MOUNT_POINT}" -o ${mountOptions}`;
    await execAsync(mountCmd, { timeout: 30000 });

    return { success: true, message: 'Mounted successfully' };
  } catch (error) {
    throw new Error(`Failed to mount SMB share: ${error.message}`);
  }
}

async function unmountSmb() {
  try {
    const { SMB_MOUNT_POINT } = getEnv();
    if (await isMounted()) {
      await execAsync(`sudo umount "${SMB_MOUNT_POINT}"`);
    }
    return { success: true };
  } catch (error) {
    return { success: false, message: error.message };
  }
}

export async function testConnection() {
  try {
    const { SMB_MOUNT_POINT } = getEnv();
    await mountSmb();

    // Vérifier qu'on peut lister le contenu (lecture seule suffit)
    await execAsync(`ls "${SMB_MOUNT_POINT}"`);

    await unmountSmb();

    return { success: true, message: 'SMB connection successful' };
  } catch (error) {
    await unmountSmb();
    return { success: false, error: error.message };
  }
}

function parseRsyncStats(output) {
  const stats = {
    filesTransferred: 0,
    totalSize: 0,
    transferredSize: 0,
    speed: ''
  };

  const filesMatch = output.match(/Number of regular files transferred: ([\d,]+)/);
  if (filesMatch) {
    stats.filesTransferred = parseInt(filesMatch[1].replace(/,/g, ''));
  }

  const sizeMatch = output.match(/Total file size: ([\d,]+)/);
  if (sizeMatch) {
    stats.totalSize = parseInt(sizeMatch[1].replace(/,/g, ''));
  }

  const transferMatch = output.match(/Total transferred file size: ([\d,]+)/);
  if (transferMatch) {
    stats.transferredSize = parseInt(transferMatch[1].replace(/,/g, ''));
  }

  const speedMatch = output.match(/([\d.]+[KMG]?B\/s)/);
  if (speedMatch) {
    stats.speed = speedMatch[1];
  }

  return stats;
}

// Parse rsync --info=progress2 output
// Format EN: "     32,768 100%    2.08MB/s    0:00:00"
// Format FR: "  1.049.919.488   0% 1001,25MB/s    0:09:03"
function parseRsyncProgress(line) {
  // Match numbers with dots or commas as thousand separators, then percentage, then speed
  const match = line.match(/^\s*([\d.,]+)\s+(\d+)%\s+([\d.,]+\s*[KMG]?B\/s)/);
  if (match) {
    // Remove thousand separators (both . and ,) but keep the number
    const bytesStr = match[1].replace(/[.,]/g, '');
    return {
      transferredBytes: parseInt(bytesStr),
      percent: parseInt(match[2]),
      speed: match[3].replace(/\s+/g, '')
    };
  }
  return null;
}

// Run rsync with spawn for streaming progress
function runRsyncWithProgress(source, destPath, sourceIndex, sourceName, sourcesCount) {
  return new Promise((resolve, reject) => {
    // Use stdbuf to force line-buffered output (rsync doesn't output progress when not on TTY)
    const args = [
      'stdbuf', '-oL',
      'rsync', '-av', '--delete', '--info=progress2', '--no-inc-recursive', '--stats',
      `${source}/`, `${destPath}/`
    ];

    const rsync = spawn('sudo', args);
    activeBackupProcess = rsync;

    let stdout = '';
    let stderr = '';

    // Function to parse and emit progress from any output
    const processOutput = (chunk) => {
      // Split by carriage return or newline
      const lines = chunk.split(/[\r\n]+/);
      for (const line of lines) {
        if (line.includes('%')) {
          console.log('[rsync progress line]', JSON.stringify(line));
        }
        const progress = parseRsyncProgress(line);
        if (progress) {
          console.log('[rsync progress parsed]', progress);
          getIO().emit('backup:progress', {
            sourceIndex,
            sourceName,
            sourcesCount,
            percent: progress.percent,
            transferredBytes: progress.transferredBytes,
            speed: progress.speed
          });
        }
      }
    };

    rsync.stdout.on('data', (data) => {
      const chunk = data.toString();
      stdout += chunk;
      processOutput(chunk);
    });

    rsync.stderr.on('data', (data) => {
      const chunk = data.toString();
      stderr += chunk;
      // rsync sometimes outputs progress to stderr
      processOutput(chunk);
    });

    rsync.on('close', (code) => {
      activeBackupProcess = null;
      if (backupCancelled) {
        reject(new Error('Backup cancelled by user'));
      } else if (code === 0) {
        resolve({ stdout, stderr });
      } else {
        reject(new Error(`rsync exited with code ${code}: ${stderr}`));
      }
    });

    rsync.on('error', (err) => {
      activeBackupProcess = null;
      reject(err);
    });
  });
}

export async function runBackup() {
  const startTime = Date.now();
  const timestamp = new Date().toISOString();
  const { SMB_MOUNT_POINT } = getEnv();

  // Reset cancellation state
  backupCancelled = false;

  try {
    const { config } = await getConfig();
    const sources = config.sources;

    if (!sources || sources.length === 0) {
      return { success: false, error: 'No backup sources configured' };
    }

    const validSources = sources.filter(src => existsSync(src));
    if (validSources.length === 0) {
      return { success: false, error: 'No valid backup sources found' };
    }

    // Wake-on-LAN si configuré
    if (config.wolMacAddress) {
      getIO().emit('backup:wol-sent', { macAddress: config.wolMacAddress });
      await sendWakeOnLan(config.wolMacAddress);
      await waitForServer();
    }

    await mountSmb();

    // SAFETY CHECK: Verify SMB is actually mounted before starting backup
    // This prevents writing to local disk if mount failed silently
    if (!await isMounted()) {
      throw new Error('SMB share not mounted - aborting to prevent local disk backup');
    }

    // Emit backup started
    getIO().emit('backup:started', {
      timestamp,
      sourcesCount: validSources.length,
      sources: validSources.map(s => path.basename(s))
    });

    const results = [];
    let totalFiles = 0;
    let totalTransferred = 0;

    for (let i = 0; i < validSources.length; i++) {
      if (backupCancelled) break;

      // Verify mount is still active before each source
      if (!await isMounted()) {
        throw new Error('SMB share disconnected during backup - aborting');
      }

      const source = validSources[i];
      const sourceName = path.basename(source);
      const destPath = path.join(SMB_MOUNT_POINT, sourceName);

      // Emit source start
      getIO().emit('backup:source-start', {
        sourceIndex: i,
        sourceName,
        sourcePath: source,
        sourcesCount: validSources.length
      });

      try {
        const { stdout } = await runRsyncWithProgress(source, destPath, i, sourceName, validSources.length);
        const stats = parseRsyncStats(stdout);

        results.push({
          source,
          success: true,
          filesTransferred: stats.filesTransferred,
          transferredSize: stats.transferredSize
        });

        totalFiles += stats.filesTransferred;
        totalTransferred += stats.transferredSize;

        // Emit source complete
        getIO().emit('backup:source-complete', {
          sourceIndex: i,
          sourceName,
          filesTransferred: stats.filesTransferred,
          transferredSize: stats.transferredSize
        });
      } catch (error) {
        if (backupCancelled) {
          results.push({
            source,
            success: false,
            error: 'Cancelled'
          });
        } else {
          results.push({
            source,
            success: false,
            error: error.message
          });
        }
      }
    }

    await unmountSmb();

    const duration = Date.now() - startTime;
    const allSuccess = results.every(r => r.success);
    const status = backupCancelled ? 'cancelled' : (allSuccess ? 'success' : 'partial');

    await addToHistory({
      timestamp,
      duration,
      status,
      sourcesCount: validSources.length,
      filesTransferred: totalFiles,
      transferredSize: totalTransferred,
      results
    });

    // Emit backup complete
    getIO().emit('backup:complete', {
      success: !backupCancelled && allSuccess,
      cancelled: backupCancelled,
      duration,
      totalFiles,
      totalSize: totalTransferred,
      results
    });

    if (backupCancelled) {
      return { success: false, error: 'Backup cancelled by user' };
    }

    return {
      success: true,
      message: allSuccess ? 'Backup completed successfully' : 'Backup completed with some errors',
      details: {
        duration,
        sourcesBackedUp: validSources.length,
        filesTransferred: totalFiles,
        transferredSize: totalTransferred,
        results
      }
    };
  } catch (error) {
    await unmountSmb();

    await addToHistory({
      timestamp,
      duration: Date.now() - startTime,
      status: 'failed',
      error: error.message
    });

    // Emit error
    getIO().emit('backup:error', { error: error.message });

    return { success: false, error: error.message };
  }
}

export async function cancelBackup() {
  if (!activeBackupProcess) {
    return { success: false, error: 'No backup in progress' };
  }

  backupCancelled = true;

  try {
    // Kill the rsync process (need to kill sudo and its child)
    await execAsync(`sudo pkill -P ${activeBackupProcess.pid}`);
    activeBackupProcess.kill('SIGTERM');
  } catch {
    // Process may have already exited
  }

  getIO().emit('backup:cancelled', { reason: 'user' });

  return { success: true, message: 'Backup cancellation requested' };
}

export function isBackupRunning() {
  return activeBackupProcess !== null;
}

export async function getHistory() {
  try {
    const { BACKUP_HISTORY_FILE } = getEnv();

    if (!existsSync(BACKUP_HISTORY_FILE)) {
      return { success: true, history: [] };
    }

    const content = await readFile(BACKUP_HISTORY_FILE, 'utf-8');
    const history = JSON.parse(content);

    return { success: true, history };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

async function addToHistory(entry) {
  try {
    const { BACKUP_HISTORY_FILE } = getEnv();
    await ensureConfigDir();

    let history = [];
    if (existsSync(BACKUP_HISTORY_FILE)) {
      const content = await readFile(BACKUP_HISTORY_FILE, 'utf-8');
      history = JSON.parse(content);
    }

    history.unshift(entry);
    history = history.slice(0, 50);

    await writeFile(BACKUP_HISTORY_FILE, JSON.stringify(history, null, 2));
  } catch (error) {
    console.error('Failed to save backup history:', error);
  }
}

// Lister le contenu d'un répertoire distant (explorateur de fichiers)
export async function getRemoteBackups(relativePath = '') {
  const { SMB_MOUNT_POINT } = getEnv();

  try {
    await mountSmb();

    // Construire le chemin complet (sécurisé - pas de ..)
    const safePath = relativePath.replace(/\.\./g, '').replace(/^\/+/, '');
    const targetPath = path.join(SMB_MOUNT_POINT, safePath);

    // Vérifier que le chemin est bien sous le point de montage
    if (!targetPath.startsWith(SMB_MOUNT_POINT)) {
      await unmountSmb();
      return { success: false, error: 'Invalid path' };
    }

    // Lister le contenu du répertoire
    const { stdout: lsOutput } = await execAsync(`ls -1 "${targetPath}"`);
    const entries = lsOutput.trim().split('\n').filter(f => f && !f.startsWith('.'));

    const items = [];

    for (const entry of entries) {
      const entryPath = path.join(targetPath, entry);

      try {
        // Obtenir le type (fichier ou dossier)
        const { stdout: statType } = await execAsync(`stat -c %F "${entryPath}"`);
        const isDirectory = statType.trim().includes('directory');

        // Obtenir la taille
        let size = 0;
        if (isDirectory) {
          // Pour les dossiers, obtenir la taille totale (peut être lent)
          try {
            const { stdout: duOutput } = await execAsync(`du -sb "${entryPath}" | cut -f1`, { timeout: 30000 });
            size = parseInt(duOutput.trim()) || 0;
          } catch {
            size = 0;
          }
        } else {
          const { stdout: sizeOutput } = await execAsync(`stat -c %s "${entryPath}"`);
          size = parseInt(sizeOutput.trim()) || 0;
        }

        // Obtenir la date de dernière modification
        const { stdout: statOutput } = await execAsync(`stat -c %Y "${entryPath}"`);
        const lastModified = new Date(parseInt(statOutput.trim()) * 1000).toISOString();

        items.push({
          name: entry,
          type: isDirectory ? 'directory' : 'file',
          size,
          lastModified
        });
      } catch (err) {
        console.error(`Error getting info for ${entry}:`, err.message);
      }
    }

    await unmountSmb();

    // Trier: dossiers d'abord, puis par nom
    items.sort((a, b) => {
      if (a.type !== b.type) return a.type === 'directory' ? -1 : 1;
      return a.name.localeCompare(b.name);
    });

    return { success: true, items, currentPath: safePath };
  } catch (error) {
    await unmountSmb();
    return { success: false, error: error.message };
  }
}

// Supprimer un élément distant (fichier ou dossier)
export async function deleteRemoteItem(relativePath) {
  const { SMB_MOUNT_POINT } = getEnv();

  if (!relativePath || relativePath === '/' || relativePath === '') {
    return { success: false, error: 'Cannot delete root directory' };
  }

  try {
    await mountSmb();

    // Construire le chemin complet (sécurisé)
    const safePath = relativePath.replace(/\.\./g, '').replace(/^\/+/, '');
    const targetPath = path.join(SMB_MOUNT_POINT, safePath);

    // Vérifier que le chemin est bien sous le point de montage
    if (!targetPath.startsWith(SMB_MOUNT_POINT) || targetPath === SMB_MOUNT_POINT) {
      await unmountSmb();
      return { success: false, error: 'Invalid path' };
    }

    // Vérifier que l'élément existe
    if (!existsSync(targetPath)) {
      await unmountSmb();
      return { success: false, error: 'Path does not exist' };
    }

    // Supprimer (rm -rf pour les dossiers)
    await execAsync(`rm -rf "${targetPath}"`);

    await unmountSmb();
    return { success: true, message: `Deleted: ${safePath}` };
  } catch (error) {
    await unmountSmb();
    return { success: false, error: error.message };
  }
}

// Arrêter le serveur distant via SSH
export async function shutdownServer() {
  const { SMB_SERVER } = getEnv();
  const sshUser = process.env.SSH_USER || 'root';

  try {
    await execAsync(
      `ssh -o ConnectTimeout=5 -o StrictHostKeyChecking=no -o BatchMode=yes ${sshUser}@${SMB_SERVER} 'sudo -n /sbin/shutdown -h now'`,
      { timeout: 10000 }
    );
    return { success: true, message: 'Shutdown command sent' };
  } catch (error) {
    // Le SSH peut se terminer abruptement quand le serveur s'éteint
    // On considère ça comme un succès si l'erreur est liée à la connexion fermée
    if (error.killed || error.code === 255 || error.message.includes('closed')) {
      return { success: true, message: 'Shutdown initiated' };
    }
    return { success: false, error: error.message };
  }
}
