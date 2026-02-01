import { promises as fs } from 'fs';
import { v4 as uuidv4 } from 'uuid';
import cron from 'node-cron';
import { sendWakeOnLan, shutdownServer, rebootServer } from './wol.js';
import { getServerById } from './servers.js';

const SCHEDULES_FILE = '/data/wol-schedules.json';

// Store active cron jobs
const cronJobs = new Map();

/**
 * Ensure schedules file exists
 */
async function ensureSchedulesFile() {
  try {
    await fs.access(SCHEDULES_FILE);
  } catch (error) {
    await fs.writeFile(SCHEDULES_FILE, JSON.stringify({ schedules: [] }, null, 2));
  }
}

/**
 * Read schedules from file
 */
export async function readSchedules() {
  await ensureSchedulesFile();
  const data = await fs.readFile(SCHEDULES_FILE, 'utf-8');
  return JSON.parse(data);
}

/**
 * Write schedules to file
 */
async function writeSchedules(data) {
  await fs.writeFile(SCHEDULES_FILE, JSON.stringify(data, null, 2));
}

/**
 * Get all schedules
 */
export async function listSchedules() {
  const data = await readSchedules();
  return data.schedules || [];
}

/**
 * Get schedule by ID
 */
export async function getScheduleById(id) {
  const schedules = await listSchedules();
  return schedules.find(s => s.id === id);
}

/**
 * Add a new schedule
 */
export async function addSchedule(scheduleData) {
  const { serverId, action, cron: cronExpression, description, enabled = true } = scheduleData;

  if (!serverId || !action || !cronExpression) {
    throw new Error('Missing required fields: serverId, action, cron');
  }

  // Validate server exists
  const server = await getServerById(serverId);
  if (!server) {
    throw new Error('Server not found');
  }

  // Validate action
  if (!['wake', 'shutdown', 'reboot'].includes(action)) {
    throw new Error('Invalid action. Must be: wake, shutdown, or reboot');
  }

  // Validate cron expression
  if (!cron.validate(cronExpression)) {
    throw new Error('Invalid cron expression');
  }

  const schedule = {
    id: uuidv4(),
    serverId,
    serverName: server.name,
    action,
    cron: cronExpression,
    description: description || `${action} ${server.name}`,
    enabled,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    lastRun: null,
    nextRun: null
  };

  // Save to file
  const data = await readSchedules();
  data.schedules.push(schedule);
  await writeSchedules(data);

  // Start cron job if enabled
  if (enabled) {
    startCronJob(schedule);
  }

  console.log(`Schedule ${schedule.id} created for ${server.name}: ${action} at ${cronExpression}`);

  return schedule;
}

/**
 * Update schedule
 */
export async function updateSchedule(id, updates) {
  const data = await readSchedules();
  const index = data.schedules.findIndex(s => s.id === id);

  if (index === -1) {
    throw new Error('Schedule not found');
  }

  const oldSchedule = data.schedules[index];

  // If cron expression is being updated, validate it
  if (updates.cron && !cron.validate(updates.cron)) {
    throw new Error('Invalid cron expression');
  }

  // If action is being updated, validate it
  if (updates.action && !['wake', 'shutdown', 'reboot'].includes(updates.action)) {
    throw new Error('Invalid action. Must be: wake, shutdown, or reboot');
  }

  // Update allowed fields
  const allowedFields = ['serverId', 'action', 'cron', 'description', 'enabled'];
  allowedFields.forEach(field => {
    if (updates[field] !== undefined) {
      data.schedules[index][field] = updates[field];
    }
  });

  data.schedules[index].updatedAt = new Date().toISOString();

  await writeSchedules(data);

  // Stop old cron job
  stopCronJob(id);

  // Start new cron job if enabled
  if (data.schedules[index].enabled) {
    startCronJob(data.schedules[index]);
  }

  console.log(`Schedule ${id} updated`);

  return data.schedules[index];
}

/**
 * Delete schedule
 */
export async function deleteSchedule(id) {
  const data = await readSchedules();
  const index = data.schedules.findIndex(s => s.id === id);

  if (index === -1) {
    throw new Error('Schedule not found');
  }

  const deleted = data.schedules.splice(index, 1)[0];
  await writeSchedules(data);

  // Stop cron job
  stopCronJob(id);

  console.log(`Schedule ${id} deleted`);

  return deleted;
}

/**
 * Execute schedule action
 */
async function executeSchedule(schedule) {
  console.log(`Executing schedule ${schedule.id}: ${schedule.action} on ${schedule.serverName}`);

  try {
    let result;

    switch (schedule.action) {
      case 'wake':
        result = await sendWakeOnLan(schedule.serverId);
        break;
      case 'shutdown':
        result = await shutdownServer(schedule.serverId);
        break;
      case 'reboot':
        result = await rebootServer(schedule.serverId);
        break;
      default:
        throw new Error(`Unknown action: ${schedule.action}`);
    }

    // Update last run time
    const data = await readSchedules();
    const index = data.schedules.findIndex(s => s.id === schedule.id);
    if (index !== -1) {
      data.schedules[index].lastRun = new Date().toISOString();
      await writeSchedules(data);
    }

    console.log(`Schedule ${schedule.id} executed successfully:`, result.message);
  } catch (error) {
    console.error(`Failed to execute schedule ${schedule.id}:`, error);
  }
}

/**
 * Start cron job for schedule
 */
function startCronJob(schedule) {
  if (cronJobs.has(schedule.id)) {
    console.warn(`Cron job ${schedule.id} already running, stopping it first`);
    stopCronJob(schedule.id);
  }

  try {
    const job = cron.schedule(schedule.cron, () => {
      executeSchedule(schedule);
    });

    cronJobs.set(schedule.id, job);
    console.log(`Cron job started for schedule ${schedule.id}: ${schedule.cron}`);
  } catch (error) {
    console.error(`Failed to start cron job for schedule ${schedule.id}:`, error);
  }
}

/**
 * Stop cron job for schedule
 */
function stopCronJob(id) {
  const job = cronJobs.get(id);
  if (job) {
    job.stop();
    cronJobs.delete(id);
    console.log(`Cron job stopped for schedule ${id}`);
  }
}

/**
 * Initialize all schedules
 * Called on server startup
 */
export async function initializeSchedules() {
  console.log('Initializing WOL schedules...');

  const schedules = await listSchedules();

  for (const schedule of schedules) {
    if (schedule.enabled) {
      startCronJob(schedule);
    }
  }

  console.log(`Initialized ${schedules.filter(s => s.enabled).length} enabled schedule(s)`);
}

/**
 * Stop all schedules
 * Called on server shutdown
 */
export async function stopAllSchedules() {
  console.log('Stopping all WOL schedules...');

  for (const [id, job] of cronJobs.entries()) {
    job.stop();
    console.log(`Stopped schedule ${id}`);
  }

  cronJobs.clear();
}

/**
 * Toggle schedule enabled status
 */
export async function toggleSchedule(id, enabled) {
  const data = await readSchedules();
  const index = data.schedules.findIndex(s => s.id === id);

  if (index === -1) {
    throw new Error('Schedule not found');
  }

  data.schedules[index].enabled = enabled;
  data.schedules[index].updatedAt = new Date().toISOString();

  await writeSchedules(data);

  if (enabled) {
    startCronJob(data.schedules[index]);
  } else {
    stopCronJob(id);
  }

  console.log(`Schedule ${id} ${enabled ? 'enabled' : 'disabled'}`);

  return data.schedules[index];
}

/**
 * Execute schedule manually (bypass cron)
 */
export async function executeScheduleManually(id) {
  const schedule = await getScheduleById(id);

  if (!schedule) {
    throw new Error('Schedule not found');
  }

  await executeSchedule(schedule);

  return {
    success: true,
    message: `Schedule executed manually: ${schedule.action} on ${schedule.serverName}`
  };
}
