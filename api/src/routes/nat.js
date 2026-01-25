import { Router } from 'express';
import {
  getNatRules,
  getFilterRules,
  getMasqueradeRules,
  getPortForwards,
  getFirewallStatus,
  getRoutingRules,
  getChainStats
} from '../services/firewall.js';

const router = Router();

// GET /api/nat/rules - Règles NAT complètes
router.get('/rules', async (req, res) => {
  const result = await getNatRules();
  res.json(result);
});

// GET /api/nat/filter - Règles de filtrage
router.get('/filter', async (req, res) => {
  const result = await getFilterRules();
  res.json(result);
});

// GET /api/nat/masquerade - Règles MASQUERADE
router.get('/masquerade', async (req, res) => {
  const result = await getMasqueradeRules();
  res.json(result);
});

// GET /api/nat/forwards - Port forwards (DNAT)
router.get('/forwards', async (req, res) => {
  const result = await getPortForwards();
  res.json(result);
});

// GET /api/nat/status - Firewall status
router.get('/status', async (req, res) => {
  const result = await getFirewallStatus();
  res.json(result);
});

// GET /api/nat/routing-rules - Policy routing rules
router.get('/routing-rules', async (req, res) => {
  const result = await getRoutingRules();
  res.json(result);
});

// GET /api/nat/stats - Chain statistics
router.get('/stats', async (req, res) => {
  const result = await getChainStats();
  res.json(result);
});

export default router;
