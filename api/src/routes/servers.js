import { Router } from 'express';
import { requireAuth } from '../middleware/auth.js';
import {
  listServers,
  getServerById,
  addServer,
  updateServer,
  deleteServer,
  testServerConnection,
  getServerInterfaces,
  getServerInfo,
  refreshServerInterfaces,
  getAllGroups
} from '../services/servers.js';

const router = Router();

/**
 * GET /api/servers
 * List all servers
 */
router.get('/', requireAuth, async (req, res) => {
  try {
    const servers = await listServers();
    res.json({ success: true, data: servers });
  } catch (error) {
    console.error('Failed to list servers:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * GET /api/servers/groups
 * Get all groups
 */
router.get('/groups', requireAuth, async (req, res) => {
  try {
    const groups = await getAllGroups();
    res.json({ success: true, data: groups });
  } catch (error) {
    console.error('Failed to get groups:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * GET /api/servers/:id
 * Get server by ID
 */
router.get('/:id', requireAuth, async (req, res) => {
  try {
    const server = await getServerById(req.params.id);

    if (!server) {
      return res.status(404).json({ success: false, error: 'Server not found' });
    }

    res.json({ success: true, data: server });
  } catch (error) {
    console.error('Failed to get server:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * POST /api/servers
 * Add a new server
 */
router.post('/', requireAuth, async (req, res) => {
  try {
    const { name, host, port, username, password, groups, wolInterface } = req.body;

    if (!name || !host || !username || !password) {
      return res.status(400).json({
        success: false,
        error: 'Missing required fields: name, host, username, password'
      });
    }

    const result = await addServer({
      name,
      host,
      port: port || 22,
      username,
      password,
      groups: groups || [],
      wolInterface
    });

    res.json({
      success: true,
      data: result.server,
      interfaces: result.interfaces,
      message: 'Server added successfully'
    });
  } catch (error) {
    console.error('Failed to add server:', error);
    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * PUT /api/servers/:id
 * Update server
 */
router.put('/:id', requireAuth, async (req, res) => {
  try {
    const updates = req.body;
    const server = await updateServer(req.params.id, updates);

    res.json({
      success: true,
      data: server,
      message: 'Server updated successfully'
    });
  } catch (error) {
    console.error('Failed to update server:', error);

    if (error.message === 'Server not found') {
      return res.status(404).json({ success: false, error: error.message });
    }

    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * DELETE /api/servers/:id
 * Delete server
 */
router.delete('/:id', requireAuth, async (req, res) => {
  try {
    const server = await deleteServer(req.params.id);

    res.json({
      success: true,
      data: server,
      message: 'Server deleted successfully'
    });
  } catch (error) {
    console.error('Failed to delete server:', error);

    if (error.message === 'Server not found') {
      return res.status(404).json({ success: false, error: error.message });
    }

    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * POST /api/servers/:id/test
 * Test server connection
 */
router.post('/:id/test', requireAuth, async (req, res) => {
  try {
    const result = await testServerConnection(req.params.id);

    res.json({
      success: result.success,
      data: result,
      message: result.message
    });
  } catch (error) {
    console.error('Failed to test server connection:', error);

    if (error.message === 'Server not found') {
      return res.status(404).json({ success: false, error: error.message });
    }

    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * GET /api/servers/:id/interfaces
 * Get network interfaces from server
 */
router.get('/:id/interfaces', requireAuth, async (req, res) => {
  try {
    const interfaces = await getServerInterfaces(req.params.id);

    res.json({
      success: true,
      data: interfaces
    });
  } catch (error) {
    console.error('Failed to get server interfaces:', error);

    if (error.message === 'Server not found') {
      return res.status(404).json({ success: false, error: error.message });
    }

    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * POST /api/servers/:id/refresh-interfaces
 * Refresh server network interfaces
 */
router.post('/:id/refresh-interfaces', requireAuth, async (req, res) => {
  try {
    const interfaces = await refreshServerInterfaces(req.params.id);

    res.json({
      success: true,
      data: interfaces,
      message: 'Interfaces refreshed successfully'
    });
  } catch (error) {
    console.error('Failed to refresh server interfaces:', error);

    if (error.message === 'Server not found') {
      return res.status(404).json({ success: false, error: error.message });
    }

    res.status(500).json({ success: false, error: error.message });
  }
});

/**
 * GET /api/servers/:id/info
 * Get detailed server information
 */
router.get('/:id/info', requireAuth, async (req, res) => {
  try {
    const info = await getServerInfo(req.params.id);

    res.json({
      success: true,
      data: info
    });
  } catch (error) {
    console.error('Failed to get server info:', error);

    if (error.message === 'Server not found') {
      return res.status(404).json({ success: false, error: error.message });
    }

    res.status(500).json({ success: false, error: error.message });
  }
});

export default router;
