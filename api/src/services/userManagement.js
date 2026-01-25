/**
 * Service de gestion des utilisateurs
 *
 * Interface unifiee pour la gestion des utilisateurs locaux
 */

import { existsSync } from 'fs';
import path from 'path';
import {
  getUsers as getUsersBase,
  getUser as getUserBase,
  createUser as createUserBase,
  updateUser as updateUserBase,
  changePassword as changePasswordBase,
  deleteUser as deleteUserBase,
  getGroups as getGroupsBase
} from './authUsers.js';

const DATA_DIR = process.env.AUTH_DATA_DIR || path.join(process.cwd(), 'data');
const BASE_DOMAIN = process.env.BASE_DOMAIN || 'localhost';

// Groupes predefinis avec leurs descriptions
const PREDEFINED_GROUPS = {
  admins: {
    name: 'admins',
    displayName: 'Administrateurs',
    description: 'Acces complet, gestion des utilisateurs et services'
  },
  power_users: {
    name: 'power_users',
    displayName: 'Power Users',
    description: 'Acces etendu aux services'
  },
  users: {
    name: 'users',
    displayName: 'Utilisateurs',
    description: 'Acces basique aux services'
  }
};

// ========== Status Service Auth ==========

export async function getAutheliaStatus() {
  try {
    const usersFileExists = existsSync(path.join(DATA_DIR, 'users.yml'));
    const dbExists = existsSync(path.join(DATA_DIR, 'auth.db'));

    return {
      success: true,
      status: 'running',
      healthy: true,
      configExists: usersFileExists,
      usersExists: usersFileExists,
      dbExists,
      service: 'local-auth',
      integrated: true
    };
  } catch (error) {
    return {
      success: false,
      status: 'error',
      healthy: false,
      error: error.message
    };
  }
}

// ========== CRUD Users ==========

export async function getUsers() {
  try {
    const users = getUsersBase();
    return { success: true, users };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function getUser(username) {
  try {
    const user = getUserBase(username);
    if (!user) {
      return { success: false, error: 'Utilisateur non trouve' };
    }
    return { success: true, user };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

export async function createUser(username, password, displayname, email, groups = ['users']) {
  // Validation
  if (!username || username.length < 3) {
    return { success: false, error: 'Le nom d\'utilisateur doit contenir au moins 3 caracteres' };
  }

  if (!password || password.length < 8) {
    return { success: false, error: 'Le mot de passe doit contenir au moins 8 caracteres' };
  }

  // Valider le format username
  if (!/^[a-zA-Z0-9_-]+$/.test(username)) {
    return { success: false, error: 'Le nom d\'utilisateur ne peut contenir que des lettres, chiffres, underscores et tirets' };
  }

  // Valider les groupes
  const validGroups = groups.filter(g => PREDEFINED_GROUPS[g]);
  if (validGroups.length === 0) {
    validGroups.push('users');
  }

  return createUserBase(username.toLowerCase(), password, displayname, email, validGroups);
}

export async function updateUser(username, updates) {
  // Valider les groupes si presents
  if (updates.groups) {
    const validGroups = updates.groups.filter(g => PREDEFINED_GROUPS[g]);
    if (validGroups.length > 0) {
      updates.groups = validGroups;
    } else {
      delete updates.groups;
    }
  }

  return updateUserBase(username.toLowerCase(), updates);
}

export async function changePassword(username, newPassword) {
  if (!newPassword || newPassword.length < 8) {
    return { success: false, error: 'Le mot de passe doit contenir au moins 8 caracteres' };
  }

  return changePasswordBase(username.toLowerCase(), newPassword);
}

export async function deleteUser(username) {
  // Verifier qu'on ne supprime pas le dernier admin
  const users = getUsersBase();
  const admins = users.filter(u => u.groups?.includes('admins') && !u.disabled);
  const userToDelete = getUserBase(username.toLowerCase());

  if (userToDelete?.groups?.includes('admins') && admins.length <= 1) {
    return { success: false, error: 'Impossible de supprimer le dernier administrateur' };
  }

  return deleteUserBase(username.toLowerCase());
}

// ========== Groupes ==========

export async function getGroups() {
  try {
    const users = getUsersBase();

    // Compter les membres de chaque groupe
    const groupCounts = {};
    for (const group of Object.keys(PREDEFINED_GROUPS)) {
      groupCounts[group] = 0;
    }

    for (const user of users) {
      for (const group of user.groups || []) {
        if (groupCounts[group] !== undefined) {
          groupCounts[group]++;
        }
      }
    }

    const groups = Object.values(PREDEFINED_GROUPS).map(group => ({
      ...group,
      memberCount: groupCounts[group.name] || 0
    }));

    return { success: true, groups };
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ========== Bootstrap ==========

export async function bootstrapAdmin(password) {
  try {
    const users = getUsersBase();

    // Verifier si un admin existe deja
    const existingAdmins = users.filter(u => u.groups?.includes('admins'));

    if (existingAdmins.length > 0) {
      return { success: false, error: 'Un administrateur existe deja' };
    }

    // Creer l'admin par defaut
    return createUserBase(
      'admin',
      password,
      'Administrateur',
      `admin@${BASE_DOMAIN}`,
      ['admins']
    );
  } catch (error) {
    return { success: false, error: error.message };
  }
}

// ========== Instructions ==========

export function getInstallationInstructions() {
  return {
    success: true,
    instructions: `# Service d'authentification integre

## Architecture

L'authentification est maintenant integree directement dans l'API principale.
Plus besoin de service externe.

## Donnees

- Utilisateurs: ${DATA_DIR}/users.yml
- Sessions: ${DATA_DIR}/auth.db

## Premier demarrage

Creez un compte administrateur via l'interface ou l'API:

\`\`\`bash
curl -X POST http://localhost:4000/api/users/authelia/bootstrap \\
  -H "Content-Type: application/json" \\
  -d '{"password": "votre_mot_de_passe"}'
\`\`\`
`
  };
}
