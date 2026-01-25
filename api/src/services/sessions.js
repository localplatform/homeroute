/**
 * Gestion des sessions d'authentification
 */

import { v4 as uuidv4 } from 'uuid';
import { getDb, sessionQueries } from './authdb.js';

// Session durations
const SESSION_DURATION = 60 * 60 * 1000; // 1 hour
const REMEMBER_ME_DURATION = 30 * 24 * 60 * 60 * 1000; // 30 days
const INACTIVITY_TIMEOUT = 30 * 60 * 1000; // 30 minutes

// Create a new session
export function createSession(userId, ipAddress, userAgent, rememberMe = false) {
  const db = getDb();
  const sessionId = uuidv4();
  const now = Date.now();
  const duration = rememberMe ? REMEMBER_ME_DURATION : SESSION_DURATION;

  sessionQueries.create(db).run(
    sessionId,
    userId,
    now,
    now + duration,
    ipAddress,
    userAgent,
    now,
    rememberMe ? 1 : 0
  );

  return {
    sessionId,
    expiresAt: now + duration
  };
}

// Get session by ID
export function getSession(sessionId) {
  const db = getDb();
  const session = sessionQueries.get(db).get(sessionId);

  if (!session) {
    return null;
  }

  const now = Date.now();

  // Check if expired
  if (session.expires_at < now) {
    sessionQueries.delete(db).run(sessionId);
    return null;
  }

  // Check inactivity (unless remember_me is set)
  if (!session.remember_me && (now - session.last_activity) > INACTIVITY_TIMEOUT) {
    sessionQueries.delete(db).run(sessionId);
    return null;
  }

  return session;
}

// Update session activity
export function updateSessionActivity(sessionId) {
  const db = getDb();
  sessionQueries.updateActivity(db).run(Date.now(), sessionId);
}

// Delete session (logout)
export function deleteSession(sessionId) {
  const db = getDb();
  sessionQueries.delete(db).run(sessionId);
}

// Delete all sessions for user
export function deleteUserSessions(userId) {
  const db = getDb();
  sessionQueries.deleteByUserId(db).run(userId);
}

// Get all sessions for user
export function getUserSessions(userId) {
  const db = getDb();
  return sessionQueries.getByUserId(db).all(userId);
}

// Validate session and return user info
export function validateSession(sessionId) {
  const session = getSession(sessionId);

  if (!session) {
    return null;
  }

  // Update last activity
  updateSessionActivity(sessionId);

  return {
    userId: session.user_id,
    createdAt: session.created_at,
    expiresAt: session.expires_at,
    ipAddress: session.ip_address,
    userAgent: session.user_agent,
    rememberMe: session.remember_me === 1
  };
}

// Cleanup expired sessions (called periodically from authdb.js)
export function cleanupExpiredSessions() {
  const db = getDb();
  const now = Date.now();
  sessionQueries.deleteExpired(db).run(now);
}
