import { Router } from 'express';
import {
  getCpuInfo,
  getGovernorStatus,
  setGovernor,
  getScheduleConfig,
  saveScheduleConfig,
  applyMode,
  getCurrentMode,
  getEnergyModes,
  getBenchmarkStatus,
  startBenchmark,
  stopBenchmark,
  getAutoSelectConfig,
  saveAutoSelectConfig,
  getNetworkRps,
  getAutoSelectStatus,
  getSelectableInterfaces,
  energyEvents
} from '../services/energy.js';

const router = Router();

// ============ SSE (Server-Sent Events) ============

// GET /api/energy/events - Real-time updates via SSE
router.get('/events', (req, res) => {
  // Set headers for SSE
  res.setHeader('Content-Type', 'text/event-stream');
  res.setHeader('Cache-Control', 'no-cache');
  res.setHeader('Connection', 'keep-alive');
  res.setHeader('X-Accel-Buffering', 'no'); // Disable nginx buffering
  res.flushHeaders();

  // Send initial connection event
  res.write('event: connected\ndata: {}\n\n');

  // Handler for mode changes
  const onModeChange = (data) => {
    res.write(`event: modeChange\ndata: ${JSON.stringify(data)}\n\n`);
  };

  // Handler for RPS updates
  const onRpsUpdate = (data) => {
    res.write(`event: rpsUpdate\ndata: ${JSON.stringify(data)}\n\n`);
  };

  // Subscribe to events
  energyEvents.on('modeChange', onModeChange);
  energyEvents.on('rpsUpdate', onRpsUpdate);

  // Heartbeat every 30 seconds to keep connection alive
  const heartbeat = setInterval(() => {
    res.write('event: heartbeat\ndata: {}\n\n');
  }, 30000);

  // Cleanup on connection close
  req.on('close', () => {
    clearInterval(heartbeat);
    energyEvents.off('modeChange', onModeChange);
    energyEvents.off('rpsUpdate', onRpsUpdate);
  });
});

// ============ CPU INFO ============

// GET /api/energy/cpu - Infos CPU (temp, freq, usage) pour polling
router.get('/cpu', async (req, res) => {
  const result = await getCpuInfo();
  res.json(result);
});

// ============ GOVERNOR ============

// GET /api/energy/status - Gouverneur actuel + disponibles
router.get('/status', async (req, res) => {
  const result = await getGovernorStatus();
  res.json(result);
});

// POST /api/energy/governor - Changer le gouverneur
router.post('/governor', async (req, res) => {
  const { governor } = req.body;

  if (!governor) {
    return res.status(400).json({ success: false, error: 'Governor is required' });
  }

  const result = await setGovernor(governor);
  res.json(result);
});

// ============ SCHEDULE ============

// GET /api/energy/schedule - Config de programmation
router.get('/schedule', async (req, res) => {
  const result = await getScheduleConfig();
  res.json(result);
});

// POST /api/energy/schedule - Sauvegarder la programmation
router.post('/schedule', async (req, res) => {
  const config = req.body;
  const result = await saveScheduleConfig(config);
  res.json(result);
});

// ============ AUTO-SELECT ============

// GET /api/energy/interfaces - Liste des interfaces réseau sélectionnables
router.get('/interfaces', async (req, res) => {
  const result = await getSelectableInterfaces();
  res.json(result);
});

// GET /api/energy/autoselect - Config de sélection automatique
router.get('/autoselect', async (req, res) => {
  const result = await getAutoSelectConfig();
  res.json(result);
});

// POST /api/energy/autoselect - Sauvegarder la config auto-select
router.post('/autoselect', async (req, res) => {
  const config = req.body;
  const result = await saveAutoSelectConfig(config);
  res.json(result);
});

// GET /api/energy/autoselect/rps - Requêtes par seconde sur interface SFP
router.get('/autoselect/rps', async (req, res) => {
  const result = await getNetworkRps();
  res.json(result);
});

// GET /api/energy/autoselect/status - Status de l'auto-select
router.get('/autoselect/status', (req, res) => {
  const result = getAutoSelectStatus();
  res.json(result);
});

// ============ ENERGY MODES ============

// GET /api/energy/modes - Liste des modes disponibles
router.get('/modes', (req, res) => {
  res.json(getEnergyModes());
});

// GET /api/energy/mode - Mode actuel
router.get('/mode', async (req, res) => {
  const result = await getCurrentMode();
  res.json(result);
});

// POST /api/energy/mode/:mode - Appliquer un mode (economy/auto/performance ou day/night)
router.post('/mode/:mode', async (req, res) => {
  const { mode } = req.params;
  const result = await applyMode(mode);
  res.json(result);
});

// ============ BENCHMARK ============

// GET /api/energy/benchmark - Status du benchmark
router.get('/benchmark', (req, res) => {
  const result = getBenchmarkStatus();
  res.json(result);
});

// POST /api/energy/benchmark/start - Démarrer le benchmark
router.post('/benchmark/start', async (req, res) => {
  const { duration = 60 } = req.body;
  const result = await startBenchmark(Math.min(60, Math.max(10, duration)));
  res.json(result);
});

// POST /api/energy/benchmark/stop - Arrêter le benchmark
router.post('/benchmark/stop', (req, res) => {
  const result = stopBenchmark();
  res.json(result);
});

export default router;
