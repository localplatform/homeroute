/**
 * Routes d'authentification pour les applications proxifiées
 *
 * Ces endpoints permettent aux apps proxifiées de vérifier
 * le cookie auth_session et l'appartenance aux groupes
 */

import { Router } from 'express';
import http from 'http';

const router = Router();
const AUTH_SERVICE_URL = process.env.AUTH_SERVICE_URL || 'http://localhost:9100';

/**
 * Vérifie un cookie auth_session auprès de auth-service
 * @param {string} sessionCookie - Valeur du cookie auth_session
 * @returns {Promise<object|null>} - User info ou null si invalide
 */
async function verifySession(sessionCookie) {
  return new Promise((resolve) => {
    const url = new URL('/api/authz/forward-auth', AUTH_SERVICE_URL);

    const options = {
      hostname: url.hostname,
      port: url.port || 9100,
      path: url.pathname,
      method: 'GET',
      headers: {
        'Cookie': `auth_session=${sessionCookie}`,
        'X-Forwarded-Host': 'proxy.mynetwk.biz',
        'X-Forwarded-Proto': 'https'
      },
      timeout: 5000
    };

    const req = http.request(options, (res) => {
      let body = '';
      res.on('data', chunk => body += chunk);
      res.on('end', () => {
        if (res.statusCode === 200) {
          const user = {
            username: res.headers['remote-user'],
            email: res.headers['remote-email'] || '',
            displayName: res.headers['remote-name'] || res.headers['remote-user'],
            groups: res.headers['remote-groups'] ? res.headers['remote-groups'].split(',').map(g => g.trim()) : []
          };

          if (user.username) {
            user.isAdmin = user.groups.includes('admins');
            user.isPowerUser = user.groups.includes('power_users');
            resolve(user);
          } else {
            resolve(null);
          }
        } else {
          resolve(null);
        }
      });
    });

    req.on('error', () => resolve(null));
    req.on('timeout', () => {
      req.destroy();
      resolve(null);
    });

    req.end();
  });
}

/**
 * POST /api/authproxy/verify
 *
 * Vérifie un cookie auth_session et retourne les infos utilisateur
 *
 * Body: { cookie: "auth_session_value" }
 * Response: { valid: true, user: {...} } ou { valid: false }
 */
router.post('/verify', async (req, res) => {
  const { cookie } = req.body;

  if (!cookie) {
    return res.status(400).json({
      valid: false,
      error: 'Cookie required'
    });
  }

  try {
    const user = await verifySession(cookie);

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
 * Vérifie si un utilisateur appartient à un ou plusieurs groupes
 *
 * Body: { cookie: "auth_session_value", groups: ["admins", "power_users"] }
 * Response: { valid: true, hasAccess: true, matchedGroups: ["admins"] }
 */
router.post('/check-group', async (req, res) => {
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
    const user = await verifySession(cookie);

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
