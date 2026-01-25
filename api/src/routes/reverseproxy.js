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
  getCertificatesStatus,
  // Environment management
  getEnvironments,
  addEnvironment,
  updateEnvironment,
  deleteEnvironment,
  // Application management
  getApplications,
  addApplication,
  updateApplication,
  deleteApplication,
  toggleApplication,
  // Cloudflare
  getCloudflareConfig,
  updateCloudflareConfig
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

// ========== Environment Endpoints ==========

// GET /api/reverseproxy/environments - Liste des environnements
router.get('/environments', async (req, res) => {
  const result = await getEnvironments();
  res.json(result);
});

// POST /api/reverseproxy/environments - Ajouter un environnement
router.post('/environments', async (req, res) => {
  const { name, prefix, apiPrefix } = req.body;
  if (!name) {
    return res.status(400).json({ success: false, error: 'Name is required' });
  }
  const result = await addEnvironment({ name, prefix, apiPrefix });
  res.json(result);
});

// PUT /api/reverseproxy/environments/:id - Modifier un environnement
router.put('/environments/:id', async (req, res) => {
  const result = await updateEnvironment(req.params.id, req.body);
  res.json(result);
});

// DELETE /api/reverseproxy/environments/:id - Supprimer un environnement
router.delete('/environments/:id', async (req, res) => {
  const result = await deleteEnvironment(req.params.id);
  res.json(result);
});

// ========== Application Endpoints ==========

// GET /api/reverseproxy/applications - Liste des applications
router.get('/applications', async (req, res) => {
  const result = await getApplications();
  res.json(result);
});

// POST /api/reverseproxy/applications - Ajouter une application
router.post('/applications', async (req, res) => {
  const { name, slug, endpoints } = req.body;
  if (!name || !slug) {
    return res.status(400).json({ success: false, error: 'Name and slug are required' });
  }
  if (!endpoints || typeof endpoints !== 'object' || Object.keys(endpoints).length === 0) {
    return res.status(400).json({ success: false, error: 'At least one environment endpoint is required' });
  }
  const result = await addApplication({ name, slug, endpoints });
  res.json(result);
});

// PUT /api/reverseproxy/applications/:id - Modifier une application
router.put('/applications/:id', async (req, res) => {
  const result = await updateApplication(req.params.id, req.body);
  res.json(result);
});

// DELETE /api/reverseproxy/applications/:id - Supprimer une application
router.delete('/applications/:id', async (req, res) => {
  const result = await deleteApplication(req.params.id);
  res.json(result);
});

// POST /api/reverseproxy/applications/:id/toggle - Activer/désactiver une application
router.post('/applications/:id/toggle', async (req, res) => {
  const { enabled } = req.body;
  const result = await toggleApplication(req.params.id, enabled);
  res.json(result);
});

// ========== Cloudflare Endpoints ==========

// GET /api/reverseproxy/cloudflare - Configuration Cloudflare
router.get('/cloudflare', async (req, res) => {
  const result = await getCloudflareConfig();
  res.json(result);
});

// PUT /api/reverseproxy/cloudflare - Modifier configuration Cloudflare
router.put('/cloudflare', async (req, res) => {
  const result = await updateCloudflareConfig(req.body);
  res.json(result);
});

export default router;
