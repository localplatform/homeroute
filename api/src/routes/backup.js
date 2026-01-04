import { Router } from 'express';
import {
  getConfig,
  saveConfig,
  runBackup,
  getHistory,
  testConnection,
  cancelBackup,
  isBackupRunning,
  sendWakeOnLan,
  pingServer,
  getRemoteBackups,
  deleteRemoteItem,
  shutdownServer
} from '../services/backup.js';

const router = Router();

// GET /api/backup/config - Configuration actuelle
router.get('/config', async (req, res) => {
  const result = await getConfig();
  res.json(result);
});

// POST /api/backup/config - Sauvegarder configuration
router.post('/config', async (req, res) => {
  const { sources, wolMacAddress } = req.body;

  // Valider les sources si présentes
  if (sources !== undefined && !Array.isArray(sources)) {
    return res.status(400).json({ success: false, error: 'Sources must be an array' });
  }

  // Valider l'adresse MAC si présente
  if (wolMacAddress !== undefined && wolMacAddress !== '') {
    const macClean = wolMacAddress.replace(/[:-]/g, '').toLowerCase();
    if (!/^[0-9a-f]{12}$/.test(macClean)) {
      return res.status(400).json({ success: false, error: 'Invalid MAC address format' });
    }
  }

  const result = await saveConfig({ sources, wolMacAddress });
  res.json(result);
});

// POST /api/backup/run - Lancer un backup
router.post('/run', async (req, res) => {
  req.setTimeout(3600000);
  const result = await runBackup();
  res.json(result);
});

// GET /api/backup/history - Historique des backups
router.get('/history', async (req, res) => {
  const result = await getHistory();
  res.json(result);
});

// POST /api/backup/test - Tester connexion SMB
router.post('/test', async (req, res) => {
  const result = await testConnection();
  res.json(result);
});

// POST /api/backup/cancel - Annuler le backup en cours
router.post('/cancel', async (req, res) => {
  const result = await cancelBackup();
  res.json(result);
});

// GET /api/backup/status - Statut du backup
router.get('/status', (req, res) => {
  res.json({ running: isBackupRunning() });
});

// POST /api/backup/wake - Envoyer Wake-on-LAN
router.post('/wake', async (req, res) => {
  try {
    const configResult = await getConfig();
    if (!configResult.success) {
      return res.status(500).json({ success: false, error: 'Failed to load config' });
    }

    const { wolMacAddress } = configResult.config;
    if (!wolMacAddress) {
      return res.status(400).json({ success: false, error: 'No MAC address configured' });
    }

    const result = await sendWakeOnLan(wolMacAddress);
    res.json(result);
  } catch (error) {
    res.status(500).json({ success: false, error: error.message });
  }
});

// GET /api/backup/server-status - Vérifier si le serveur est en ligne (ping + SMB)
router.get('/server-status', async (req, res) => {
  try {
    const configResult = await getConfig();
    if (!configResult.success) {
      return res.status(500).json({ success: false, error: 'Failed to load config' });
    }

    const { smbServer } = configResult.config;
    if (!smbServer) {
      return res.status(400).json({ success: false, error: 'No SMB server configured' });
    }

    // Étape 1: Ping
    const pingResult = await pingServer(smbServer, 2000);
    if (!pingResult.online) {
      return res.json({ online: false, pingMs: null, smbOk: false });
    }

    // Étape 2: Test SMB (seulement si ping OK)
    const smbResult = await testConnection();

    res.json({
      online: true,
      pingMs: pingResult.pingMs,
      smbOk: smbResult.success
    });
  } catch (error) {
    res.status(500).json({ success: false, error: error.message });
  }
});

// GET /api/backup/remote - Lister le contenu d'un répertoire distant
router.get('/remote', async (req, res) => {
  const path = req.query.path || '';
  const result = await getRemoteBackups(path);
  res.json(result);
});

// DELETE /api/backup/remote - Supprimer un fichier ou dossier distant
router.delete('/remote', async (req, res) => {
  const { path } = req.body;
  if (!path) {
    return res.status(400).json({ success: false, error: 'Path is required' });
  }
  const result = await deleteRemoteItem(path);
  res.json(result);
});

// POST /api/backup/shutdown - Arrêter le serveur distant via SSH
router.post('/shutdown', async (req, res) => {
  try {
    const result = await shutdownServer();
    res.json(result);
  } catch (error) {
    res.status(500).json({ success: false, error: error.message });
  }
});

export default router;
