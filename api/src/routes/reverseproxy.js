import { Router } from 'express';
import {
  getConfig,
  updateBaseDomain,
  getHosts,
  addHost,
  updateHost,
  deleteHost,
  toggleHost,
  getCaddyStatus,
  reloadCaddy,
  renewCertificates,
  getSystemRouteStatus,
  getCertificatesStatus
} from '../services/reverseproxy.js';

const router = Router();

// ========== Configuration Endpoints ==========

// GET /api/reverseproxy/config - Configuration générale
router.get('/config', async (req, res) => {
  const result = await getConfig();
  res.json(result);
});

// PUT /api/reverseproxy/config/domain - Modifier domaine de base
router.put('/config/domain', async (req, res) => {
  const { baseDomain } = req.body;
  if (!baseDomain) {
    return res.status(400).json({ success: false, error: 'Base domain required' });
  }
  const result = await updateBaseDomain(baseDomain);
  res.json(result);
});

// ========== Host Management Endpoints ==========

// GET /api/reverseproxy/hosts - Liste des hôtes
router.get('/hosts', async (req, res) => {
  const result = await getHosts();
  res.json(result);
});

// POST /api/reverseproxy/hosts - Ajouter un hôte
router.post('/hosts', async (req, res) => {
  const { subdomain, customDomain, targetHost, targetPort, localOnly, requireAuth } = req.body;
  if (!targetHost || !targetPort) {
    return res.status(400).json({ success: false, error: 'Target host and port required' });
  }
  if (!subdomain && !customDomain) {
    return res.status(400).json({ success: false, error: 'Subdomain or custom domain required' });
  }
  const result = await addHost({ subdomain, customDomain, targetHost, targetPort, localOnly, requireAuth });
  res.json(result);
});

// PUT /api/reverseproxy/hosts/:id - Modifier un hôte
router.put('/hosts/:id', async (req, res) => {
  const result = await updateHost(req.params.id, req.body);
  res.json(result);
});

// DELETE /api/reverseproxy/hosts/:id - Supprimer un hôte
router.delete('/hosts/:id', async (req, res) => {
  const result = await deleteHost(req.params.id);
  res.json(result);
});

// POST /api/reverseproxy/hosts/:id/toggle - Activer/désactiver un hôte
router.post('/hosts/:id/toggle', async (req, res) => {
  const { enabled } = req.body;
  const result = await toggleHost(req.params.id, enabled);
  res.json(result);
});

// ========== Caddy Status Endpoints ==========

// GET /api/reverseproxy/status - Statut Caddy
router.get('/status', async (req, res) => {
  const caddyStatus = await getCaddyStatus();
  res.json({
    success: true,
    caddy: caddyStatus
  });
});

// POST /api/reverseproxy/reload - Recharger Caddy
router.post('/reload', async (req, res) => {
  const result = await reloadCaddy();
  res.json(result);
});

// POST /api/reverseproxy/certificates/renew - Renouveler les certificats
router.post('/certificates/renew', async (req, res) => {
  const result = await renewCertificates();
  res.json(result);
});

// GET /api/reverseproxy/certificates/status - Status des certificats
router.get('/certificates/status', async (req, res) => {
  const result = await getCertificatesStatus();
  res.json(result);
});

// ========== System Route Endpoints ==========

// GET /api/reverseproxy/system-route - Status de la route système
router.get('/system-route', async (req, res) => {
  const result = await getSystemRouteStatus();
  res.json(result);
});

export default router;
