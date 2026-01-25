import { readFile, writeFile, mkdir, readdir } from 'fs/promises';
import { existsSync } from 'fs';
import { exec, spawn } from 'child_process';
import { promisify } from 'util';
import { EventEmitter } from 'events';
import path from 'path';
import os from 'os';

// Event emitter for real-time updates
export const energyEvents = new EventEmitter();

const execAsync = promisify(exec);

// Config paths
const CONFIG_DIR = process.env.ENERGY_CONFIG_DIR || '/var/lib/server-dashboard';
const SCHEDULE_CONFIG_FILE = path.join(CONFIG_DIR, 'energy-schedule.json');
const AUTOSELECT_CONFIG_FILE = path.join(CONFIG_DIR, 'energy-autoselect.json');

// Unified energy modes
export const ENERGY_MODES = {
  economy: {
    governor: 'powersave',
    epp: 'power',
    maxFreqPercent: 60,
    label: 'Économie',
    icon: 'Moon'
  },
  auto: {
    governor: 'powersave',
    epp: 'balance_power',
    maxFreqPercent: 85,
    label: 'Auto',
    icon: 'Zap'
  },
  performance: {
    governor: 'performance',
    epp: 'performance',
    maxFreqPercent: 100,
    label: 'Performance',
    icon: 'Rocket'
  }
};

// CPU sysfs paths
const CPU_FREQ_PATH = '/sys/devices/system/cpu/cpu0/cpufreq';
const HWMON_PATH = '/sys/class/hwmon';

// Cache for hwmon paths (they can change between boots)
let cpuSensorPath = null;
let cpuSensorName = null;

// Previous CPU stats for usage calculation
let prevCpuStats = null;

async function ensureConfigDir() {
  if (!existsSync(CONFIG_DIR)) {
    await mkdir(CONFIG_DIR, { recursive: true });
  }
}

// Find hwmon device by name
async function findHwmonByName(name) {
  try {
    const hwmons = await readdir(HWMON_PATH);
    for (const hwmon of hwmons) {
      const namePath = path.join(HWMON_PATH, hwmon, 'name');
      if (existsSync(namePath)) {
        const hwmonName = (await readFile(namePath, 'utf-8')).trim();
        if (hwmonName === name) {
          return path.join(HWMON_PATH, hwmon);
        }
      }
    }
  } catch {
    // Ignore errors
  }
  return null;
}

// Find first valid CPU temperature sensor (supports AMD and Intel)
async function findFirstValidCpuSensor() {
  // Try cached path first
  if (cpuSensorPath && cpuSensorName) {
    return { path: cpuSensorPath, name: cpuSensorName };
  }

  // List of CPU temperature sensors to try (in order of preference)
  const sensorNames = ['k10temp', 'coretemp', 'zenpower'];

  for (const name of sensorNames) {
    const hwmonPath = await findHwmonByName(name);
    if (hwmonPath) {
      // Verify temp1_input exists and is readable
      const tempPath = path.join(hwmonPath, 'temp1_input');
      const temp = await readSysfs(tempPath);
      if (temp !== null) {
        // Cache the result
        cpuSensorPath = hwmonPath;
        cpuSensorName = name;
        console.log(`CPU temperature sensor detected: ${name} at ${hwmonPath}`);
        return { path: hwmonPath, name };
      }
    }
  }

  // No sensor found - log available hwmon devices for debugging
  console.error('No CPU temperature sensor found. Available hwmon devices:');
  try {
    const hwmons = await readdir(HWMON_PATH);
    for (const hwmon of hwmons) {
      const namePath = path.join(HWMON_PATH, hwmon, 'name');
      if (existsSync(namePath)) {
        const hwmonName = (await readFile(namePath, 'utf-8')).trim();
        console.error(`  - ${hwmon}: ${hwmonName}`);
      }
    }
  } catch (error) {
    console.error('  Error reading hwmon devices:', error.message);
  }

  return null;
}

// Read a sysfs file safely
async function readSysfs(filePath) {
  try {
    const content = await readFile(filePath, 'utf-8');
    return content.trim();
  } catch {
    return null;
  }
}

// Write to a sysfs file
async function writeSysfs(filePath, value) {
  try {
    await writeFile(filePath, String(value));
    return true;
  } catch {
    return false;
  }
}

// ============ CPU INFO ============

export async function getCpuTemperature() {
  try {
    const sensor = await findFirstValidCpuSensor();
    if (!sensor) {
      return { success: false, error: 'No CPU temperature sensor found (tried: k10temp, coretemp, zenpower)' };
    }

    const tempPath = path.join(sensor.path, 'temp1_input');
    const tempRaw = await readSysfs(tempPath);
    if (!tempRaw) {
      console.error(`Failed to read temperature from ${tempPath}`);
      return { success: false, error: `Cannot read temperature from ${sensor.name}` };
    }

    const tempC = parseInt(tempRaw) / 1000;
    return { success: true, temperature: tempC, sensor: sensor.name };
  } catch (error) {
    console.error('Error reading CPU temperature:', error);
    return { success: false, error: error.message };
  }
}

export async function getCpuFrequency() {
  try {
    const currentRaw = await readSysfs(path.join(CPU_FREQ_PATH, 'scaling_cur_freq'));
    const minRaw = await readSysfs(path.join(CPU_FREQ_PATH, 'scaling_min_freq'));
    const maxRaw = await readSysfs(path.join(CPU_FREQ_PATH, 'scaling_max_freq'));

    return {
      success: true,
      current: currentRaw ? parseInt(currentRaw) / 1000000 : null, // GHz
      min: minRaw ? parseInt(minRaw) / 1000000 : null,
      max: maxRaw ? parseInt(maxRaw) / 1000000 : null
    };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getCpuUsage() {
  try {
    const statContent = await readFile('/proc/stat', 'utf-8');
    const cpuLine = statContent.split('\n').find(line => line.startsWith('cpu '));
    if (!cpuLine) {
      return { success: false, error: 'Cannot read CPU stats' };
    }

    const parts = cpuLine.split(/\s+/).slice(1).map(Number);
    const [user, nice, system, idle, iowait, irq, softirq, steal] = parts;

    const total = user + nice + system + idle + iowait + irq + softirq + steal;
    const idleTime = idle + iowait;

    if (!prevCpuStats) {
      prevCpuStats = { total, idle: idleTime };
      return { success: true, usage: 0 };
    }

    const totalDiff = total - prevCpuStats.total;
    const idleDiff = idleTime - prevCpuStats.idle;

    prevCpuStats = { total, idle: idleTime };

    const usage = totalDiff > 0 ? ((totalDiff - idleDiff) / totalDiff) * 100 : 0;
    return { success: true, usage: Math.round(usage * 10) / 10 };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getCpuModel() {
  try {
    const cpuinfo = await readFile('/proc/cpuinfo', 'utf-8');
    const match = cpuinfo.match(/model name\s*:\s*(.+)/);
    if (match) {
      // Clean up CPU model name (remove trademark symbols and extra spaces)
      const model = match[1]
        .trim()
        .replace(/\(R\)/g, '')
        .replace(/\(TM\)/g, '')
        .replace(/\s+/g, ' ')
        .trim();
      return { success: true, model };
    }
    return { success: false, error: 'CPU model not found in /proc/cpuinfo' };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getCpuInfo() {
  const [tempResult, freqResult, usageResult, modelResult] = await Promise.all([
    getCpuTemperature(),
    getCpuFrequency(),
    getCpuUsage(),
    getCpuModel()
  ]);

  return {
    success: true,
    temperature: tempResult.success ? tempResult.temperature : null,
    frequency: freqResult.success ? freqResult : null,
    usage: usageResult.success ? usageResult.usage : null,
    model: modelResult.success ? modelResult.model : 'CPU'
  };
}

// ============ GOVERNOR ============

export async function getCurrentGovernor() {
  try {
    const governor = await readSysfs(path.join(CPU_FREQ_PATH, 'scaling_governor'));
    return { success: true, governor };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getAvailableGovernors() {
  try {
    const governors = await readSysfs(path.join(CPU_FREQ_PATH, 'scaling_available_governors'));
    return { success: true, governors: governors ? governors.split(' ') : [] };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function setGovernor(governor) {
  try {
    // Validate governor
    const available = await getAvailableGovernors();
    if (!available.success || !available.governors.includes(governor)) {
      return { success: false, error: `Invalid governor: ${governor}` };
    }

    // Set governor on all CPUs
    const { stdout } = await execAsync('ls /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor');
    const files = stdout.trim().split('\n');

    for (const file of files) {
      await writeSysfs(file, governor);
    }

    return { success: true, message: `Governor set to ${governor}` };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ============ EPP (Energy Performance Preference) Functions ============

// Get hardware max frequency
async function getHwMaxFreq() {
  try {
    const freq = await readSysfs('/sys/devices/system/cpu/cpu0/cpufreq/cpuinfo_max_freq');
    return freq ? parseInt(freq) : null;
  } catch (error) {
    return null;
  }
}

// Check if EPP is available (Intel P-State)
async function isEppAvailable() {
  return existsSync('/sys/devices/system/cpu/cpu0/cpufreq/energy_performance_preference');
}

// Set Energy Performance Preference
async function setEpp(preference) {
  if (!await isEppAvailable()) {
    return { success: false, error: 'EPP not supported on this CPU' };
  }

  try {
    const { stdout } = await execAsync('ls /sys/devices/system/cpu/cpu*/cpufreq/energy_performance_preference');
    const files = stdout.trim().split('\n');

    for (const file of files) {
      await writeSysfs(file, preference);
    }

    return { success: true, message: `EPP set to ${preference}` };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// Set CPU frequency limits
async function setFrequencyLimits(maxFreqPercent) {
  try {
    const hwMaxFreq = await getHwMaxFreq();
    if (!hwMaxFreq) {
      return { success: false, error: 'Cannot determine hardware max frequency' };
    }

    const targetMaxFreq = Math.round(hwMaxFreq * (maxFreqPercent / 100));

    const { stdout } = await execAsync('ls /sys/devices/system/cpu/cpu*/cpufreq/scaling_max_freq');
    const files = stdout.trim().split('\n');

    for (const file of files) {
      await writeSysfs(file, targetMaxFreq.toString());
    }

    return { success: true, message: `Max frequency set to ${maxFreqPercent}% (${targetMaxFreq} kHz)` };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// Reset frequency limits to hardware defaults
async function resetFrequencyLimits() {
  try {
    const hwMaxFreq = await getHwMaxFreq();
    if (!hwMaxFreq) {
      return { success: false, error: 'Cannot determine hardware max frequency' };
    }

    const { stdout } = await execAsync('ls /sys/devices/system/cpu/cpu*/cpufreq/scaling_max_freq');
    const files = stdout.trim().split('\n');

    for (const file of files) {
      await writeSysfs(file, hwMaxFreq.toString());
    }

    return { success: true };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getGovernorStatus() {
  const [current, available] = await Promise.all([
    getCurrentGovernor(),
    getAvailableGovernors()
  ]);

  return {
    success: true,
    current: current.success ? current.governor : null,
    available: available.success ? available.governors : []
  };
}

// Get current energy mode based on governor
export async function getCurrentMode() {
  try {
    const { governor } = await getCurrentGovernor();

    // Find which mode matches the current governor
    for (const [modeName, modeConfig] of Object.entries(ENERGY_MODES)) {
      if (modeConfig.governor === governor) {
        return { success: true, mode: modeName, config: modeConfig };
      }
    }

    // Default to auto if governor doesn't match any mode
    return { success: true, mode: 'auto', config: ENERGY_MODES.auto };
  } catch (error) {
    return { success: false, error: error.message, mode: 'auto', config: ENERGY_MODES.auto };
  }
}

// Get all available energy modes
export function getEnergyModes() {
  return { success: true, modes: ENERGY_MODES };
}

// ============ SCHEDULE ============

const DEFAULT_SCHEDULE = {
  enabled: false,
  nightStart: '22:00',
  nightEnd: '08:00',
  dayMode: 'auto',      // economy | auto | performance
  nightMode: 'economy'
};

export async function getScheduleConfig() {
  try {
    await ensureConfigDir();

    if (!existsSync(SCHEDULE_CONFIG_FILE)) {
      return { success: true, config: DEFAULT_SCHEDULE };
    }

    const content = await readFile(SCHEDULE_CONFIG_FILE, 'utf-8');
    const config = JSON.parse(content);
    return { success: true, config: { ...DEFAULT_SCHEDULE, ...config } };
  } catch (error) {
    return { success: false, error: error.message, config: DEFAULT_SCHEDULE };
  }
}

export async function saveScheduleConfig(config) {
  try {
    await ensureConfigDir();

    const newConfig = { ...DEFAULT_SCHEDULE, ...config };

    // Schedule and auto-select can work together
    // Night mode forces economy, overriding auto-select

    await writeFile(SCHEDULE_CONFIG_FILE, JSON.stringify(newConfig, null, 2));

    // Sync cron jobs
    await syncCronJobs(newConfig);

    return { success: true, message: 'Schedule saved' };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ============ CRON MANAGEMENT ============

const CRON_MARKER = '# server-dashboard-energy';

export async function syncCronJobs(config) {
  try {
    // Read current crontab
    let crontab = '';
    try {
      const { stdout } = await execAsync('crontab -l 2>/dev/null');
      crontab = stdout;
    } catch {
      // No crontab exists
    }

    // Remove existing energy cron jobs
    const lines = crontab.split('\n').filter(line => !line.includes(CRON_MARKER));

    if (config.enabled) {
      // Parse times
      const [nightHour, nightMin] = config.nightStart.split(':').map(Number);
      const [dayHour, dayMin] = config.nightEnd.split(':').map(Number);

      // Add new cron jobs
      const apiUrl = process.env.API_URL || 'http://localhost:4000';

      lines.push(`${nightMin} ${nightHour} * * * curl -X POST ${apiUrl}/api/energy/mode/night ${CRON_MARKER}`);
      lines.push(`${dayMin} ${dayHour} * * * curl -X POST ${apiUrl}/api/energy/mode/day ${CRON_MARKER}`);
    }

    // Write new crontab
    const newCrontab = lines.filter(l => l.trim()).join('\n') + '\n';
    await execAsync(`echo '${newCrontab}' | crontab -`);

    return { success: true };
  } catch (error) {
    console.error('Failed to sync cron jobs:', error);
    return { success: false, error: error.message };
  }
}

// Apply energy mode (economy/auto/performance) or scheduled mode (day/night)
export async function applyMode(mode) {
  try {
    // Handle scheduled modes (day/night)
    if (mode === 'day' || mode === 'night') {
      const { config } = await getScheduleConfig();
      const targetMode = mode === 'night' ? config.nightMode : config.dayMode;
      return applyMode(targetMode);
    }

    // Handle unified energy modes
    if (!ENERGY_MODES[mode]) {
      return { success: false, error: `Invalid mode: ${mode}. Valid modes: economy, auto, performance` };
    }

    const modeConfig = ENERGY_MODES[mode];

    // Apply governor
    await setGovernor(modeConfig.governor);

    // Apply EPP if available
    if (modeConfig.epp && await isEppAvailable()) {
      const eppResult = await setEpp(modeConfig.epp);
      if (!eppResult.success) {
        console.warn('EPP not applied:', eppResult.error);
      }
    }

    // Apply frequency limits
    if (modeConfig.maxFreqPercent) {
      const freqResult = await setFrequencyLimits(modeConfig.maxFreqPercent);
      if (!freqResult.success) {
        console.warn('Frequency limits not applied:', freqResult.error);
      }
    }

    // Emit event for real-time updates
    energyEvents.emit('modeChange', { mode, config: modeConfig });

    return { success: true, message: `Mode ${modeConfig.label} appliqué`, mode, config: modeConfig };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ============ AUTO-SELECT (based on network RPS) ============

const DEFAULT_AUTOSELECT = {
  enabled: false,
  networkInterface: null,  // Auto-detected via IP 10.0.0.10
  thresholds: {
    low: 500,     // Below this -> economy (req/s)
    high: 15000   // Above this -> performance (req/s)
  },
  averagingTime: 3,      // Averaging window in seconds (replaces hysteresis)
  sampleInterval: 500    // Sampling interval (ms) - 500ms for smooth updates
};

// Network stats tracking
let prevNetworkStats = null;
let prevNetworkStatsTime = null;
let currentRps = 0;
let rpsHistory = [];  // Array of { timestamp, rps } for averaging
let averagedRps = 0;
let autoSelectInterval = null;
let currentAutoMode = null;  // Track current mode for progressive transitions

// Find network interface by IP address
async function findInterfaceByIp(targetIp) {
  try {
    const { stdout } = await execAsync('ip -j addr show');
    const interfaces = JSON.parse(stdout);

    for (const iface of interfaces) {
      if (iface.addr_info) {
        for (const addr of iface.addr_info) {
          if (addr.local === targetIp) {
            return iface.ifname;
          }
        }
      }
    }
    return null;
  } catch (error) {
    console.error('Error finding interface by IP:', error);
    return null;
  }
}

// Get selectable network interfaces (physical interfaces with IPv4)
export async function getSelectableInterfaces() {
  try {
    const { stdout } = await execAsync('ip -j addr show');
    const interfaces = JSON.parse(stdout);

    const selectable = interfaces
      .filter(iface => {
        const name = iface.ifname;
        // Exclude virtual interfaces
        if (name === 'lo' ||
            name.startsWith('veth') ||
            name.startsWith('docker') ||
            name.startsWith('br-') ||
            name.startsWith('virbr')) {
          return false;
        }
        // Must have at least one IPv4 address
        return iface.addr_info?.some(addr => addr.family === 'inet');
      })
      .map(iface => {
        const ipv4Addresses = iface.addr_info
          .filter(addr => addr.family === 'inet')
          .map(addr => addr.local);

        return {
          name: iface.ifname,
          primaryIp: ipv4Addresses[0] || null,
          state: iface.operstate || 'UNKNOWN'
        };
      });

    return { success: true, interfaces: selectable };
  } catch (error) {
    return { success: false, error: error.message, interfaces: [] };
  }
}

// Read network packets from /proc/net/dev
async function getNetworkPackets(interfaceName) {
  try {
    const content = await readFile('/proc/net/dev', 'utf-8');
    const lines = content.split('\n');

    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed.startsWith(interfaceName + ':')) {
        // Format: iface: rx_bytes rx_packets rx_errs ... tx_bytes tx_packets tx_errs ...
        const parts = trimmed.split(/\s+/);
        const rxPackets = parseInt(parts[2]) || 0;
        const txPackets = parseInt(parts[10]) || 0;
        return { success: true, rxPackets, txPackets, total: rxPackets + txPackets };
      }
    }

    return { success: false, error: `Interface ${interfaceName} not found` };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// Calculate averaged RPS over the configured time window
function calculateAveragedRps(averagingTime) {
  const now = Date.now();
  const windowStart = now - (averagingTime * 1000);

  // Remove old samples outside the window
  rpsHistory = rpsHistory.filter(s => s.timestamp >= windowStart);

  if (rpsHistory.length === 0) {
    return 0;
  }

  // Calculate average
  const sum = rpsHistory.reduce((acc, s) => acc + s.rps, 0);
  return Math.round(sum / rpsHistory.length);
}

// Get current RPS (requests per second) for the configured network interface
export async function getNetworkRps() {
  try {
    const { config } = await getAutoSelectConfig();

    // Use configured interface
    const interfaceName = config.networkInterface;
    if (!interfaceName) {
      return {
        success: false,
        error: 'Aucune interface configurée',
        rps: 0,
        averagedRps: 0,
        interfaceError: 'not_configured'
      };
    }

    const stats = await getNetworkPackets(interfaceName);
    if (!stats.success) {
      return {
        success: false,
        error: `Interface ${interfaceName} introuvable`,
        rps: 0,
        averagedRps: 0,
        interfaceError: 'not_found'
      };
    }

    const now = Date.now();

    if (!prevNetworkStats || !prevNetworkStatsTime) {
      prevNetworkStats = stats.total;
      prevNetworkStatsTime = now;
      return { success: true, rps: 0, averagedRps: 0, interface: interfaceName };
    }

    const timeDiff = (now - prevNetworkStatsTime) / 1000; // seconds
    if (timeDiff > 0) {
      currentRps = Math.round((stats.total - prevNetworkStats) / timeDiff);

      // Add to history for averaging
      rpsHistory.push({ timestamp: now, rps: currentRps });
    }

    prevNetworkStats = stats.total;
    prevNetworkStatsTime = now;

    // Calculate averaged RPS
    averagedRps = calculateAveragedRps(config.averagingTime || 3);

    return { success: true, rps: currentRps, averagedRps, interface: interfaceName, appliedMode: currentAutoMode };
  } catch (error) {
    return { success: false, error: error.message, rps: 0, averagedRps: 0, appliedMode: null };
  }
}

// Get auto-select configuration
export async function getAutoSelectConfig() {
  try {
    await ensureConfigDir();

    if (!existsSync(AUTOSELECT_CONFIG_FILE)) {
      return { success: true, config: DEFAULT_AUTOSELECT };
    }

    const content = await readFile(AUTOSELECT_CONFIG_FILE, 'utf-8');
    const config = JSON.parse(content);
    return { success: true, config: { ...DEFAULT_AUTOSELECT, ...config } };
  } catch (error) {
    return { success: false, error: error.message, config: DEFAULT_AUTOSELECT };
  }
}

// Save auto-select configuration
export async function saveAutoSelectConfig(config) {
  try {
    await ensureConfigDir();

    const newConfig = { ...DEFAULT_AUTOSELECT, ...config };

    // Validate interface if enabling auto-select
    if (newConfig.enabled && !newConfig.networkInterface) {
      return {
        success: false,
        error: 'Une interface doit être sélectionnée pour activer l\'auto-select'
      };
    }

    // Validate interface exists if specified
    if (newConfig.networkInterface) {
      const stats = await getNetworkPackets(newConfig.networkInterface);
      if (!stats.success) {
        return {
          success: false,
          error: `Interface ${newConfig.networkInterface} introuvable`
        };
      }
    }

    await writeFile(AUTOSELECT_CONFIG_FILE, JSON.stringify(newConfig, null, 2));

    // Start or stop auto-select loop
    if (newConfig.enabled) {
      startAutoSelect(newConfig);
    } else {
      stopAutoSelect();
    }

    return { success: true, message: 'Configuration enregistrée' };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// Determine which mode should be active based on averaged RPS
// Progressive transitions: economy ↔ auto ↔ performance (can't skip auto)
function determineMode(avgRps, config) {
  const { thresholds } = config;

  // Determine target mode based on thresholds
  let targetMode;
  if (avgRps < thresholds.low) {
    targetMode = 'economy';
  } else if (avgRps >= thresholds.high) {
    targetMode = 'performance';
  } else {
    targetMode = 'auto';
  }

  // If no current mode, set directly
  if (!currentAutoMode) {
    return targetMode;
  }

  // Progressive transitions - can only move one step at a time
  const modeOrder = ['economy', 'auto', 'performance'];
  const currentIndex = modeOrder.indexOf(currentAutoMode);
  const targetIndex = modeOrder.indexOf(targetMode);

  if (targetIndex > currentIndex) {
    // Moving up: economy → auto → performance
    return modeOrder[currentIndex + 1];
  } else if (targetIndex < currentIndex) {
    // Moving down: performance → auto → economy
    return modeOrder[currentIndex - 1];
  }

  // Same mode, no change
  return currentAutoMode;
}

// Check if current time is within night period (schedule)
function isInNightPeriod(scheduleConfig) {
  if (!scheduleConfig.enabled) return false;

  const now = new Date();
  const currentMinutes = now.getHours() * 60 + now.getMinutes();

  const [startH, startM] = scheduleConfig.nightStart.split(':').map(Number);
  const [endH, endM] = scheduleConfig.nightEnd.split(':').map(Number);

  const startMinutes = startH * 60 + startM;
  const endMinutes = endH * 60 + endM;

  // Handle overnight periods (e.g., 22:00 to 08:00)
  if (startMinutes > endMinutes) {
    return currentMinutes >= startMinutes || currentMinutes < endMinutes;
  }

  return currentMinutes >= startMinutes && currentMinutes < endMinutes;
}

// Auto-select update function
async function updateAutoSelect() {
  try {
    const { config } = await getAutoSelectConfig();
    if (!config.enabled) {
      stopAutoSelect();
      return;
    }

    // Get current RPS first (this also updates the averaging buffer)
    const rpsResult = await getNetworkRps();
    if (!rpsResult.success) {
      console.error('Auto-select: Failed to get RPS:', rpsResult.error);
      return;
    }

    const avgRps = rpsResult.averagedRps;

    // Check if schedule forces economy during night
    const { config: scheduleConfig } = await getScheduleConfig();
    if (isInNightPeriod(scheduleConfig)) {
      // Night period: force economy mode
      if (currentAutoMode !== 'economy') {
        console.log('Auto-select: Night period active, forcing economy mode');
        currentAutoMode = 'economy';
        await applyMode('economy');
      }
      // Still emit RPS update even during night period
      energyEvents.emit('rpsUpdate', {
        rps: rpsResult.rps,
        averagedRps: avgRps,
        appliedMode: currentAutoMode
      });
      return;
    }

    // Use averaged RPS for mode determination
    const targetMode = determineMode(avgRps, config);

    // Emit RPS update event
    energyEvents.emit('rpsUpdate', {
      rps: rpsResult.rps,
      averagedRps: avgRps,
      appliedMode: currentAutoMode
    });

    // Only change mode if different
    if (targetMode !== currentAutoMode) {
      console.log(`Auto-select: avgRPS=${avgRps}, switching from ${currentAutoMode} to ${targetMode}`);
      currentAutoMode = targetMode;
      await applyMode(targetMode);
    }
  } catch (error) {
    console.error('Auto-select update error:', error);
  }
}

// Start auto-select loop
function startAutoSelect(config) {
  stopAutoSelect();

  const interval = config.sampleInterval || DEFAULT_AUTOSELECT.sampleInterval;
  autoSelectInterval = setInterval(updateAutoSelect, interval);

  // Run immediately
  updateAutoSelect();

  console.log(`Auto-select started (interval: ${interval}ms)`);
}

// Stop auto-select loop
function stopAutoSelect() {
  if (autoSelectInterval) {
    clearInterval(autoSelectInterval);
    autoSelectInterval = null;
  }
  currentAutoMode = null;
  rpsHistory = [];
  averagedRps = 0;
}

// Get auto-select status
export function getAutoSelectStatus() {
  return {
    active: !!autoSelectInterval,
    currentMode: currentAutoMode,
    currentRps,
    averagedRps
  };
}

// Initialize auto-select on startup if enabled
async function initAutoSelect() {
  try {
    const { config } = await getAutoSelectConfig();
    if (config.enabled) {
      startAutoSelect(config);
    }
  } catch (error) {
    console.error('Failed to initialize auto-select:', error);
  }
}

// Call init on module load
initAutoSelect();

// ============ BENCHMARK ============

let benchmarkProcess = null;
let benchmarkStartTime = null;
let benchmarkTimeout = null;

export function getBenchmarkStatus() {
  if (!benchmarkProcess) {
    return { success: true, running: false };
  }

  const elapsed = Date.now() - benchmarkStartTime;
  return {
    success: true,
    running: true,
    elapsed: Math.round(elapsed / 1000),
    pid: benchmarkProcess.pid
  };
}

export async function startBenchmark(duration = 60) {
  if (benchmarkProcess) {
    return { success: false, error: 'Benchmark already running' };
  }

  const cpuCount = os.cpus().length;

  // Use stress-ng if available, fallback to yes command
  try {
    await execAsync('which stress-ng');
    // stress-ng available - use CPU stress test
    benchmarkProcess = spawn('stress-ng', ['--cpu', String(cpuCount), '--timeout', `${duration}s`], {
      detached: false,
      stdio: 'ignore'
    });
  } catch {
    // Fallback: use multiple yes processes piped to /dev/null
    benchmarkProcess = spawn('sh', ['-c', `for i in $(seq 1 ${cpuCount}); do yes > /dev/null & done; wait`], {
      detached: true,
      stdio: 'ignore'
    });
  }

  benchmarkStartTime = Date.now();

  // Auto-stop after duration
  benchmarkTimeout = setTimeout(() => {
    stopBenchmark();
  }, duration * 1000);

  benchmarkProcess.on('exit', () => {
    benchmarkProcess = null;
    benchmarkStartTime = null;
    if (benchmarkTimeout) {
      clearTimeout(benchmarkTimeout);
      benchmarkTimeout = null;
    }
  });

  return {
    success: true,
    message: `Benchmark started (${cpuCount} threads, ${duration}s)`,
    pid: benchmarkProcess.pid
  };
}

export function stopBenchmark() {
  if (!benchmarkProcess) {
    return { success: false, error: 'No benchmark running' };
  }

  try {
    // Kill the process and all children
    process.kill(-benchmarkProcess.pid, 'SIGTERM');
  } catch {
    try {
      benchmarkProcess.kill('SIGTERM');
    } catch {
      // Process already dead
    }
  }

  // Also kill any stray yes processes from fallback method
  try {
    exec('pkill -f "yes > /dev/null"');
  } catch {
    // Ignore
  }

  if (benchmarkTimeout) {
    clearTimeout(benchmarkTimeout);
    benchmarkTimeout = null;
  }

  const elapsed = benchmarkStartTime ? Math.round((Date.now() - benchmarkStartTime) / 1000) : 0;
  benchmarkProcess = null;
  benchmarkStartTime = null;

  return { success: true, message: `Benchmark stopped after ${elapsed}s` };
}
