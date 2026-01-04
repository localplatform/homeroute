import { readFile, writeFile, mkdir, readdir } from 'fs/promises';
import { existsSync } from 'fs';
import { exec, spawn } from 'child_process';
import { promisify } from 'util';
import path from 'path';
import os from 'os';

const execAsync = promisify(exec);

// Config paths
const CONFIG_DIR = process.env.ENERGY_CONFIG_DIR || '/var/lib/server-dashboard';
const SCHEDULE_CONFIG_FILE = path.join(CONFIG_DIR, 'energy-schedule.json');
const FAN_PROFILES_FILE = path.join(CONFIG_DIR, 'fan-profiles.json');
const AUTOSELECT_CONFIG_FILE = path.join(CONFIG_DIR, 'energy-autoselect.json');

// Unified energy modes
export const ENERGY_MODES = {
  economy: { governor: 'powersave', fanProfile: 'silent', label: 'Économie', icon: 'Moon' },
  auto: { governor: 'schedutil', fanProfile: 'balanced', label: 'Auto', icon: 'Zap' },
  performance: { governor: 'performance', fanProfile: 'performance', label: 'Performance', icon: 'Rocket' }
};

// CPU sysfs paths
const CPU_FREQ_PATH = '/sys/devices/system/cpu/cpu0/cpufreq';
const HWMON_PATH = '/sys/class/hwmon';

// Cache for hwmon paths (they can change between boots)
let it87HwmonPath = null;
let k10tempHwmonPath = null;

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

// Get IT87 hwmon path (cached)
async function getIt87Path() {
  if (!it87HwmonPath) {
    it87HwmonPath = await findHwmonByName('it8686');
    if (!it87HwmonPath) {
      // Try finding by platform device
      try {
        const { stdout } = await execAsync('ls -d /sys/devices/platform/it87.*/hwmon/hwmon* 2>/dev/null | head -1');
        it87HwmonPath = stdout.trim() || null;
      } catch {
        it87HwmonPath = null;
      }
    }
  }
  return it87HwmonPath;
}

// Get k10temp hwmon path (cached)
async function getK10tempPath() {
  if (!k10tempHwmonPath) {
    k10tempHwmonPath = await findHwmonByName('k10temp');
  }
  return k10tempHwmonPath;
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
    const k10tempPath = await getK10tempPath();
    if (!k10tempPath) {
      return { success: false, error: 'k10temp not found' };
    }

    const tempRaw = await readSysfs(path.join(k10tempPath, 'temp1_input'));
    if (!tempRaw) {
      return { success: false, error: 'Cannot read temperature' };
    }

    const tempC = parseInt(tempRaw) / 1000;
    return { success: true, temperature: tempC };
  } catch (error) {
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

export async function getCpuInfo() {
  const [tempResult, freqResult, usageResult] = await Promise.all([
    getCpuTemperature(),
    getCpuFrequency(),
    getCpuUsage()
  ]);

  return {
    success: true,
    temperature: tempResult.success ? tempResult.temperature : null,
    frequency: freqResult.success ? freqResult : null,
    usage: usageResult.success ? usageResult.usage : null
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

// ============ FANS ============

export async function getFanStatus() {
  try {
    const it87Path = await getIt87Path();
    if (!it87Path) {
      return { success: false, error: 'IT87 driver not loaded', available: false };
    }

    const fans = [];

    // Check fan1 and fan2
    for (let i = 1; i <= 2; i++) {
      const rpmPath = path.join(it87Path, `fan${i}_input`);
      const pwmPath = path.join(it87Path, `pwm${i}`);
      const enablePath = path.join(it87Path, `pwm${i}_enable`);

      if (existsSync(rpmPath)) {
        const rpm = await readSysfs(rpmPath);
        const pwm = await readSysfs(pwmPath);
        const enable = await readSysfs(enablePath);

        fans.push({
          id: `fan${i}`,
          name: i === 1 ? 'CPU_FAN' : 'SYS_FAN',
          rpm: rpm ? parseInt(rpm) : 0,
          pwm: pwm ? parseInt(pwm) : 0,
          pwmPercent: pwm ? Math.round((parseInt(pwm) / 255) * 100) : 0,
          mode: enable === '1' ? 'manual' : (enable === '2' ? 'auto' : 'off')
        });
      }
    }

    return { success: true, fans, available: true };
  } catch (error) {
    return { success: false, error: error.message, available: false };
  }
}

export async function setFanSpeed(fanId, pwm, mode) {
  try {
    const it87Path = await getIt87Path();
    if (!it87Path) {
      return { success: false, error: 'IT87 driver not loaded' };
    }

    const fanNum = fanId.replace('fan', '');
    const pwmPath = path.join(it87Path, `pwm${fanNum}`);
    const enablePath = path.join(it87Path, `pwm${fanNum}_enable`);

    if (!existsSync(pwmPath)) {
      return { success: false, error: `Fan ${fanId} not found` };
    }

    // Set mode if provided
    if (mode !== undefined) {
      const modeValue = mode === 'manual' ? '1' : (mode === 'auto' ? '2' : '0');
      await writeSysfs(enablePath, modeValue);
    }

    // Set PWM if provided and mode is manual
    if (pwm !== undefined) {
      // Ensure mode is manual before setting PWM
      const currentMode = await readSysfs(enablePath);
      if (currentMode !== '1') {
        await writeSysfs(enablePath, '1');
      }

      const pwmValue = Math.min(255, Math.max(0, Math.round(pwm)));
      await writeSysfs(pwmPath, String(pwmValue));
    }

    return { success: true, message: `Fan ${fanId} updated` };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ============ FAN PROFILES ============

const DEFAULT_PROFILES = [
  {
    name: 'silent',
    label: 'Économie',
    fans: {
      fan1: {
        mode: 'manual',
        pwm: 70,   // ~27% - silencieux
        curve: [[30, 20], [50, 35], [70, 100]]
      },
      fan2: {
        mode: 'manual',
        pwm: 50,   // ~20%
        curve: [[30, 15], [50, 30], [70, 80]]
      }
    }
  },
  {
    name: 'balanced',
    label: 'Auto',
    fans: {
      fan1: {
        mode: 'manual',
        pwm: 120,  // ~47% - équilibré
        curve: [[30, 35], [50, 55], [70, 100]]
      },
      fan2: {
        mode: 'manual',
        pwm: 100,  // ~39%
        curve: [[30, 30], [50, 50], [70, 90]]
      }
    }
  },
  {
    name: 'performance',
    label: 'Performance',
    fans: {
      fan1: {
        mode: 'manual',
        pwm: 200,  // ~78% - performance
        curve: [[30, 50], [50, 75], [70, 100]]
      },
      fan2: {
        mode: 'manual',
        pwm: 170,  // ~67%
        curve: [[30, 45], [50, 70], [70, 100]]
      }
    }
  }
];

export async function getFanProfiles() {
  try {
    await ensureConfigDir();

    if (!existsSync(FAN_PROFILES_FILE)) {
      // Return default profiles
      return { success: true, profiles: DEFAULT_PROFILES };
    }

    const content = await readFile(FAN_PROFILES_FILE, 'utf-8');
    const profiles = JSON.parse(content);
    return { success: true, profiles };
  } catch (error) {
    return { success: false, error: error.message, profiles: DEFAULT_PROFILES };
  }
}

export async function saveFanProfile(profile) {
  try {
    await ensureConfigDir();

    let profiles = [];
    if (existsSync(FAN_PROFILES_FILE)) {
      const content = await readFile(FAN_PROFILES_FILE, 'utf-8');
      profiles = JSON.parse(content);
    } else {
      profiles = [...DEFAULT_PROFILES];
    }

    // Update or add profile
    const existingIndex = profiles.findIndex(p => p.name === profile.name);
    if (existingIndex >= 0) {
      profiles[existingIndex] = profile;
    } else {
      profiles.push(profile);
    }

    await writeFile(FAN_PROFILES_FILE, JSON.stringify(profiles, null, 2));
    return { success: true, message: 'Profile saved' };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// Interpolate PWM value from curve based on temperature
function interpolateCurve(curve, temp) {
  if (!curve || curve.length === 0) return 128; // Default 50%

  // If temp is below first point, use first point's PWM
  if (temp <= curve[0][0]) {
    return Math.round((curve[0][1] / 100) * 255);
  }

  // If temp is above last point, use last point's PWM
  if (temp >= curve[curve.length - 1][0]) {
    return Math.round((curve[curve.length - 1][1] / 100) * 255);
  }

  // Find surrounding points and interpolate
  for (let i = 0; i < curve.length - 1; i++) {
    if (temp >= curve[i][0] && temp <= curve[i + 1][0]) {
      const [t1, p1] = curve[i];
      const [t2, p2] = curve[i + 1];
      const pwmPercent = p1 + ((temp - t1) / (t2 - t1)) * (p2 - p1);
      return Math.round((pwmPercent / 100) * 255);
    }
  }

  return 128;
}

export async function applyFanProfile(profileName) {
  try {
    const { profiles } = await getFanProfiles();
    const profile = profiles.find(p => p.name === profileName);

    if (!profile) {
      return { success: false, error: `Profile ${profileName} not found` };
    }

    // Get current CPU temperature for curve interpolation
    const tempResult = await getCpuTemperature();
    const currentTemp = tempResult.success ? tempResult.temperature : 40;

    // Apply each fan setting using curve interpolation
    for (const [fanId, settings] of Object.entries(profile.fans)) {
      let pwmValue;

      if (settings.curve && settings.curve.length > 0) {
        // Use curve interpolation based on current temperature
        pwmValue = interpolateCurve(settings.curve, currentTemp);
      } else {
        // Fallback to fixed PWM value
        pwmValue = settings.pwm || 128;
      }

      await setFanSpeed(fanId, pwmValue, settings.mode);
    }

    // Start the continuous fan control loop for this profile
    startFanControl(profileName);

    return { success: true, message: `Profile ${profileName} applied`, temperature: currentTemp };
  } catch (error) {
    return { success: false, error: error.message };
  }
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

    // Apply fan profile
    await applyFanProfile(modeConfig.fanProfile);

    return { success: true, message: `Mode ${modeConfig.label} appliqué`, mode, config: modeConfig };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ============ FAN CONTROL LOOP ============

let activeFanProfile = null;
let fanControlInterval = null;
const FAN_CONTROL_INTERVAL_MS = 1000; // Check every 1 second

// Update fan speeds based on current temperature and active profile
async function updateFanSpeeds() {
  if (!activeFanProfile) return;

  try {
    const { profiles } = await getFanProfiles();
    const profile = profiles.find(p => p.name === activeFanProfile);

    if (!profile) {
      console.error(`Fan profile ${activeFanProfile} not found, stopping control loop`);
      stopFanControl();
      return;
    }

    // Get current CPU temperature
    const tempResult = await getCpuTemperature();
    if (!tempResult.success) return;

    const currentTemp = tempResult.temperature;

    // Apply interpolated PWM for each fan
    for (const [fanId, settings] of Object.entries(profile.fans)) {
      if (settings.curve && settings.curve.length > 0) {
        const pwmValue = interpolateCurve(settings.curve, currentTemp);
        // Only set PWM, don't change mode (already set to manual)
        const it87Path = await getIt87Path();
        if (it87Path) {
          const fanNum = fanId.replace('fan', '');
          const pwmPath = path.join(it87Path, `pwm${fanNum}`);
          await writeSysfs(pwmPath, String(pwmValue));
        }
      }
    }
  } catch (error) {
    console.error('Fan control loop error:', error);
  }
}

// Start the fan control loop
function startFanControl(profileName) {
  // Stop any existing loop
  stopFanControl();

  activeFanProfile = profileName;

  // Start new control loop
  fanControlInterval = setInterval(updateFanSpeeds, FAN_CONTROL_INTERVAL_MS);

  // Also run immediately
  updateFanSpeeds();

  console.log(`Fan control loop started for profile: ${profileName}`);
}

// Stop the fan control loop
function stopFanControl() {
  if (fanControlInterval) {
    clearInterval(fanControlInterval);
    fanControlInterval = null;
  }
  activeFanProfile = null;
}

// Get current fan control status
export function getFanControlStatus() {
  return {
    active: !!activeFanProfile,
    profile: activeFanProfile,
    intervalMs: FAN_CONTROL_INTERVAL_MS
  };
}

// ============ AUTO-SELECT (based on network RPS) ============

const DEFAULT_AUTOSELECT = {
  enabled: false,
  networkInterface: null,  // Auto-detected via IP 10.0.0.10
  thresholds: {
    low: 1000,    // Below this -> economy (req/s)
    high: 10000   // Above this -> performance (req/s)
  },
  averagingTime: 3,      // Averaging window in seconds (replaces hysteresis)
  sampleInterval: 1000   // Sampling interval (ms) - faster for smooth averaging
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

// Get current RPS (requests per second) for the SFP interface
export async function getNetworkRps() {
  try {
    const { config } = await getAutoSelectConfig();

    // Find interface if not cached
    let interfaceName = config.networkInterface;
    if (!interfaceName) {
      interfaceName = await findInterfaceByIp('10.0.0.10');
      if (!interfaceName) {
        return { success: false, error: 'SFP interface (10.0.0.10) not found', rps: 0, averagedRps: 0 };
      }
    }

    const stats = await getNetworkPackets(interfaceName);
    if (!stats.success) {
      return { success: false, error: stats.error, rps: 0, averagedRps: 0 };
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

    // Schedule and auto-select can now work together
    // Night mode from schedule will force economy, overriding auto-select

    await writeFile(AUTOSELECT_CONFIG_FILE, JSON.stringify(newConfig, null, 2));

    // Start or stop auto-select loop
    if (newConfig.enabled) {
      startAutoSelect(newConfig);
    } else {
      stopAutoSelect();
    }

    return { success: true, message: 'Auto-select config saved' };
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

    // Check if schedule forces economy during night
    const { config: scheduleConfig } = await getScheduleConfig();
    if (isInNightPeriod(scheduleConfig)) {
      // Night period: force economy mode
      if (currentAutoMode !== 'economy') {
        console.log('Auto-select: Night period active, forcing economy mode');
        currentAutoMode = 'economy';
        await applyMode('economy');
      }
      return;
    }

    // Get current RPS (this also updates the averaging buffer)
    const rpsResult = await getNetworkRps();
    if (!rpsResult.success) {
      console.error('Auto-select: Failed to get RPS:', rpsResult.error);
      return;
    }

    // Use averaged RPS for mode determination
    const avgRps = rpsResult.averagedRps;
    const targetMode = determineMode(avgRps, config);

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

  const interval = config.sampleInterval || 5000;
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
