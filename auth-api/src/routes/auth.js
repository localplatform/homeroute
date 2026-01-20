import { Router } from 'express';
import {
  getUserWithPassword,
  getUser,
  verifyPassword,
  updateLastLogin
} from '../services/users.js';
import {
  createSession,
  validateSession,
  deleteSession,
  getUserSessions,
  getSession
} from '../services/sessions.js';

const router = Router();

// Cookie options
const COOKIE_OPTIONS = {
  httpOnly: true,
  secure: true,
  sameSite: 'lax',
  domain: '.mynetwk.biz',
  path: '/'
};

// POST /api/auth/login - Login with username/password
router.post('/login', async (req, res) => {
  try {
    const { username, password, remember_me } = req.body;

    if (!username || !password) {
      return res.status(400).json({
        success: false,
        error: 'Nom d\'utilisateur et mot de passe requis'
      });
    }

    // Get user with password
    const user = getUserWithPassword(username.toLowerCase());

    if (!user) {
      // Don't reveal if user exists
      return res.status(401).json({
        success: false,
        error: 'Identifiants invalides'
      });
    }

    if (user.disabled) {
      return res.status(401).json({
        success: false,
        error: 'Compte désactivé'
      });
    }

    // Verify password
    const isValid = await verifyPassword(password, user.password);

    if (!isValid) {
      return res.status(401).json({
        success: false,
        error: 'Identifiants invalides'
      });
    }

    // Create session directly (no MFA)
    const { sessionId, expiresAt } = createSession(
      username,
      req.ip,
      req.get('user-agent'),
      !!remember_me
    );

    // Update last login
    updateLastLogin(username);

    // Set cookie
    res.cookie('auth_session', sessionId, {
      ...COOKIE_OPTIONS,
      maxAge: remember_me ? 30 * 24 * 60 * 60 * 1000 : undefined // 30 days or session
    });

    res.json({
      success: true,
      user: {
        username: user.username,
        displayname: user.displayname,
        email: user.email,
        groups: user.groups
      },
      expires_at: expiresAt
    });

  } catch (error) {
    console.error('Login error:', error);
    res.status(500).json({
      success: false,
      error: 'Erreur lors de la connexion'
    });
  }
});

// POST /api/auth/logout - Logout
router.post('/logout', (req, res) => {
  const sessionId = req.cookies.auth_session;

  if (sessionId) {
    deleteSession(sessionId);
  }

  res.clearCookie('auth_session', COOKIE_OPTIONS);

  res.json({ success: true });
});

// GET /api/auth/check - Check if logged in
router.get('/check', (req, res) => {
  const sessionId = req.cookies.auth_session;

  if (!sessionId) {
    return res.status(401).json({
      success: false,
      authenticated: false
    });
  }

  const session = validateSession(sessionId);

  if (!session) {
    res.clearCookie('auth_session', COOKIE_OPTIONS);
    return res.status(401).json({
      success: false,
      authenticated: false
    });
  }

  res.json({
    success: true,
    authenticated: true,
    user_id: session.userId
  });
});

// GET /api/auth/me - Get current user info
router.get('/me', (req, res) => {
  const sessionId = req.cookies.auth_session;

  if (!sessionId) {
    return res.status(401).json({
      success: false,
      error: 'Non authentifié'
    });
  }

  const session = validateSession(sessionId);

  if (!session) {
    res.clearCookie('auth_session', COOKIE_OPTIONS);
    return res.status(401).json({
      success: false,
      error: 'Session expirée'
    });
  }

  const user = getUser(session.userId);

  if (!user) {
    deleteSession(sessionId);
    res.clearCookie('auth_session', COOKIE_OPTIONS);
    return res.status(401).json({
      success: false,
      error: 'Utilisateur non trouvé'
    });
  }

  res.json({
    success: true,
    user: {
      username: user.username,
      displayname: user.displayname,
      email: user.email,
      groups: user.groups
    },
    session: {
      created_at: session.createdAt,
      expires_at: session.expiresAt,
      ip_address: session.ipAddress
    }
  });
});

// GET /api/auth/sessions - Get all sessions for current user
router.get('/sessions', (req, res) => {
  const sessionId = req.cookies.auth_session;

  if (!sessionId) {
    return res.status(401).json({
      success: false,
      error: 'Non authentifié'
    });
  }

  const session = validateSession(sessionId);

  if (!session) {
    return res.status(401).json({
      success: false,
      error: 'Session expirée'
    });
  }

  // Get all sessions for this user
  const sessions = getUserSessions(session.userId);

  res.json({
    success: true,
    sessions: sessions.map(s => ({
      id: s.id,
      current: s.id === sessionId,
      ip_address: s.ip_address,
      user_agent: s.user_agent,
      created_at: s.created_at,
      last_activity: s.last_activity,
      remember_me: s.remember_me === 1
    }))
  });
});

// DELETE /api/auth/sessions/:id - Revoke a specific session
router.delete('/sessions/:id', (req, res) => {
  const currentSessionId = req.cookies.auth_session;

  if (!currentSessionId) {
    return res.status(401).json({
      success: false,
      error: 'Non authentifié'
    });
  }

  const session = validateSession(currentSessionId);

  if (!session) {
    return res.status(401).json({
      success: false,
      error: 'Session expirée'
    });
  }

  const targetSessionId = req.params.id;

  // Don't allow revoking current session via this endpoint
  if (targetSessionId === currentSessionId) {
    return res.status(400).json({
      success: false,
      error: 'Utilisez /logout pour déconnecter la session actuelle'
    });
  }

  // Verify the target session belongs to the same user
  const targetSession = getSession(targetSessionId);

  if (!targetSession || targetSession.user_id !== session.userId) {
    return res.status(404).json({
      success: false,
      error: 'Session non trouvée'
    });
  }

  deleteSession(targetSessionId);

  res.json({ success: true });
});

export default router;
