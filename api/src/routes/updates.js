import { Router } from 'express';
import {
  runFullCheck,
  cancelCheck,
  isCheckRunning,
  getLastCheckResult,
  runAptUpgrade,
  runSnapRefresh,
  isUpgradeRunning,
  cancelUpgrade
} from '../services/updates.js';

const router = Router();

// GET /api/updates/status - Check if update check is running
router.get('/status', (req, res) => {
  res.json({ running: isCheckRunning() });
});

// GET /api/updates/last - Get last check results (cached)
router.get('/last', (req, res) => {
  const result = getLastCheckResult();
  if (!result) {
    return res.json({ success: false, error: 'No check results available' });
  }
  res.json({ success: true, result });
});

// POST /api/updates/check - Start a new update check
router.post('/check', async (req, res) => {
  req.setTimeout(300000); // 5 min timeout
  const result = await runFullCheck();
  res.json(result);
});

// POST /api/updates/cancel - Cancel running check
router.post('/cancel', async (req, res) => {
  const result = await cancelCheck();
  res.json(result);
});

// --- Upgrade routes ---

// GET /api/updates/upgrade/status - Check if upgrade is running
router.get('/upgrade/status', (req, res) => {
  res.json({ running: isUpgradeRunning() });
});

// POST /api/updates/upgrade/apt - Run apt upgrade
router.post('/upgrade/apt', async (req, res) => {
  req.setTimeout(1800000); // 30 min timeout
  const result = await runAptUpgrade(false);
  res.json(result);
});

// POST /api/updates/upgrade/apt-full - Run apt full-upgrade
router.post('/upgrade/apt-full', async (req, res) => {
  req.setTimeout(1800000); // 30 min timeout
  const result = await runAptUpgrade(true);
  res.json(result);
});

// POST /api/updates/upgrade/snap - Run snap refresh
router.post('/upgrade/snap', async (req, res) => {
  req.setTimeout(1800000); // 30 min timeout
  const result = await runSnapRefresh();
  res.json(result);
});

// POST /api/updates/upgrade/cancel - Cancel running upgrade
router.post('/upgrade/cancel', async (req, res) => {
  const result = await cancelUpgrade();
  res.json(result);
});

export default router;
