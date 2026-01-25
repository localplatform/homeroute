/**
 * Routes d'authentification pour les applications proxifiees
 *
 * Ces endpoints permettent aux apps proxifiees de verifier
 * le cookie auth_session et l'appartenance aux groupes
 */

import { Router } from 'express';
import { validateSession } from '../services/sessions.js';
import { getUser } from '../services/authUsers.js';

const router = Router();

/**
 * Verifie un cookie auth_session localement
 * @param {string} sessionCookie - Valeur du cookie auth_session
 * @returns {object|null} - User info ou null si invalide
 */
function verifySessionLocal(sessionCookie) {
  const session = validateSession(sessionCookie);
  if (!session) {
    return null;
  }

  const user = getUser(session.userId);
  if (!user || user.disabled) {
    return null;
  }

  return {
    username: user.username,
    email: user.email || '',
    displayName: user.displayname || user.username,
    groups: user.groups || [],
    isAdmin: user.groups?.includes('admins') || false,
    isPowerUser: user.groups?.includes('power_users') || false
  };
}

/**
 * POST /api/authproxy/verify
 *
 * Verifie un cookie auth_session et retourne les infos utilisateur
 *
 * Body: { cookie: "auth_session_value" }
 * Response: { valid: true, user: {...} } ou { valid: false }
 */
router.post('/verify', (req, res) => {
  const { cookie } = req.body;

  if (!cookie) {
    return res.status(400).json({
      valid: false,
      error: 'Cookie required'
    });
  }

  try {
    const user = verifySessionLocal(cookie);

    if (user) {
      res.json({
        valid: true,
        user: {
          username: user.username,
          email: user.email,
          displayName: user.displayName,
          groups: user.groups,
          isAdmin: user.isAdmin,
          isPowerUser: user.isPowerUser
        }
      });
    } else {
      res.status(401).json({ valid: false });
    }
  } catch (err) {
    console.error('Auth verification error:', err);
    res.status(500).json({
      valid: false,
      error: 'Verification failed'
    });
  }
});

/**
 * POST /api/authproxy/check-group
 *
 * Verifie si un utilisateur appartient a un ou plusieurs groupes
 *
 * Body: { cookie: "auth_session_value", groups: ["admins", "power_users"] }
 * Response: { valid: true, hasAccess: true, matchedGroups: ["admins"] }
 */
router.post('/check-group', (req, res) => {
  const { cookie, groups } = req.body;

  if (!cookie) {
    return res.status(400).json({
      valid: false,
      error: 'Cookie required'
    });
  }

  if (!groups || !Array.isArray(groups) || groups.length === 0) {
    return res.status(400).json({
      valid: false,
      error: 'Groups array required'
    });
  }

  try {
    const user = verifySessionLocal(cookie);

    if (user) {
      const matchedGroups = groups.filter(g => user.groups.includes(g));
      const hasAccess = matchedGroups.length > 0;

      res.json({
        valid: true,
        hasAccess,
        matchedGroups,
        user: {
          username: user.username,
          groups: user.groups
        }
      });
    } else {
      res.status(401).json({
        valid: false,
        hasAccess: false
      });
    }
  } catch (err) {
    console.error('Group check error:', err);
    res.status(500).json({
      valid: false,
      error: 'Verification failed'
    });
  }
});

export default router;
