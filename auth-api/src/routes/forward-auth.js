import { Router } from 'express';
import { validateSession } from '../services/sessions.js';
import { getUser } from '../services/users.js';

const router = Router();

// GET /api/authz/forward-auth - Caddy forward_auth endpoint
router.get('/forward-auth', (req, res) => {
  const sessionId = req.cookies.auth_session;

  // Get the original URL for redirect
  const forwardedHost = req.get('X-Forwarded-Host') || req.get('host');
  const forwardedUri = req.get('X-Forwarded-Uri') || '/';
  const forwardedProto = req.get('X-Forwarded-Proto') || 'https';

  const originalUrl = `${forwardedProto}://${forwardedHost}${forwardedUri}`;
  const loginUrl = `https://auth.mynetwk.biz/login?rd=${encodeURIComponent(originalUrl)}`;

  // No session cookie
  if (!sessionId) {
    // Return 401 with login redirect URL in header
    res.set('X-Auth-Redirect', loginUrl);
    return res.status(401).json({
      success: false,
      error: 'Authentication required',
      redirect: loginUrl
    });
  }

  // Validate session
  const session = validateSession(sessionId);

  if (!session) {
    res.set('X-Auth-Redirect', loginUrl);
    return res.status(401).json({
      success: false,
      error: 'Session expired',
      redirect: loginUrl
    });
  }

  // Get user info
  const user = getUser(session.userId);

  if (!user) {
    res.set('X-Auth-Redirect', loginUrl);
    return res.status(401).json({
      success: false,
      error: 'User not found',
      redirect: loginUrl
    });
  }

  // Check if user is disabled
  if (user.disabled) {
    res.set('X-Auth-Redirect', loginUrl);
    return res.status(403).json({
      success: false,
      error: 'Account disabled'
    });
  }

  // Set headers for downstream services
  res.set('Remote-User', user.username);
  res.set('Remote-Email', user.email || '');
  res.set('Remote-Name', user.displayname || user.username);
  res.set('Remote-Groups', (user.groups || []).join(','));

  // Authentication successful
  res.status(200).json({
    success: true,
    user: user.username
  });
});

// GET /api/authz/verify - Simple session verification (for internal use)
router.get('/verify', (req, res) => {
  const sessionId = req.cookies.auth_session;

  if (!sessionId) {
    return res.status(401).json({ valid: false });
  }

  const session = validateSession(sessionId);

  if (!session) {
    return res.status(401).json({ valid: false });
  }

  res.json({
    valid: true,
    user_id: session.userId
  });
});

export default router;
