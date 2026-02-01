import { Router } from 'express';
import { requireAuth } from '../middleware/auth.js';
import {
  sendWakeOnLan,
  shutdownServer,
  rebootServer,
  sendWakeOnLanBulk,
  shutdownServersBulk
} from '../services/wol.js';
import {
  listSchedules,
  getScheduleById,
  addSchedule,
  updateSchedule,
  deleteSchedule,
  toggleSchedule,
  executeScheduleManually
} from '../services/serverScheduler.js';

const router = Router();

// ========== WOL Actions ==========

/**
 * POST /api/wol/:id/wake
 * Send Wake-on-LAN magic packet
 */
router.post('/:id/wake', requireAuth, async (req, res) => {
  try {
    const result = await sendWakeOnLan(req.params.id);
    res.json({ success: true, data: result, message: result.message });
  } catch (error) {
    console.error('Failed to send WOL:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * POST /api/wol/:id/shutdown
 * Shutdown server via SSH
 */
router.post('/:id/shutdown', requireAuth, async (req, res) => {
  try {
    const result = await shutdownServer(req.params.id);
    res.json({ success: true, data: result, message: result.message });
  } catch (error) {
    console.error('Failed to shutdown server:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * POST /api/wol/:id/reboot
 * Reboot server via SSH
 */
router.post('/:id/reboot', requireAuth, async (req, res) => {
  try {
    const result = await rebootServer(req.params.id);
    res.json({ success: true, data: result, message: result.message });
  } catch (error) {
    console.error('Failed to reboot server:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * POST /api/wol/bulk/wake
 * Send WOL to multiple servers
 */
router.post('/bulk/wake', requireAuth, async (req, res) => {
  try {
    const { serverIds } = req.body;

    if (!serverIds || !Array.isArray(serverIds)) {
      return res.status(400).json({
        success: false,
        error: 'serverIds must be an array'
      });
    }

    const results = await sendWakeOnLanBulk(serverIds);
    const successCount = results.filter(r => r.success).length;

    res.json({
      success: true,
      data: results,
      message: `WOL sent to ${successCount}/${results.length} server(s)`
    });
  } catch (error) {
    console.error('Failed to send bulk WOL:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * POST /api/wol/bulk/shutdown
 * Shutdown multiple servers
 */
router.post('/bulk/shutdown', requireAuth, async (req, res) => {
  try {
    const { serverIds } = req.body;

    if (!serverIds || !Array.isArray(serverIds)) {
      return res.status(400).json({
        success: false,
        error: 'serverIds must be an array'
      });
    }

    const results = await shutdownServersBulk(serverIds);
    const successCount = results.filter(r => r.success).length;

    res.json({
      success: true,
      data: results,
      message: `Shutdown sent to ${successCount}/${results.length} server(s)`
    });
  } catch (error) {
    console.error('Failed to send bulk shutdown:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

// ========== Schedules ==========

/**
 * GET /api/wol/schedules
 * List all schedules
 */
router.get('/schedules', requireAuth, async (req, res) => {
  try {
    const schedules = await listSchedules();
    res.json({ success: true, data: schedules });
  } catch (error) {
    console.error('Failed to list schedules:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * GET /api/wol/schedules/:id
 * Get schedule by ID
 */
router.get('/schedules/:id', requireAuth, async (req, res) => {
  try {
    const schedule = await getScheduleById(req.params.id);

    if (!schedule) {
      return res.status(404).json({ success: false, error: 'Schedule not found' });
    }

    res.json({ success: true, data: schedule });
  } catch (error) {
    console.error('Failed to get schedule:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * POST /api/wol/schedules
 * Add a new schedule
 */
router.post('/schedules', requireAuth, async (req, res) => {
  try {
    const { serverId, action, cron, description, enabled } = req.body;

    if (!serverId || !action || !cron) {
      return res.status(400).json({
        success: false,
        error: 'Missing required fields: serverId, action, cron'
      });
    }

    const schedule = await addSchedule({
      serverId,
      action,
      cron,
      description,
      enabled
    });

    res.json({
      success: true,
      data: schedule,
      message: 'Schedule added successfully'
    });
  } catch (error) {
    console.error('Failed to add schedule:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * PUT /api/wol/schedules/:id
 * Update schedule
 */
router.put('/schedules/:id', requireAuth, async (req, res) => {
  try {
    const updates = req.body;
    const schedule = await updateSchedule(req.params.id, updates);

    res.json({
      success: true,
      data: schedule,
      message: 'Schedule updated successfully'
    });
  } catch (error) {
    console.error('Failed to update schedule:', error);

    if (error.message === 'Schedule not found') {
      return res.status(404).json({ success: false, error: error.message });
    }

    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * DELETE /api/wol/schedules/:id
 * Delete schedule
 */
router.delete('/schedules/:id', requireAuth, async (req, res) => {
  try {
    const schedule = await deleteSchedule(req.params.id);

    res.json({
      success: true,
      data: schedule,
      message: 'Schedule deleted successfully'
    });
  } catch (error) {
    console.error('Failed to delete schedule:', error);

    if (error.message === 'Schedule not found') {
      return res.status(404).json({ success: false, error: error.message });
    }

    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * POST /api/wol/schedules/:id/toggle
 * Toggle schedule enabled status
 */
router.post('/schedules/:id/toggle', requireAuth, async (req, res) => {
  try {
    const { enabled } = req.body;

    if (typeof enabled !== 'boolean') {
      return res.status(400).json({
        success: false,
        error: 'enabled must be a boolean'
      });
    }

    const schedule = await toggleSchedule(req.params.id, enabled);

    res.json({
      success: true,
      data: schedule,
      message: `Schedule ${enabled ? 'enabled' : 'disabled'} successfully`
    });
  } catch (error) {
    console.error('Failed to toggle schedule:', error);

    if (error.message === 'Schedule not found') {
      return res.status(404).json({ success: false, error: error.message });
    }

    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * POST /api/wol/schedules/:id/execute
 * Execute schedule manually
 */
router.post('/schedules/:id/execute', requireAuth, async (req, res) => {
  try {
    const result = await executeScheduleManually(req.params.id);

    res.json({
      success: true,
      data: result,
      message: result.message
    });
  } catch (error) {
    console.error('Failed to execute schedule:', error);

    if (error.message === 'Schedule not found') {
      return res.status(404).json({ success: false, error: error.message });
    }

    res.status(500).json({ success: false, error: error.message });
  }
});

export default router;
