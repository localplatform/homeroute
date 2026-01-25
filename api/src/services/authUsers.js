/**
 * Gestion des utilisateurs (stockage YAML)
 */

import fs from 'fs';
import path from 'path';
import yaml from 'js-yaml';
import argon2 from 'argon2';

const DATA_DIR = process.env.AUTH_DATA_DIR || path.join(process.cwd(), 'data');
const USERS_FILE = path.join(DATA_DIR, 'users.yml');

// Default groups configuration
const DEFAULT_GROUPS = {
  admins: {
    displayName: 'Administrateurs',
    description: 'Acces complet a tous les services',
    policy: 'two_factor'
  },
  power_users: {
    displayName: 'Power Users',
    description: 'Acces etendu avec 2FA',
    policy: 'two_factor'
  },
  users: {
    displayName: 'Utilisateurs',
    description: 'Acces basique avec 1FA',
    policy: 'one_factor'
  }
};

// Load users from YAML file
function loadUsers() {
  try {
    if (!fs.existsSync(USERS_FILE)) {
      // Create default file with empty users
      const dir = path.dirname(USERS_FILE);
      if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
      }
      const defaultData = { users: {} };
      fs.writeFileSync(USERS_FILE, yaml.dump(defaultData), 'utf8');
      return defaultData;
    }
    const content = fs.readFileSync(USERS_FILE, 'utf8');
    return yaml.load(content) || { users: {} };
  } catch (error) {
    console.error('Error loading users file:', error);
    return { users: {} };
  }
}

// Save users to YAML file
function saveUsers(data) {
  try {
    const dir = path.dirname(USERS_FILE);
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true });
    }
    fs.writeFileSync(USERS_FILE, yaml.dump(data, { lineWidth: -1 }), 'utf8');
    return true;
  } catch (error) {
    console.error('Error saving users file:', error);
    return false;
  }
}

// Hash password with Argon2id
export async function hashPassword(password) {
  return argon2.hash(password, {
    type: argon2.argon2id,
    memoryCost: 65536,
    timeCost: 3,
    parallelism: 4
  });
}

// Verify password against hash
export async function verifyPassword(password, hash) {
  try {
    return await argon2.verify(hash, password);
  } catch (error) {
    console.error('Password verification error:', error);
    return false;
  }
}

// Get all users (without passwords)
export function getUsers() {
  const data = loadUsers();
  const users = [];

  for (const [username, userData] of Object.entries(data.users || {})) {
    users.push({
      username,
      displayname: userData.displayname || username,
      email: userData.email || '',
      groups: userData.groups || [],
      disabled: userData.disabled || false,
      created: userData.created,
      last_login: userData.last_login
    });
  }

  return users;
}

// Get single user (without password)
export function getUser(username) {
  const data = loadUsers();
  const userData = data.users?.[username];

  if (!userData) {
    return null;
  }

  return {
    username,
    displayname: userData.displayname || username,
    email: userData.email || '',
    groups: userData.groups || [],
    disabled: userData.disabled || false,
    created: userData.created,
    last_login: userData.last_login
  };
}

// Get user with password hash (for authentication)
export function getUserWithPassword(username) {
  const data = loadUsers();
  const userData = data.users?.[username];

  if (!userData) {
    return null;
  }

  return {
    username,
    displayname: userData.displayname || username,
    email: userData.email || '',
    password: userData.password,
    groups: userData.groups || [],
    disabled: userData.disabled || false
  };
}

// Create new user
export async function createUser(username, password, displayname, email, groups = ['users']) {
  const data = loadUsers();

  if (data.users?.[username]) {
    return { success: false, error: 'Utilisateur deja existant' };
  }

  // Validate username
  if (!/^[a-z0-9_-]{3,32}$/i.test(username)) {
    return { success: false, error: 'Nom d\'utilisateur invalide (3-32 caracteres, lettres, chiffres, _ ou -)' };
  }

  // Validate password strength
  if (password.length < 8) {
    return { success: false, error: 'Le mot de passe doit contenir au moins 8 caracteres' };
  }

  const hashedPassword = await hashPassword(password);

  data.users = data.users || {};
  data.users[username] = {
    displayname: displayname || username,
    email: email || '',
    password: hashedPassword,
    groups: groups,
    disabled: false,
    created: new Date().toISOString()
  };

  if (!saveUsers(data)) {
    return { success: false, error: 'Erreur lors de la sauvegarde' };
  }

  return { success: true, user: getUser(username) };
}

// Update user
export async function updateUser(username, updates) {
  const data = loadUsers();

  if (!data.users?.[username]) {
    return { success: false, error: 'Utilisateur non trouve' };
  }

  const user = data.users[username];

  // Update allowed fields
  if (updates.displayname !== undefined) {
    user.displayname = updates.displayname;
  }
  if (updates.email !== undefined) {
    user.email = updates.email;
  }
  if (updates.groups !== undefined) {
    user.groups = updates.groups;
  }
  if (updates.disabled !== undefined) {
    user.disabled = updates.disabled;
  }

  if (!saveUsers(data)) {
    return { success: false, error: 'Erreur lors de la sauvegarde' };
  }

  return { success: true, user: getUser(username) };
}

// Update user's last login
export function updateLastLogin(username) {
  const data = loadUsers();

  if (!data.users?.[username]) {
    return false;
  }

  data.users[username].last_login = new Date().toISOString();
  return saveUsers(data);
}

// Change password
export async function changePassword(username, newPassword) {
  const data = loadUsers();

  if (!data.users?.[username]) {
    return { success: false, error: 'Utilisateur non trouve' };
  }

  if (newPassword.length < 8) {
    return { success: false, error: 'Le mot de passe doit contenir au moins 8 caracteres' };
  }

  data.users[username].password = await hashPassword(newPassword);

  if (!saveUsers(data)) {
    return { success: false, error: 'Erreur lors de la sauvegarde' };
  }

  return { success: true };
}

// Delete user
export function deleteUser(username) {
  const data = loadUsers();

  if (!data.users?.[username]) {
    return { success: false, error: 'Utilisateur non trouve' };
  }

  delete data.users[username];

  if (!saveUsers(data)) {
    return { success: false, error: 'Erreur lors de la sauvegarde' };
  }

  return { success: true };
}

// Get all groups
export function getGroups() {
  return DEFAULT_GROUPS;
}

// Check if user has admin rights
export function isAdmin(username) {
  const user = getUser(username);
  return user?.groups?.includes('admins') || false;
}

// Get required auth level for user
export function getRequiredAuthLevel(username) {
  const user = getUser(username);
  if (!user) return null;

  // Check groups in order of priority
  if (user.groups.includes('admins') || user.groups.includes('power_users')) {
    return 'two_factor';
  }
  return 'one_factor';
}
