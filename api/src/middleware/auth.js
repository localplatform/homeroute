/**
 * Middleware d'authentification local
 *
 * Verifie le cookie auth_session directement via les services locaux
 * et peuple req.user avec les infos utilisateur
 */

import { validateSession } from '../services/sessions.js';
import { getUser } from '../services/authUsers.js';

const BASE_DOMAIN = process.env.BASE_DOMAIN || 'localhost';

/**
 * Middleware global qui verifie le cookie auth_session
 * Ne bloque pas - peuple simplement req.user si authentifie
 */
export async function authMiddleware(req, res, next) {
  const sessionCookie = req.cookies?.auth_session;

  if (sessionCookie) {
    try {
      const session = validateSession(sessionCookie);
      if (session) {
        const user = getUser(session.userId);
        if (user && !user.disabled) {
          req.user = {
            username: user.username,
            email: user.email || '',
            displayName: user.displayname || user.username,
            groups: user.groups || [],
            isAdmin: user.groups?.includes('admins') || false,
            isPowerUser: user.groups?.includes('power_users') || false,
            hasGroup: (group) => user.groups?.includes(group) || false
          };
          // Compatibilite avec l'ancien nom
          req.autheliaUser = req.user;
        }
      }
    } catch (err) {
      console.error('Auth verification error:', err);
    }
  }

  next();
}

/**
 * Middleware qui exige une authentification
 * Retourne 401 avec URL de redirection si non authentifie
 */
export function requireAuth(req, res, next) {
  if (!req.user) {
    const originalUrl = req.get('X-Original-URL') || `https://proxy.${BASE_DOMAIN}${req.originalUrl}`;
    return res.status(401).json({
      success: false,
      error: 'Authentication required',
      authUrl: `https://auth.${BASE_DOMAIN}`,
      redirect: `https://auth.${BASE_DOMAIN}/login?rd=${encodeURIComponent(originalUrl)}`
    });
  }
  next();
}

/**
 * Middleware factory qui exige un ou plusieurs groupes specifiques
 * @param {...string} groups - Groupes autorises (au moins un doit correspondre)
 */
export function requireGroup(...groups) {
  return (req, res, next) => {
    if (!req.user) {
      return res.status(401).json({
        success: false,
        error: 'Authentication required',
        authUrl: `https://auth.${BASE_DOMAIN}`
      });
    }

    const userGroups = req.user.groups;
    const hasRequiredGroup = groups.some(g => userGroups.includes(g));

    if (!hasRequiredGroup) {
      return res.status(403).json({
        success: false,
        error: 'Insufficient permissions',
        requiredGroups: groups,
        userGroups: userGroups
      });
    }
    next();
  };
}

/**
 * Raccourci pour exiger le groupe admins
 */
export const requireAdmin = requireGroup('admins');

/**
 * Raccourci pour exiger admins ou power_users
 */
export const requirePowerUser = requireGroup('admins', 'power_users');

export default {
  authMiddleware,
  requireAuth,
  requireGroup,
  requireAdmin,
  requirePowerUser
};
