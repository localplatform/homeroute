import { exec, spawn } from 'child_process';
import { promisify } from 'util';
import { existsSync } from 'fs';
import { getIO } from '../socket.js';

const execAsync = promisify(exec);

// State management
let activeCheckProcess = null;
let checkCancelled = false;
let lastCheckResult = null;

// Check if a check is running
export function isCheckRunning() {
  return activeCheckProcess !== null;
}

// Get last check result
export function getLastCheckResult() {
  return lastCheckResult;
}

// Cancel running check
export async function cancelCheck() {
  if (!activeCheckProcess) {
    return { success: false, error: 'No check in progress' };
  }

  checkCancelled = true;

  try {
    activeCheckProcess.kill('SIGTERM');
  } catch {
    // Process may have already exited
  }

  getIO().emit('updates:cancelled', {});

  return { success: true, message: 'Check cancellation requested' };
}

// Parse apt list --upgradable output
function parseAptList(output) {
  const lines = output.trim().split('\n').filter(l => l.includes('[upgradable'));
  return lines.map(line => {
    // Format: package/repo version arch [upgradable from: oldversion]
    const match = line.match(/^([^/]+)\/([^\s]+)\s+([^\s]+)\s+([^\s]+)\s+\[upgradable from: ([^\]]+)\]/);
    if (!match) return null;
    const [, name, repo, newVersion, arch, currentVersion] = match;
    return {
      name,
      repository: repo,
      isSecurity: repo.toLowerCase().includes('security'),
      currentVersion,
      newVersion,
      arch
    };
  }).filter(Boolean);
}

// Parse snap refresh --list output
function parseSnapList(output) {
  const lines = output.trim().split('\n');
  if (lines.length <= 1) return []; // Only header or empty

  // Skip header line
  return lines.slice(1).map(line => {
    const parts = line.trim().split(/\s+/);
    if (parts.length < 4) return null;
    return {
      name: parts[0],
      newVersion: parts[1],
      revision: parts[2],
      publisher: parts[3]
    };
  }).filter(Boolean);
}

// Parse needrestart -b output
function parseNeedrestart(output) {
  const result = {
    kernelRebootNeeded: false,
    currentKernel: null,
    expectedKernel: null,
    services: []
  };

  const lines = output.split('\n');
  for (const line of lines) {
    if (line.startsWith('NEEDRESTART-KSTA:')) {
      const status = parseInt(line.split(':')[1].trim());
      result.kernelRebootNeeded = status > 1; // 1=current, 2=ABI compatible, 3=different
    } else if (line.startsWith('NEEDRESTART-KCUR:')) {
      result.currentKernel = line.split(':')[1].trim();
    } else if (line.startsWith('NEEDRESTART-KEXP:')) {
      result.expectedKernel = line.split(':')[1].trim();
    } else if (line.startsWith('NEEDRESTART-SVC:')) {
      result.services.push(line.split(':')[1].trim());
    }
  }

  return result;
}

// Run apt update with streaming output
function runAptUpdate() {
  return new Promise((resolve, reject) => {
    const io = getIO();
    const apt = spawn('sudo', ['apt', 'update']);
    activeCheckProcess = apt;

    let stdout = '';
    let stderr = '';

    apt.stdout.on('data', (data) => {
      const chunk = data.toString();
      stdout += chunk;
      // Stream each line
      const lines = chunk.split('\n').filter(Boolean);
      for (const line of lines) {
        io.emit('updates:output', { phase: 'apt-update', line });
      }
    });

    apt.stderr.on('data', (data) => {
      stderr += data.toString();
    });

    apt.on('close', (code) => {
      activeCheckProcess = null;
      if (checkCancelled) {
        reject(new Error('Check cancelled'));
      } else if (code === 0) {
        resolve({ stdout, stderr });
      } else {
        reject(new Error(`apt update failed with code ${code}: ${stderr}`));
      }
    });

    apt.on('error', (err) => {
      activeCheckProcess = null;
      reject(err);
    });
  });
}

// Main check function
export async function runFullCheck() {
  if (activeCheckProcess) {
    return { success: false, error: 'A check is already in progress' };
  }

  const io = getIO();
  const startTime = Date.now();
  checkCancelled = false;

  const result = {
    timestamp: new Date().toISOString(),
    duration: 0,
    apt: { packages: [], securityCount: 0, normalCount: 0 },
    snap: { packages: [] },
    needrestart: { services: [], kernelRebootNeeded: false },
    summary: { totalUpdates: 0, securityUpdates: 0, servicesNeedingRestart: 0 }
  };

  try {
    io.emit('updates:started', { timestamp: result.timestamp });

    // Phase 1: APT update
    io.emit('updates:phase', { phase: 'apt-update', message: 'Mise a jour des listes de paquets...' });
    try {
      await runAptUpdate();
    } catch (err) {
      if (checkCancelled) throw err;
      console.error('apt update error:', err.message);
    }

    if (checkCancelled) throw new Error('Check cancelled');

    // Phase 2: APT list upgradable
    io.emit('updates:phase', { phase: 'apt-list', message: 'Verification des paquets...' });
    try {
      const { stdout } = await execAsync('apt list --upgradable 2>/dev/null', { timeout: 60000 });
      result.apt.packages = parseAptList(stdout);
      result.apt.securityCount = result.apt.packages.filter(p => p.isSecurity).length;
      result.apt.normalCount = result.apt.packages.length - result.apt.securityCount;
      io.emit('updates:apt-complete', {
        packages: result.apt.packages,
        securityCount: result.apt.securityCount,
        normalCount: result.apt.normalCount
      });
    } catch (err) {
      console.error('apt list error:', err.message);
    }

    if (checkCancelled) throw new Error('Check cancelled');

    // Phase 3: Snap refresh list
    io.emit('updates:phase', { phase: 'snap', message: 'Verification des snaps...' });
    try {
      const { stdout } = await execAsync('snap refresh --list 2>/dev/null', { timeout: 60000 });
      result.snap.packages = parseSnapList(stdout);
      io.emit('updates:snap-complete', { snaps: result.snap.packages });
    } catch (err) {
      // snap refresh --list returns non-zero if no updates
      if (!err.message.includes('All snaps up to date')) {
        console.error('snap refresh error:', err.message);
      }
    }

    if (checkCancelled) throw new Error('Check cancelled');

    // Phase 4: Needrestart
    io.emit('updates:phase', { phase: 'needrestart', message: 'Detection des services a redemarrer...' });
    try {
      // Check if needrestart is available
      const { stdout: whichOutput } = await execAsync('which needrestart 2>/dev/null');
      if (whichOutput.trim()) {
        const { stdout } = await execAsync('sudo needrestart -b 2>/dev/null', { timeout: 60000 });
        result.needrestart = parseNeedrestart(stdout);
      } else {
        // Fallback: check reboot-required file
        if (existsSync('/var/run/reboot-required')) {
          result.needrestart.kernelRebootNeeded = true;
        }
        // Try to get services from /var/run/reboot-required.pkgs
        if (existsSync('/var/run/reboot-required.pkgs')) {
          try {
            const { stdout } = await execAsync('cat /var/run/reboot-required.pkgs');
            result.needrestart.services = stdout.trim().split('\n').filter(Boolean);
          } catch {}
        }
      }
      io.emit('updates:needrestart-complete', result.needrestart);
    } catch (err) {
      console.error('needrestart error:', err.message);
    }

    // Calculate summary
    result.summary = {
      totalUpdates: result.apt.packages.length + result.snap.packages.length,
      securityUpdates: result.apt.securityCount,
      servicesNeedingRestart: result.needrestart.services.length,
      kernelRebootNeeded: result.needrestart.kernelRebootNeeded
    };

    result.duration = Date.now() - startTime;
    lastCheckResult = result;

    io.emit('updates:complete', {
      success: true,
      summary: result.summary,
      duration: result.duration
    });

    return { success: true, result };

  } catch (err) {
    const duration = Date.now() - startTime;

    if (checkCancelled) {
      io.emit('updates:cancelled', {});
      return { success: false, error: 'Check cancelled', cancelled: true };
    }

    io.emit('updates:error', { error: err.message });
    return { success: false, error: err.message };
  } finally {
    activeCheckProcess = null;
  }
}

// State for upgrade operations
let activeUpgradeProcess = null;
let upgradeCancelled = false;

export function isUpgradeRunning() {
  return activeUpgradeProcess !== null;
}

export async function cancelUpgrade() {
  if (!activeUpgradeProcess) {
    return { success: false, error: 'No upgrade in progress' };
  }

  upgradeCancelled = true;

  try {
    activeUpgradeProcess.kill('SIGTERM');
  } catch {
    // Process may have already exited
  }

  getIO().emit('updates:upgrade-cancelled', {});

  return { success: true, message: 'Upgrade cancellation requested' };
}

// Run apt upgrade with streaming output
export async function runAptUpgrade(fullUpgrade = false) {
  if (activeUpgradeProcess) {
    return { success: false, error: 'An upgrade is already in progress' };
  }

  const io = getIO();
  const startTime = Date.now();
  upgradeCancelled = false;

  const command = fullUpgrade ? 'full-upgrade' : 'upgrade';

  return new Promise((resolve) => {
    io.emit('updates:upgrade-started', { type: 'apt', command });

    const apt = spawn('sudo', ['apt', command, '-y'], {
      env: { ...process.env, DEBIAN_FRONTEND: 'noninteractive' }
    });
    activeUpgradeProcess = apt;

    let stdout = '';
    let stderr = '';

    apt.stdout.on('data', (data) => {
      const chunk = data.toString();
      stdout += chunk;
      const lines = chunk.split('\n').filter(Boolean);
      for (const line of lines) {
        io.emit('updates:upgrade-output', { type: 'apt', line });
      }
    });

    apt.stderr.on('data', (data) => {
      const chunk = data.toString();
      stderr += chunk;
      const lines = chunk.split('\n').filter(Boolean);
      for (const line of lines) {
        io.emit('updates:upgrade-output', { type: 'apt', line });
      }
    });

    apt.on('close', (code) => {
      activeUpgradeProcess = null;
      const duration = Date.now() - startTime;

      if (upgradeCancelled) {
        io.emit('updates:upgrade-cancelled', { type: 'apt' });
        resolve({ success: false, error: 'Upgrade cancelled', cancelled: true });
      } else if (code === 0) {
        io.emit('updates:upgrade-complete', { type: 'apt', success: true, duration });
        resolve({ success: true, duration });
      } else {
        io.emit('updates:upgrade-complete', { type: 'apt', success: false, error: stderr });
        resolve({ success: false, error: `apt ${command} failed with code ${code}` });
      }
    });

    apt.on('error', (err) => {
      activeUpgradeProcess = null;
      io.emit('updates:upgrade-complete', { type: 'apt', success: false, error: err.message });
      resolve({ success: false, error: err.message });
    });
  });
}

// Run snap refresh with streaming output
export async function runSnapRefresh() {
  if (activeUpgradeProcess) {
    return { success: false, error: 'An upgrade is already in progress' };
  }

  const io = getIO();
  const startTime = Date.now();
  upgradeCancelled = false;

  return new Promise((resolve) => {
    io.emit('updates:upgrade-started', { type: 'snap' });

    const snap = spawn('sudo', ['snap', 'refresh']);
    activeUpgradeProcess = snap;

    let stdout = '';
    let stderr = '';

    snap.stdout.on('data', (data) => {
      const chunk = data.toString();
      stdout += chunk;
      const lines = chunk.split('\n').filter(Boolean);
      for (const line of lines) {
        io.emit('updates:upgrade-output', { type: 'snap', line });
      }
    });

    snap.stderr.on('data', (data) => {
      const chunk = data.toString();
      stderr += chunk;
      const lines = chunk.split('\n').filter(Boolean);
      for (const line of lines) {
        io.emit('updates:upgrade-output', { type: 'snap', line });
      }
    });

    snap.on('close', (code) => {
      activeUpgradeProcess = null;
      const duration = Date.now() - startTime;

      if (upgradeCancelled) {
        io.emit('updates:upgrade-cancelled', { type: 'snap' });
        resolve({ success: false, error: 'Upgrade cancelled', cancelled: true });
      } else if (code === 0) {
        io.emit('updates:upgrade-complete', { type: 'snap', success: true, duration });
        resolve({ success: true, duration });
      } else {
        // snap refresh returns non-zero if no updates, check output
        if (stdout.includes('All snaps up to date') || stderr.includes('All snaps up to date')) {
          io.emit('updates:upgrade-complete', { type: 'snap', success: true, duration, message: 'All snaps up to date' });
          resolve({ success: true, duration, message: 'All snaps up to date' });
        } else {
          io.emit('updates:upgrade-complete', { type: 'snap', success: false, error: stderr });
          resolve({ success: false, error: `snap refresh failed with code ${code}` });
        }
      }
    });

    snap.on('error', (err) => {
      activeUpgradeProcess = null;
      io.emit('updates:upgrade-complete', { type: 'snap', success: false, error: err.message });
      resolve({ success: false, error: err.message });
    });
  });
}
