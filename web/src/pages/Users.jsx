import { useState, useEffect } from 'react';
import {
  Users as UsersIcon,
  Plus,
  Trash2,
  Pencil,
  Key,
  Shield,
  ShieldCheck,
  ShieldOff,
  RefreshCw,
  AlertCircle,
  CheckCircle,
  XCircle,
  ExternalLink,
  Copy,
  Eye,
  EyeOff
} from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import GroupBadge from '../components/GroupBadge';
import UserModal from '../components/UserModal';
import {
  getAutheliaStatus,
  getAutheliaInstallInstructions,
  bootstrapAdmin,
  getUsers,
  createUser,
  updateUser,
  deleteUser,
  changeUserPassword,
  getUserGroups
} from '../api/client';

function Users() {
  const [authStatus, setAuthStatus] = useState(null);
  const [users, setUsers] = useState([]);
  const [groups, setGroups] = useState([]);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState(null);
  const [activeTab, setActiveTab] = useState('users');

  // Modal states
  const [showUserModal, setShowUserModal] = useState(false);
  const [showPasswordModal, setShowPasswordModal] = useState(false);
  const [showInstallModal, setShowInstallModal] = useState(false);
  const [showBootstrapModal, setShowBootstrapModal] = useState(false);
  const [editingUser, setEditingUser] = useState(null);
  const [selectedUser, setSelectedUser] = useState(null);

  // Form states
  const [newPassword, setNewPassword] = useState({ password: '', confirmPassword: '' });
  const [bootstrapPassword, setBootstrapPassword] = useState({ password: '', confirmPassword: '' });
  const [showPassword, setShowPassword] = useState(false);
  const [installInstructions, setInstallInstructions] = useState('');

  // Action states
  const [saving, setSaving] = useState(false);
  const [deleting, setDeleting] = useState(null);

  useEffect(() => {
    fetchData();
  }, []);

  async function fetchData() {
    try {
      setLoading(true);
      const [statusRes, usersRes, groupsRes] = await Promise.all([
        getAutheliaStatus(),
        getUsers().catch(() => ({ data: { success: false } })),
        getUserGroups().catch(() => ({ data: { success: false } }))
      ]);

      if (statusRes.data.success) {
        setAuthStatus(statusRes.data);
      }

      if (usersRes.data?.success) {
        setUsers(usersRes.data.users || []);
      }

      if (groupsRes.data?.success) {
        setGroups(groupsRes.data.groups || []);
      }
    } catch (error) {
      console.error('Error:', error);
      setMessage({ type: 'error', text: 'Erreur de chargement' });
    } finally {
      setLoading(false);
    }
  }

  async function fetchInstallInstructions() {
    try {
      const res = await getAutheliaInstallInstructions();
      if (res.data.success) {
        setInstallInstructions(res.data.instructions);
        setShowInstallModal(true);
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur de chargement des instructions' });
    }
  }

  // ========== User Handlers ==========

  async function handleCreateUser(data) {
    try {
      setSaving(true);
      const res = await createUser(data);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Utilisateur créé avec succès' });
        setShowUserModal(false);
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error || 'Erreur lors de la création' });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur lors de la création' });
    } finally {
      setSaving(false);
    }
  }

  async function handleUpdateUser(data) {
    if (!editingUser) return;

    try {
      setSaving(true);
      const res = await updateUser(editingUser.username, data);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Utilisateur modifié avec succès' });
        setShowUserModal(false);
        setEditingUser(null);
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error || 'Erreur lors de la modification' });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur lors de la modification' });
    } finally {
      setSaving(false);
    }
  }

  async function handleDeleteUser(username) {
    if (!confirm(`Supprimer l'utilisateur "${username}" ?`)) return;

    try {
      setDeleting(username);
      const res = await deleteUser(username);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Utilisateur supprimé' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error || 'Erreur lors de la suppression' });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur lors de la suppression' });
    } finally {
      setDeleting(null);
    }
  }

  async function handleChangePassword() {
    if (!selectedUser) return;

    if (newPassword.password.length < 8) {
      setMessage({ type: 'error', text: 'Le mot de passe doit contenir au moins 8 caractères' });
      return;
    }

    if (newPassword.password !== newPassword.confirmPassword) {
      setMessage({ type: 'error', text: 'Les mots de passe ne correspondent pas' });
      return;
    }

    try {
      setSaving(true);
      const res = await changeUserPassword(selectedUser.username, newPassword.password);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Mot de passe modifié avec succès' });
        setShowPasswordModal(false);
        setNewPassword({ password: '', confirmPassword: '' });
        setSelectedUser(null);
      } else {
        setMessage({ type: 'error', text: res.data.error || 'Erreur lors de la modification' });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur lors de la modification' });
    } finally {
      setSaving(false);
    }
  }

  async function handleBootstrapAdmin() {
    if (bootstrapPassword.password.length < 8) {
      setMessage({ type: 'error', text: 'Le mot de passe doit contenir au moins 8 caractères' });
      return;
    }

    if (bootstrapPassword.password !== bootstrapPassword.confirmPassword) {
      setMessage({ type: 'error', text: 'Les mots de passe ne correspondent pas' });
      return;
    }

    try {
      setSaving(true);
      const res = await bootstrapAdmin(bootstrapPassword.password);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Administrateur créé avec succès' });
        setShowBootstrapModal(false);
        setBootstrapPassword({ password: '', confirmPassword: '' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error || 'Erreur lors de la création' });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur lors de la création' });
    } finally {
      setSaving(false);
    }
  }

  // ========== Render Helpers ==========

  function renderStatusBadge() {
    if (!authStatus) return null;

    const statusConfig = {
      running: { icon: CheckCircle, text: 'En ligne', color: 'text-green-400' },
      stopped: { icon: XCircle, text: 'Arrêté', color: 'text-red-400' },
      not_installed: { icon: AlertCircle, text: 'Non installé', color: 'text-yellow-400' },
      unhealthy: { icon: AlertCircle, text: 'Problème', color: 'text-orange-400' }
    };

    const config = statusConfig[authStatus.status] || statusConfig.not_installed;
    const Icon = config.icon;

    return (
      <span className={`flex items-center gap-1 text-sm ${config.color}`}>
        <Icon className="w-4 h-4" />
        {config.text}
      </span>
    );
  }

  function renderNotInstalled() {
    return (
      <div className="text-center py-12">
        <ShieldOff className="w-16 h-16 text-gray-500 mx-auto mb-4" />
        <h3 className="text-xl font-semibold mb-2">Service d'authentification non installé</h3>
        <p className="text-gray-400 mb-6 max-w-md mx-auto">
          Le service d'authentification gère les utilisateurs et les groupes.
        </p>
        <Button variant="primary" onClick={fetchInstallInstructions}>
          <ExternalLink className="w-4 h-4" />
          Voir les instructions d'installation
        </Button>
      </div>
    );
  }

  function renderStopped() {
    return (
      <div className="text-center py-12">
        <XCircle className="w-16 h-16 text-red-500 mx-auto mb-4" />
        <h3 className="text-xl font-semibold mb-2">Service d'authentification arrêté</h3>
        <p className="text-gray-400 mb-6">
          Le service est configuré mais ne répond pas. Démarrez-le avec:
        </p>
        <code className="bg-gray-700 px-4 py-2 rounded block max-w-md mx-auto mb-6">
          cd /ssd_pool/auth-service && pm2 start ecosystem.config.cjs
        </code>
        <Button variant="secondary" onClick={fetchData}>
          <RefreshCw className="w-4 h-4" />
          Rafraîchir
        </Button>
      </div>
    );
  }

  function renderUsersTab() {
    if (users.length === 0) {
      return (
        <div className="text-center py-8">
          <UsersIcon className="w-12 h-12 text-gray-500 mx-auto mb-3" />
          <p className="text-gray-400 mb-4">Aucun utilisateur configuré</p>
          <Button variant="primary" onClick={() => setShowBootstrapModal(true)}>
            <Plus className="w-4 h-4" />
            Créer l'administrateur initial
          </Button>
        </div>
      );
    }

    return (
      <div className="overflow-x-auto">
        <table className="w-full">
          <thead>
            <tr className="text-left text-gray-400 text-sm border-b border-gray-700">
              <th className="pb-3 font-medium">Utilisateur</th>
              <th className="pb-3 font-medium">Nom affiché</th>
              <th className="pb-3 font-medium">Groupes</th>
              <th className="pb-3 font-medium">Status</th>
              <th className="pb-3 font-medium text-right">Actions</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-700">
            {users.map(user => (
              <tr key={user.username} className="hover:bg-gray-700/50">
                <td className="py-3">
                  <div className="font-medium">{user.username}</div>
                  <div className="text-xs text-gray-500">{user.email}</div>
                </td>
                <td className="py-3 text-gray-300">
                  {user.displayname || user.username}
                </td>
                <td className="py-3">
                  <div className="flex flex-wrap gap-1">
                    {(user.groups || []).map(group => (
                      <GroupBadge key={group} group={group} size="xs" />
                    ))}
                  </div>
                </td>
                <td className="py-3">
                  {user.disabled ? (
                    <span className="text-red-400 text-sm flex items-center gap-1">
                      <XCircle className="w-3 h-3" /> Désactivé
                    </span>
                  ) : (
                    <span className="text-green-400 text-sm flex items-center gap-1">
                      <CheckCircle className="w-3 h-3" /> Actif
                    </span>
                  )}
                </td>
                <td className="py-3 text-right">
                  <div className="flex justify-end gap-1">
                    <button
                      onClick={() => {
                        setSelectedUser(user);
                        setShowPasswordModal(true);
                      }}
                      className="p-2 text-gray-400 hover:text-yellow-400 hover:bg-gray-700 rounded"
                      title="Changer le mot de passe"
                    >
                      <Key className="w-4 h-4" />
                    </button>
                    <button
                      onClick={() => {
                        setEditingUser(user);
                        setShowUserModal(true);
                      }}
                      className="p-2 text-gray-400 hover:text-blue-400 hover:bg-gray-700 rounded"
                      title="Modifier"
                    >
                      <Pencil className="w-4 h-4" />
                    </button>
                    <button
                      onClick={() => handleDeleteUser(user.username)}
                      disabled={deleting === user.username}
                      className="p-2 text-gray-400 hover:text-red-400 hover:bg-gray-700 rounded disabled:opacity-50"
                      title="Supprimer"
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  }

  function renderGroupsTab() {
    return (
      <div className="grid gap-4 md:grid-cols-2">
        {groups.map(group => (
          <div
            key={group.name}
            className="bg-gray-700/50 rounded-lg p-4 border border-gray-600"
          >
            <div className="flex items-center gap-2 mb-2">
              <GroupBadge group={group.name} size="md" />
            </div>
            <p className="text-gray-400 text-sm mb-3">{group.description}</p>
            <div className="text-sm text-gray-500">
              {group.memberCount} membre(s)
            </div>
          </div>
        ))}
      </div>
    );
  }


  // ========== Main Render ==========

  if (loading) {
    return (
      <div className="flex items-center justify-center h-64">
        <RefreshCw className="w-8 h-8 animate-spin text-blue-400" />
      </div>
    );
  }

  const isAuthAvailable = authStatus?.status === 'running';

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            <ShieldCheck className="w-7 h-7 text-blue-400" />
            Utilisateurs
          </h1>
          <p className="text-gray-400 text-sm mt-1">
            Gestion des utilisateurs, groupes et authentification
          </p>
        </div>
        <div className="flex items-center gap-3">
          {renderStatusBadge()}
          <Button variant="secondary" onClick={fetchData}>
            <RefreshCw className="w-4 h-4" />
          </Button>
        </div>
      </div>

      {/* Message */}
      {message && (
        <div className={`p-4 rounded-lg flex items-center gap-2 ${
          message.type === 'success' ? 'bg-green-500/20 text-green-400 border border-green-500/30' :
          'bg-red-500/20 text-red-400 border border-red-500/30'
        }`}>
          {message.type === 'success' ? <CheckCircle className="w-5 h-5" /> : <AlertCircle className="w-5 h-5" />}
          {message.text}
          <button onClick={() => setMessage(null)} className="ml-auto text-current opacity-70 hover:opacity-100">
            ×
          </button>
        </div>
      )}

      {/* Content */}
      {authStatus?.status === 'not_installed' ? (
        <Card title="Installation requise" icon={Shield}>
          {renderNotInstalled()}
        </Card>
      ) : authStatus?.status === 'stopped' ? (
        <Card title="Service arrêté" icon={Shield}>
          {renderStopped()}
        </Card>
      ) : (
        <Card
          title="Gestion des utilisateurs"
          icon={UsersIcon}
          actions={
            isAuthAvailable && users.length > 0 && (
              <Button
                variant="primary"
                onClick={() => {
                  setEditingUser(null);
                  setShowUserModal(true);
                }}
              >
                <Plus className="w-4 h-4" />
                Ajouter
              </Button>
            )
          }
        >
          {/* Tabs */}
          <div className="flex gap-1 mb-4 border-b border-gray-700">
            {['users', 'groups'].map(tab => (
              <button
                key={tab}
                onClick={() => setActiveTab(tab)}
                className={`px-4 py-2 text-sm font-medium transition-colors border-b-2 -mb-px ${
                  activeTab === tab
                    ? 'border-blue-400 text-blue-400'
                    : 'border-transparent text-gray-400 hover:text-white'
                }`}
              >
                {tab === 'users' ? 'Utilisateurs' : 'Groupes'}
              </button>
            ))}
          </div>

          {/* Tab Content */}
          {activeTab === 'users' && renderUsersTab()}
          {activeTab === 'groups' && renderGroupsTab()}
        </Card>
      )}

      {/* User Modal */}
      <UserModal
        isOpen={showUserModal}
        onClose={() => {
          setShowUserModal(false);
          setEditingUser(null);
        }}
        onSave={editingUser ? handleUpdateUser : handleCreateUser}
        user={editingUser}
        saving={saving}
      />

      {/* Password Modal */}
      {showPasswordModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-gray-800 rounded-lg border border-gray-700 w-full max-w-md mx-4 p-4">
            <h3 className="font-semibold mb-4">
              Changer le mot de passe de {selectedUser?.username}
            </h3>
            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Nouveau mot de passe</label>
                <div className="relative">
                  <input
                    type={showPassword ? 'text' : 'password'}
                    value={newPassword.password}
                    onChange={e => setNewPassword({ ...newPassword, password: e.target.value })}
                    className="w-full bg-gray-700 border border-gray-600 rounded-lg px-3 py-2 text-white pr-10"
                    placeholder="Minimum 8 caractères"
                  />
                  <button
                    type="button"
                    onClick={() => setShowPassword(!showPassword)}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-white"
                  >
                    {showPassword ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                  </button>
                </div>
              </div>
              <div>
                <label className="block text-sm text-gray-400 mb-1">Confirmer</label>
                <input
                  type={showPassword ? 'text' : 'password'}
                  value={newPassword.confirmPassword}
                  onChange={e => setNewPassword({ ...newPassword, confirmPassword: e.target.value })}
                  className="w-full bg-gray-700 border border-gray-600 rounded-lg px-3 py-2 text-white"
                  placeholder="Confirmer le mot de passe"
                />
              </div>
              <div className="flex gap-2">
                <Button
                  variant="secondary"
                  onClick={() => {
                    setShowPasswordModal(false);
                    setNewPassword({ password: '', confirmPassword: '' });
                    setSelectedUser(null);
                  }}
                  className="flex-1"
                >
                  Annuler
                </Button>
                <Button variant="primary" onClick={handleChangePassword} loading={saving} className="flex-1">
                  Changer
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Bootstrap Modal */}
      {showBootstrapModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-gray-800 rounded-lg border border-gray-700 w-full max-w-md mx-4 p-4">
            <h3 className="font-semibold mb-2">Créer l'administrateur initial</h3>
            <p className="text-gray-400 text-sm mb-4">
              Cet utilisateur aura accès complet à la gestion des utilisateurs et des services.
            </p>
            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Mot de passe admin</label>
                <div className="relative">
                  <input
                    type={showPassword ? 'text' : 'password'}
                    value={bootstrapPassword.password}
                    onChange={e => setBootstrapPassword({ ...bootstrapPassword, password: e.target.value })}
                    className="w-full bg-gray-700 border border-gray-600 rounded-lg px-3 py-2 text-white pr-10"
                    placeholder="Minimum 8 caractères"
                  />
                  <button
                    type="button"
                    onClick={() => setShowPassword(!showPassword)}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-white"
                  >
                    {showPassword ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                  </button>
                </div>
              </div>
              <div>
                <label className="block text-sm text-gray-400 mb-1">Confirmer</label>
                <input
                  type={showPassword ? 'text' : 'password'}
                  value={bootstrapPassword.confirmPassword}
                  onChange={e => setBootstrapPassword({ ...bootstrapPassword, confirmPassword: e.target.value })}
                  className="w-full bg-gray-700 border border-gray-600 rounded-lg px-3 py-2 text-white"
                  placeholder="Confirmer le mot de passe"
                />
              </div>
              <div className="flex gap-2">
                <Button
                  variant="secondary"
                  onClick={() => {
                    setShowBootstrapModal(false);
                    setBootstrapPassword({ password: '', confirmPassword: '' });
                  }}
                  className="flex-1"
                >
                  Annuler
                </Button>
                <Button variant="primary" onClick={handleBootstrapAdmin} loading={saving} className="flex-1">
                  Créer l'admin
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* Install Instructions Modal */}
      {showInstallModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg border border-gray-700 w-full max-w-3xl max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between p-4 border-b border-gray-700">
              <h3 className="font-semibold">Instructions de démarrage</h3>
              <button
                onClick={() => setShowInstallModal(false)}
                className="text-gray-400 hover:text-white"
              >
                ×
              </button>
            </div>
            <div className="p-4 overflow-auto flex-1">
              <pre className="bg-gray-900 p-4 rounded-lg text-sm text-gray-300 whitespace-pre-wrap font-mono">
                {installInstructions}
              </pre>
            </div>
            <div className="p-4 border-t border-gray-700 flex justify-between">
              <Button
                variant="secondary"
                onClick={() => {
                  navigator.clipboard.writeText(installInstructions);
                  setMessage({ type: 'success', text: 'Instructions copiées' });
                }}
              >
                <Copy className="w-4 h-4" />
                Copier
              </Button>
              <Button variant="primary" onClick={() => setShowInstallModal(false)}>
                Fermer
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default Users;
