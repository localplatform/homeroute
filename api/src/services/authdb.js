/**
 * Base de données SQLite pour les sessions d'authentification
 */

import Database from 'better-sqlite3';
import path from 'path';
import fs from 'fs';

const DATA_DIR = process.env.AUTH_DATA_DIR || path.join(process.cwd(), 'data');
const DB_PATH = path.join(DATA_DIR, 'auth.db');

let db = null;

export function getDb() {
  if (!db) {
    throw new Error('Database not initialized');
  }
  return db;
}

export async function initDatabase() {
  // Ensure data directory exists
  if (!fs.existsSync(DATA_DIR)) {
    fs.mkdirSync(DATA_DIR, { recursive: true });
  }

  db = new Database(DB_PATH);
  db.pragma('journal_mode = WAL');

  // Sessions table
  db.exec(`
    CREATE TABLE IF NOT EXISTS sessions (
      id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      expires_at INTEGER NOT NULL,
      ip_address TEXT,
      user_agent TEXT,
      last_activity INTEGER NOT NULL,
      remember_me INTEGER DEFAULT 0
    )
  `);

  // Create indexes
  db.exec(`
    CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions(user_id);
    CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions(expires_at);
  `);

  // Cleanup expired sessions periodically
  setInterval(() => {
    const now = Date.now();
    db.prepare('DELETE FROM sessions WHERE expires_at < ?').run(now);
  }, 60 * 1000); // Every minute

  console.log('Auth database initialized:', DB_PATH);
  return db;
}

// Session helpers
export const sessionQueries = {
  create: db => db.prepare(`
    INSERT INTO sessions (id, user_id, created_at, expires_at, ip_address, user_agent, last_activity, remember_me)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
  `),

  get: db => db.prepare('SELECT * FROM sessions WHERE id = ?'),

  getByUserId: db => db.prepare('SELECT * FROM sessions WHERE user_id = ?'),

  updateActivity: db => db.prepare('UPDATE sessions SET last_activity = ? WHERE id = ?'),

  delete: db => db.prepare('DELETE FROM sessions WHERE id = ?'),

  deleteByUserId: db => db.prepare('DELETE FROM sessions WHERE user_id = ?'),

  deleteExpired: db => db.prepare('DELETE FROM sessions WHERE expires_at < ?')
};
