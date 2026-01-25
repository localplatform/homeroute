import { Router } from 'express';
import {
  getAutheliaStatus,
  getUsers,
  getUser,
  createUser,
  updateUser,
  changePassword,
  deleteUser,
  getGroups,
  bootstrapAdmin,
  getInstallationInstructions
} from '../services/userManagement.js';

const router = Router();

// ========== Status Authelia ==========

// GET /api/users/authelia/status - Status du service Authelia
router.get('/authelia/status', async (req, res) => {
  const result = await getAutheliaStatus();
  res.json(result);
});

// GET /api/users/authelia/install - Instructions d'installation
router.get('/authelia/install', (req, res) => {
  const result = getInstallationInstructions();
  res.json(result);
});

// POST /api/users/authelia/bootstrap - Créer l'admin initial
router.post('/authelia/bootstrap', async (req, res) => {
  const { password } = req.body;
  if (!password || password.length < 8) {
    return res.status(400).json({
      success: false,
      error: 'Le mot de passe doit contenir au moins 8 caractères'
    });
  }
  const result = await bootstrapAdmin(password);
  res.json(result);
});

// ========== Groups (DOIT être avant /:username) ==========

// GET /api/users/groups - Liste des groupes
router.get('/groups', async (req, res) => {
  const result = await getGroups();
  res.json(result);
});

// ========== Users CRUD ==========

// GET /api/users - Liste des utilisateurs
router.get('/', async (req, res) => {
  const result = await getUsers();
  res.json(result);
});

// GET /api/users/:username - Détails d'un utilisateur
router.get('/:username', async (req, res) => {
  const { username } = req.params;
  const result = await getUser(username);
  if (!result.success) {
    return res.status(404).json(result);
  }
  res.json(result);
});

// POST /api/users - Créer un utilisateur
router.post('/', async (req, res) => {
  const { username, password, displayname, email, groups } = req.body;

  if (!username) {
    return res.status(400).json({ success: false, error: 'Nom d\'utilisateur requis' });
  }
  if (!password) {
    return res.status(400).json({ success: false, error: 'Mot de passe requis' });
  }

  const result = await createUser(username, password, displayname, email, groups);
  if (!result.success) {
    return res.status(400).json(result);
  }
  res.status(201).json(result);
});

// PUT /api/users/:username - Modifier un utilisateur
router.put('/:username', async (req, res) => {
  const { username } = req.params;
  const updates = req.body;

  const result = await updateUser(username, updates);
  if (!result.success) {
    return res.status(400).json(result);
  }
  res.json(result);
});

// PUT /api/users/:username/password - Changer le mot de passe
router.put('/:username/password', async (req, res) => {
  const { username } = req.params;
  const { password } = req.body;

  if (!password) {
    return res.status(400).json({ success: false, error: 'Nouveau mot de passe requis' });
  }

  const result = await changePassword(username, password);
  if (!result.success) {
    return res.status(400).json(result);
  }
  res.json(result);
});

// DELETE /api/users/:username - Supprimer un utilisateur
router.delete('/:username', async (req, res) => {
  const { username } = req.params;

  const result = await deleteUser(username);
  if (!result.success) {
    return res.status(400).json(result);
  }
  res.json(result);
});

export default router;
