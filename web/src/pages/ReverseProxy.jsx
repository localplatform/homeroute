import { useState, useEffect } from 'react';
import {
  Globe,
  Plus,
  Trash2,
  Settings,
  RefreshCw,
  CheckCircle,
  XCircle,
  Power,
  ExternalLink,
  Lock,
  Server,
  Wifi,
  Pencil,
  Shield,
  Key,
  AlertTriangle,
  User,
  FileCode,
  Copy,
  ChevronDown,
  ChevronUp
} from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import {
  getReverseProxyConfig,
  getReverseProxyStatus,
  getReverseProxyHosts,
  addReverseProxyHost,
  updateReverseProxyHost,
  deleteReverseProxyHost,
  toggleReverseProxyHost,
  updateBaseDomain,
  renewCertificates,
  reloadCaddy,
  getAuthAccounts,
  addAuthAccount,
  updateAuthAccount,
  deleteAuthAccount,
  getCertificatesStatus
} from '../api/client';

function ReverseProxy() {
  const [config, setConfig] = useState(null);
  const [status, setStatus] = useState(null);
  const [hosts, setHosts] = useState([]);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState(null);

  // Tab state
  const [activeTab, setActiveTab] = useState('hosts');

  // Auth accounts state
  const [authAccounts, setAuthAccounts] = useState([]);
  const [certStatuses, setCertStatuses] = useState({});

  // Modal states
  const [showAddModal, setShowAddModal] = useState(false);
  const [showConfigModal, setShowConfigModal] = useState(false);
  const [showEditModal, setShowEditModal] = useState(false);
  const [showDomainRequiredModal, setShowDomainRequiredModal] = useState(false);
  const [showAddAccountModal, setShowAddAccountModal] = useState(false);
  const [showEditAccountModal, setShowEditAccountModal] = useState(false);
  const [editingHost, setEditingHost] = useState(null);
  const [editingAccount, setEditingAccount] = useState(null);

  // Form states
  const [hostType, setHostType] = useState('subdomain');
  const [newHost, setNewHost] = useState({ subdomain: '', customDomain: '', targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false });
  const [editForm, setEditForm] = useState({ targetHost: '', targetPort: '', localOnly: false, requireAuth: false });
  const [configForm, setConfigForm] = useState({ baseDomain: '' });
  const [newAccount, setNewAccount] = useState({ username: '', password: '' });
  const [editAccountForm, setEditAccountForm] = useState({ username: '', password: '' });

  // Action states
  const [saving, setSaving] = useState(false);
  const [renewing, setRenewing] = useState(false);
  const [reloading, setReloading] = useState(false);

  // Instructions section state
  const [showInstructions, setShowInstructions] = useState(false);
  const [instructionsCopied, setInstructionsCopied] = useState(false);

  // Instructions pour Claude Code agents
  const authInstructions = `# Authentification

Cette application utilise un système d'authentification centralisé optionnel.
L'app est accessible à tous, mais peut détecter les utilisateurs connectés.

## Récupérer l'utilisateur connecté

Si un utilisateur est connecté, le reverse proxy ajoute ces headers HTTP :

- \`X-Auth-User\` : nom d'utilisateur (ex: \`john\`)
- \`X-Auth-Email\` : email (ex: \`john@example.com\`)
- \`X-Auth-Name\` : nom affiché (ex: \`John Doe\`)
- \`X-Auth-Groups\` : groupes séparés par virgule (ex: \`users,admins\`)

Si l'utilisateur n'est pas connecté, ces headers sont absents.

## Détecter si connecté

Vérifie la présence du header \`X-Auth-User\` :
- Présent → utilisateur connecté, lire les autres headers
- Absent → utilisateur non connecté (visiteur anonyme)

## Groupes disponibles

- \`admins\` : administrateurs
- \`power_users\` : utilisateurs avancés
- \`users\` : utilisateurs standards

Pour afficher/cacher des fonctionnalités, vérifie si le groupe est dans \`X-Auth-Groups\`.

## Connexion

Pour permettre à l'utilisateur de se connecter, redirige vers :
\`https://auth.mynetwk.biz/login?rd=URL_RETOUR\`

## API de vérification (optionnel)

- \`POST https://proxy.mynetwk.biz/api/authproxy/verify\`
  - Body : \`{ "cookie": "valeur_auth_session" }\`
  - Réponse : \`{ "valid": true, "user": { username, email, displayName, groups } }\`

- \`POST https://proxy.mynetwk.biz/api/authproxy/check-group\`
  - Body : \`{ "cookie": "valeur_auth_session", "groups": ["admins"] }\`
  - Réponse : \`{ "valid": true, "hasAccess": true, "matchedGroups": ["admins"] }\`

## Notes

- L'app reste accessible même sans connexion
- Les headers sont fiables car injectés par le proxy après vérification
- Utilise les headers pour adapter l'UI (afficher bouton admin, etc.)`;

  async function copyInstructions() {
    try {
      await navigator.clipboard.writeText(authInstructions);
      setInstructionsCopied(true);
      setTimeout(() => setInstructionsCopied(false), 2000);
    } catch (err) {
      console.error('Failed to copy:', err);
    }
  }

  useEffect(() => {
    fetchData();
  }, []);

  async function fetchData() {
    try {
      const [configRes, statusRes, hostsRes, accountsRes, certsRes] = await Promise.all([
        getReverseProxyConfig(),
        getReverseProxyStatus(),
        getReverseProxyHosts(),
        getAuthAccounts(),
        getCertificatesStatus()
      ]);

      if (configRes.data.success) {
        setConfig(configRes.data.config);
        setConfigForm({ baseDomain: configRes.data.config.baseDomain || '' });
        // Check if domain is configured
        if (!configRes.data.config.baseDomain) {
          setShowDomainRequiredModal(true);
        }
      }
      if (statusRes.data.success) {
        setStatus(statusRes.data);
      }
      if (hostsRes.data.success) {
        setHosts(hostsRes.data.hosts || []);
      }
      if (accountsRes.data.success) {
        setAuthAccounts(accountsRes.data.accounts || []);
      }
      if (certsRes.data.success) {
        setCertStatuses(certsRes.data.certificates || {});
      }
    } catch (error) {
      console.error('Error:', error);
      setMessage({ type: 'error', text: 'Erreur de chargement' });
    } finally {
      setLoading(false);
    }
  }

  async function handleAddHost() {
    if (!newHost.targetHost || !newHost.targetPort) {
      setMessage({ type: 'error', text: 'Host et port requis' });
      return;
    }
    if (hostType === 'subdomain' && !newHost.subdomain) {
      setMessage({ type: 'error', text: 'Sous-domaine requis' });
      return;
    }
    if (hostType === 'custom' && !newHost.customDomain) {
      setMessage({ type: 'error', text: 'Domaine personnalisé requis' });
      return;
    }

    setSaving(true);
    try {
      const payload = {
        targetHost: newHost.targetHost,
        targetPort: parseInt(newHost.targetPort),
        localOnly: newHost.localOnly,
        requireAuth: newHost.requireAuth
      };
      if (hostType === 'subdomain') {
        payload.subdomain = newHost.subdomain;
      } else {
        payload.customDomain = newHost.customDomain;
      }

      const res = await addReverseProxyHost(payload);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Hôte ajouté' });
        setShowAddModal(false);
        setNewHost({ subdomain: '', customDomain: '', targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur' });
    } finally {
      setSaving(false);
    }
  }

  async function handleToggleHost(hostId, enabled) {
    try {
      const res = await toggleReverseProxyHost(hostId, enabled);
      if (res.data.success) {
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  async function handleDeleteHost(hostId) {
    if (!confirm('Supprimer cet hôte ?')) return;
    try {
      const res = await deleteReverseProxyHost(hostId);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Hôte supprimé' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  function openEditModal(host) {
    setEditingHost(host);
    setEditForm({ targetHost: host.targetHost, targetPort: String(host.targetPort), localOnly: !!host.localOnly, requireAuth: !!host.requireAuth });
    setShowEditModal(true);
  }

  async function handleEditHost() {
    if (!editForm.targetHost || !editForm.targetPort) {
      setMessage({ type: 'error', text: 'Host et port requis' });
      return;
    }
    setSaving(true);
    try {
      const res = await updateReverseProxyHost(editingHost.id, {
        targetHost: editForm.targetHost,
        targetPort: parseInt(editForm.targetPort),
        localOnly: editForm.localOnly,
        requireAuth: editForm.requireAuth
      });
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Hôte modifié' });
        setShowEditModal(false);
        setEditingHost(null);
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur' });
    } finally {
      setSaving(false);
    }
  }

  async function handleSaveConfig() {
    setSaving(true);
    setMessage(null);
    try {
      // Update base domain
      if (configForm.baseDomain !== config?.baseDomain) {
        const domainRes = await updateBaseDomain(configForm.baseDomain);
        if (!domainRes.data.success) {
          setMessage({ type: 'error', text: domainRes.data.error });
          setSaving(false);
          return;
        }
      }

      setMessage({ type: 'success', text: 'Configuration sauvegardée' });
      setShowConfigModal(false);
      setShowDomainRequiredModal(false);
      fetchData();
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur de sauvegarde' });
    } finally {
      setSaving(false);
    }
  }

  // ========== Auth Account Handlers ==========

  async function handleAddAccount() {
    if (!newAccount.username || !newAccount.password) {
      setMessage({ type: 'error', text: 'Nom d\'utilisateur et mot de passe requis' });
      return;
    }
    setSaving(true);
    try {
      const res = await addAuthAccount(newAccount.username, newAccount.password);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Compte créé' });
        setShowAddAccountModal(false);
        setNewAccount({ username: '', password: '' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur' });
    } finally {
      setSaving(false);
    }
  }

  function openEditAccountModal(account) {
    setEditingAccount(account);
    setEditAccountForm({ username: account.username, password: '' });
    setShowEditAccountModal(true);
  }

  async function handleEditAccount() {
    if (!editAccountForm.username) {
      setMessage({ type: 'error', text: 'Nom d\'utilisateur requis' });
      return;
    }
    setSaving(true);
    try {
      const updates = { username: editAccountForm.username };
      if (editAccountForm.password) {
        updates.password = editAccountForm.password;
      }
      const res = await updateAuthAccount(editingAccount.id, updates);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Compte modifié' });
        setShowEditAccountModal(false);
        setEditingAccount(null);
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur' });
    } finally {
      setSaving(false);
    }
  }

  async function handleDeleteAccount(accountId) {
    if (!confirm('Supprimer ce compte ?')) return;
    try {
      const res = await deleteAuthAccount(accountId);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Compte supprimé' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  async function handleRenewCerts() {
    setRenewing(true);
    setMessage(null);
    try {
      const res = await renewCertificates();
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Renouvellement déclenché' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur' });
    } finally {
      setRenewing(false);
    }
  }

  async function handleReload() {
    setReloading(true);
    try {
      const res = await reloadCaddy();
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Caddy rechargé' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur' });
    } finally {
      setReloading(false);
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  const activeHosts = hosts.filter(h => h.enabled).length;
  const caddyRunning = status?.caddy?.running;

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Reverse Proxy</h1>
        <div className="flex gap-2">
          <Button onClick={handleReload} loading={reloading} variant="secondary">
            <RefreshCw className="w-4 h-4" />
            Recharger
          </Button>
          <Button onClick={handleRenewCerts} loading={renewing} variant="secondary">
            <Lock className="w-4 h-4" />
            Renouveler certs
          </Button>
          <Button onClick={() => setShowAddModal(true)} disabled={!config?.baseDomain}>
            <Plus className="w-4 h-4" />
            Ajouter hôte
          </Button>
        </div>
      </div>

      {/* Message */}
      {message && (
        <div className={`p-4 rounded-lg flex items-center gap-2 ${
          message.type === 'success' ? 'bg-green-900/50 text-green-400' : 'bg-red-900/50 text-red-400'
        }`}>
          {message.type === 'success' ? <CheckCircle className="w-5 h-5" /> : <XCircle className="w-5 h-5" />}
          {message.text}
        </div>
      )}

      {/* Status Cards */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        <Card title="Caddy" icon={Server}>
          <div className="flex items-center gap-2">
            <div className={`w-3 h-3 rounded-full ${caddyRunning ? 'bg-green-400' : 'bg-red-400'}`} />
            <span className={caddyRunning ? 'text-green-400' : 'text-red-400'}>
              {caddyRunning ? 'En ligne' : 'Hors ligne'}
            </span>
          </div>
        </Card>

        <Card title="Domaine" icon={Globe}>
          <div className="text-lg font-mono text-blue-400 truncate">
            {config?.baseDomain || 'Non configuré'}
          </div>
        </Card>

        <Card title="Certificats" icon={Lock}>
          <div className="flex items-center gap-2">
            <div className={`w-3 h-3 rounded-full ${activeHosts > 0 ? 'bg-green-400' : 'bg-gray-400'}`} />
            <span className={activeHosts > 0 ? 'text-green-400' : 'text-gray-400'}>
              {activeHosts > 0 ? 'Let\'s Encrypt' : 'Aucun'}
            </span>
          </div>
          <p className="text-xs text-gray-500 mt-1">
            Certificats individuels automatiques
          </p>
        </Card>

        <Card title="Hôtes actifs" icon={Wifi}>
          <div className="text-2xl font-bold text-green-400">
            {activeHosts} / {hosts.length}
          </div>
        </Card>
      </div>

      {/* Tabs */}
      <div className="flex gap-1 border-b border-gray-700">
        <button
          onClick={() => setActiveTab('hosts')}
          className={`flex items-center gap-2 px-4 py-2 text-sm font-medium transition-colors border-b-2 -mb-px ${
            activeTab === 'hosts'
              ? 'border-blue-500 text-blue-400'
              : 'border-transparent text-gray-400 hover:text-gray-300'
          }`}
        >
          <Globe className="w-4 h-4" />
          Hôtes
        </button>
        <button
          onClick={() => setActiveTab('auth')}
          className={`flex items-center gap-2 px-4 py-2 text-sm font-medium transition-colors border-b-2 -mb-px ${
            activeTab === 'auth'
              ? 'border-blue-500 text-blue-400'
              : 'border-transparent text-gray-400 hover:text-gray-300'
          }`}
        >
          <Key className="w-4 h-4" />
          Authentification
          {authAccounts.length > 0 && (
            <span className="bg-gray-700 text-gray-300 text-xs px-1.5 py-0.5 rounded">{authAccounts.length}</span>
          )}
        </button>
        <button
          onClick={() => setActiveTab('integration')}
          className={`flex items-center gap-2 px-4 py-2 text-sm font-medium transition-colors border-b-2 -mb-px ${
            activeTab === 'integration'
              ? 'border-blue-500 text-blue-400'
              : 'border-transparent text-gray-400 hover:text-gray-300'
          }`}
        >
          <FileCode className="w-4 h-4" />
          Intégration
        </button>
      </div>

      {/* Hosts Tab */}
      {activeTab === 'hosts' && (
        <>
          {/* Configuration Card */}
          <Card
            title="Configuration"
            icon={Settings}
            actions={
              <Button onClick={() => setShowConfigModal(true)} variant="secondary" className="text-sm">
                Modifier
              </Button>
            }
          >
            <div className="space-y-3 text-sm">
              <div className="flex justify-between">
                <span className="text-gray-400">Domaine de base</span>
                <span className="font-mono">{config?.baseDomain || '-'}</span>
              </div>
              <div className="flex justify-between">
                <span className="text-gray-400">Dashboard distant</span>
                <span className="font-mono text-blue-400">proxy.{config?.baseDomain || 'domain.com'}</span>
              </div>
              <p className="text-xs text-gray-500 pt-2 border-t border-gray-700">
                Les sous-domaines utilisent ce domaine de base. Les certificats SSL sont obtenus automatiquement via Let&apos;s Encrypt.
              </p>
            </div>
          </Card>

          {/* Hosts Table */}
          <Card title="Hôtes configurés" icon={Globe}>
        {hosts.length === 0 ? (
          <div className="text-center py-8 text-gray-500">
            <Globe className="w-12 h-12 mx-auto mb-2 opacity-50" />
            <p>Aucun hôte configuré</p>
            {!config?.baseDomain && (
              <p className="text-xs mt-2">Configurez d&apos;abord un domaine de base</p>
            )}
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-400 border-b border-gray-700">
                  <th className="pb-2">Domaine</th>
                  <th className="pb-2">Cible</th>
                  <th className="pb-2">SSL</th>
                  <th className="pb-2">Status</th>
                  <th className="pb-2 text-right">Actions</th>
                </tr>
              </thead>
              <tbody>
                {hosts.map(host => {
                  const certStatus = certStatuses[host.id];
                  return (
                  <tr key={host.id} className="border-b border-gray-700/50">
                    <td className="py-3">
                      <div className="flex items-center gap-2 flex-wrap">
                        <ExternalLink className="w-4 h-4 text-gray-500" />
                        <a
                          href={`https://${host.customDomain || `${host.subdomain}.${config?.baseDomain}`}`}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="font-mono text-blue-400 hover:underline"
                        >
                          {host.customDomain || `${host.subdomain}.${config?.baseDomain}`}
                        </a>
                        {host.localOnly && (
                          <span className="flex items-center gap-1 text-xs text-yellow-400 bg-yellow-900/30 px-2 py-0.5 rounded" title="Réseau local uniquement">
                            <Shield className="w-3 h-3" />
                            Local
                          </span>
                        )}
                        {host.requireAuth && (
                          <span className="flex items-center gap-1 text-xs text-purple-400 bg-purple-900/30 px-2 py-0.5 rounded" title="Authentification requise">
                            <Key className="w-3 h-3" />
                            Auth
                          </span>
                        )}
                      </div>
                    </td>
                    <td className="py-3 font-mono text-sm text-gray-300">
                      {host.targetHost}:{host.targetPort}
                    </td>
                    <td className="py-3">
                      {certStatus ? (
                        <span
                          className={`flex items-center gap-1 text-xs px-2 py-0.5 rounded ${
                            certStatus.valid
                              ? certStatus.daysRemaining <= 14
                                ? 'text-yellow-400 bg-yellow-900/30'
                                : 'text-green-400 bg-green-900/30'
                              : 'text-red-400 bg-red-900/30'
                          }`}
                          title={certStatus.valid ? `Expire dans ${certStatus.daysRemaining} jours` : certStatus.error || 'Invalide'}
                        >
                          <Lock className="w-3 h-3" />
                          {certStatus.valid ? (certStatus.daysRemaining <= 14 ? `${certStatus.daysRemaining}j` : 'OK') : 'Erreur'}
                        </span>
                      ) : (
                        <span className="text-xs text-gray-500">-</span>
                      )}
                    </td>
                    <td className="py-3">
                      <button
                        onClick={() => handleToggleHost(host.id, !host.enabled)}
                        className={`p-1.5 rounded transition-colors ${
                          host.enabled
                            ? 'text-green-400 bg-green-900/30 hover:bg-green-900/50'
                            : 'text-gray-500 bg-gray-700/30 hover:bg-gray-700/50'
                        }`}
                        title={host.enabled ? 'Désactiver' : 'Activer'}
                      >
                        <Power className="w-4 h-4" />
                      </button>
                    </td>
                    <td className="py-3 text-right">
                      <div className="flex justify-end gap-1">
                        <button
                          onClick={() => openEditModal(host)}
                          className="text-blue-400 hover:text-blue-300 p-1"
                          title="Modifier"
                        >
                          <Pencil className="w-4 h-4" />
                        </button>
                        <button
                          onClick={() => handleDeleteHost(host.id)}
                          className="text-red-400 hover:text-red-300 p-1"
                          title="Supprimer"
                        >
                          <Trash2 className="w-4 h-4" />
                        </button>
                      </div>
                    </td>
                  </tr>
                );})}
              </tbody>
            </table>
          </div>
        )}
          </Card>
        </>
      )}

      {/* Auth Tab */}
      {activeTab === 'auth' && (
        <Card
          title="Comptes d'authentification"
          icon={Key}
          actions={
            <Button onClick={() => setShowAddAccountModal(true)}>
              <Plus className="w-4 h-4" />
              Ajouter
            </Button>
          }
        >
          {authAccounts.length === 0 ? (
            <div className="text-center py-8 text-gray-500">
              <Key className="w-12 h-12 mx-auto mb-2 opacity-50" />
              <p>Aucun compte configuré</p>
              <p className="text-xs mt-2">Créez un compte pour activer l&apos;authentification sur les hôtes</p>
            </div>
          ) : (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-gray-400 border-b border-gray-700">
                    <th className="pb-2">Utilisateur</th>
                    <th className="pb-2">Créé le</th>
                    <th className="pb-2 text-right">Actions</th>
                  </tr>
                </thead>
                <tbody>
                  {authAccounts.map(account => (
                    <tr key={account.id} className="border-b border-gray-700/50">
                      <td className="py-3">
                        <div className="flex items-center gap-2">
                          <User className="w-4 h-4 text-gray-500" />
                          <span className="font-mono">{account.username}</span>
                        </div>
                      </td>
                      <td className="py-3 text-gray-400">
                        {new Date(account.createdAt).toLocaleDateString('fr-FR')}
                      </td>
                      <td className="py-3 text-right">
                        <div className="flex justify-end gap-1">
                          <button
                            onClick={() => openEditAccountModal(account)}
                            className="text-blue-400 hover:text-blue-300 p-1"
                            title="Modifier"
                          >
                            <Pencil className="w-4 h-4" />
                          </button>
                          <button
                            onClick={() => handleDeleteAccount(account.id)}
                            className="text-red-400 hover:text-red-300 p-1"
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
          )}
          <p className="text-xs text-gray-500 mt-4 pt-4 border-t border-gray-700">
            Ces comptes sont utilisés pour l&apos;authentification HTTP Basic sur les hôtes avec l&apos;option &quot;Authentification requise&quot; activée.
          </p>
        </Card>
      )}

      {/* Integration Tab */}
      {activeTab === 'integration' && (
        <Card
          title="Instructions d'intégration Auth"
          icon={FileCode}
          actions={
            <Button onClick={copyInstructions} variant="secondary" className="text-sm">
              {instructionsCopied ? (
                <>
                  <CheckCircle className="w-4 h-4" />
                  Copié !
                </>
              ) : (
                <>
                  <Copy className="w-4 h-4" />
                  Copier
                </>
              )}
            </Button>
          }
        >
          <div className="space-y-4">
            <p className="text-sm text-gray-400">
              Ces instructions sont destinées aux agents Claude Code qui développent des applications proxifiées.
              Copiez ce texte dans le fichier CLAUDE.md de vos projets.
            </p>

            <div className="bg-gray-900 rounded-lg p-4 border border-gray-700">
              <div className="flex items-center justify-between mb-3">
                <span className="text-xs text-gray-500 font-mono">CLAUDE.md</span>
                <button
                  onClick={() => setShowInstructions(!showInstructions)}
                  className="text-gray-400 hover:text-gray-300 flex items-center gap-1 text-xs"
                >
                  {showInstructions ? (
                    <>
                      <ChevronUp className="w-4 h-4" />
                      Réduire
                    </>
                  ) : (
                    <>
                      <ChevronDown className="w-4 h-4" />
                      Afficher
                    </>
                  )}
                </button>
              </div>

              {showInstructions && (
                <pre className="text-xs text-gray-300 whitespace-pre-wrap font-mono overflow-x-auto max-h-96 overflow-y-auto">
                  {authInstructions}
                </pre>
              )}

              {!showInstructions && (
                <p className="text-xs text-gray-500 italic">
                  Cliquez sur &quot;Afficher&quot; pour voir le contenu complet
                </p>
              )}
            </div>

            <div className="bg-blue-900/20 border border-blue-800 rounded-lg p-4">
              <h4 className="text-sm font-medium text-blue-400 mb-2">Fonctionnement</h4>
              <ul className="text-xs text-gray-400 space-y-1 list-disc list-inside">
                <li>Par défaut, toutes les apps reçoivent les headers <code className="text-blue-400">X-Auth-*</code> si l&apos;utilisateur est connecté</li>
                <li>Si &quot;Authentification requise&quot; est activé, l&apos;accès est bloqué pour les non-connectés</li>
                <li>Les apps peuvent lire les headers pour adapter leur UI selon l&apos;utilisateur</li>
              </ul>
            </div>
          </div>
        </Card>
      )}

      {/* Add Host Modal */}
      {showAddModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-700">
            <h2 className="text-xl font-bold mb-4">Ajouter un hôte</h2>

            <div className="space-y-4">
              {/* Host Type Toggle */}
              <div className="flex gap-2">
                <button
                  onClick={() => setHostType('subdomain')}
                  className={`flex-1 py-2 rounded text-sm font-medium transition-colors ${
                    hostType === 'subdomain' ? 'bg-blue-600 text-white' : 'bg-gray-700 text-gray-300 hover:bg-gray-600'
                  }`}
                >
                  Sous-domaine
                </button>
                <button
                  onClick={() => setHostType('custom')}
                  className={`flex-1 py-2 rounded text-sm font-medium transition-colors ${
                    hostType === 'custom' ? 'bg-blue-600 text-white' : 'bg-gray-700 text-gray-300 hover:bg-gray-600'
                  }`}
                >
                  Domaine perso
                </button>
              </div>

              {/* Subdomain Input */}
              {hostType === 'subdomain' && (
                <div>
                  <label className="block text-sm text-gray-400 mb-1">Sous-domaine</label>
                  <div className="flex items-center">
                    <input
                      type="text"
                      placeholder="app"
                      value={newHost.subdomain}
                      onChange={e => setNewHost({ ...newHost, subdomain: e.target.value })}
                      className="flex-1 px-3 py-2 bg-gray-900 border border-gray-600 rounded-l text-sm focus:outline-none focus:border-blue-500"
                    />
                    <span className="px-3 py-2 bg-gray-700 border border-l-0 border-gray-600 rounded-r text-gray-400 text-sm">
                      .{config?.baseDomain}
                    </span>
                  </div>
                </div>
              )}

              {/* Custom Domain Input */}
              {hostType === 'custom' && (
                <div>
                  <label className="block text-sm text-gray-400 mb-1">Domaine complet</label>
                  <input
                    type="text"
                    placeholder="app.example.com"
                    value={newHost.customDomain}
                    onChange={e => setNewHost({ ...newHost, customDomain: e.target.value })}
                    className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                  />
                </div>
              )}

              {/* Target Host */}
              <div>
                <label className="block text-sm text-gray-400 mb-1">Hôte cible</label>
                <input
                  type="text"
                  placeholder="localhost ou 192.168.1.50"
                  value={newHost.targetHost}
                  onChange={e => setNewHost({ ...newHost, targetHost: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                />
                <div className="flex gap-2 mt-2">
                  <button
                    onClick={() => setNewHost({ ...newHost, targetHost: 'localhost' })}
                    className="text-xs px-2 py-1 bg-gray-700 rounded hover:bg-gray-600"
                  >
                    localhost
                  </button>
                  <button
                    onClick={() => setNewHost({ ...newHost, targetHost: '127.0.0.1' })}
                    className="text-xs px-2 py-1 bg-gray-700 rounded hover:bg-gray-600"
                  >
                    127.0.0.1
                  </button>
                </div>
              </div>

              {/* Target Port */}
              <div>
                <label className="block text-sm text-gray-400 mb-1">Port</label>
                <input
                  type="number"
                  placeholder="3000"
                  min="1"
                  max="65535"
                  value={newHost.targetPort}
                  onChange={e => setNewHost({ ...newHost, targetPort: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                />
              </div>

              {/* Local Only Toggle */}
              <div
                onClick={() => setNewHost({ ...newHost, localOnly: !newHost.localOnly })}
                className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
                  newHost.localOnly
                    ? 'bg-yellow-900/30 border-yellow-600 text-yellow-400'
                    : 'bg-gray-900/50 border-gray-700 text-gray-400 hover:border-gray-600'
                }`}
              >
                <Shield className={`w-5 h-5 ${newHost.localOnly ? 'text-yellow-400' : 'text-gray-500'}`} />
                <div className="flex-1">
                  <div className="font-medium text-sm">Réseau local uniquement</div>
                  <div className="text-xs opacity-75">Bloque l&apos;accès depuis Internet</div>
                </div>
                <div className={`w-10 h-6 rounded-full transition-colors ${newHost.localOnly ? 'bg-yellow-600' : 'bg-gray-600'}`}>
                  <div className={`w-4 h-4 bg-white rounded-full mt-1 transition-transform ${newHost.localOnly ? 'translate-x-5' : 'translate-x-1'}`} />
                </div>
              </div>

              {/* Require Auth Toggle */}
              <div
                onClick={() => setNewHost({ ...newHost, requireAuth: !newHost.requireAuth })}
                className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
                  newHost.requireAuth
                    ? 'bg-purple-900/30 border-purple-600 text-purple-400'
                    : 'bg-gray-900/50 border-gray-700 text-gray-400 hover:border-gray-600'
                }`}
              >
                <Key className={`w-5 h-5 ${newHost.requireAuth ? 'text-purple-400' : 'text-gray-500'}`} />
                <div className="flex-1">
                  <div className="font-medium text-sm">Authentification requise</div>
                  <div className="text-xs opacity-75">Demande un login/mot de passe</div>
                </div>
                <div className={`w-10 h-6 rounded-full transition-colors ${newHost.requireAuth ? 'bg-purple-600' : 'bg-gray-600'}`}>
                  <div className={`w-4 h-4 bg-white rounded-full mt-1 transition-transform ${newHost.requireAuth ? 'translate-x-5' : 'translate-x-1'}`} />
                </div>
              </div>
              {newHost.requireAuth && authAccounts.length === 0 && (
                <div className="flex items-center gap-2 text-xs text-yellow-400 bg-yellow-900/30 p-2 rounded">
                  <AlertTriangle className="w-4 h-4" />
                  Créez au moins un compte dans l&apos;onglet Authentification
                </div>
              )}

              {/* Certificate Info */}
              <div className="text-xs text-gray-500 bg-gray-900/50 rounded p-3">
                <p className="flex items-center gap-1">
                  <Lock className="w-3 h-3" />
                  Certificat SSL automatique via Let&apos;s Encrypt
                </p>
              </div>
            </div>

            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => setShowAddModal(false)}>
                Annuler
              </Button>
              <Button onClick={handleAddHost} loading={saving}>
                Ajouter
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* Config Modal */}
      {showConfigModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-700">
            <h2 className="text-xl font-bold mb-4 flex items-center gap-2">
              <Settings className="w-5 h-5 text-blue-400" />
              Configuration
            </h2>

            <div className="space-y-4">
              {/* Base Domain */}
              <div>
                <label className="block text-sm text-gray-400 mb-1">Domaine de base</label>
                <input
                  type="text"
                  placeholder="example.com"
                  value={configForm.baseDomain}
                  onChange={e => setConfigForm({ ...configForm, baseDomain: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                />
                <p className="text-xs text-gray-500 mt-1">
                  Les sous-domaines seront: app.{configForm.baseDomain || 'example.com'}
                </p>
              </div>

              <div className="text-xs text-gray-500 bg-gray-900/50 rounded p-3">
                <p className="flex items-center gap-1">
                  <Lock className="w-3 h-3" />
                  Les certificats SSL sont obtenus automatiquement via Let&apos;s Encrypt pour chaque domaine.
                </p>
              </div>
            </div>

            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => setShowConfigModal(false)}>
                Annuler
              </Button>
              <Button onClick={handleSaveConfig} loading={saving}>
                Sauvegarder
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* Edit Host Modal */}
      {showEditModal && editingHost && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-700">
            <h2 className="text-xl font-bold mb-4 flex items-center gap-2">
              <Pencil className="w-5 h-5 text-blue-400" />
              Modifier l&apos;hôte
            </h2>

            <div className="space-y-4">
              {/* Domain (read-only) */}
              <div>
                <label className="block text-sm text-gray-400 mb-1">Domaine</label>
                <div className="px-3 py-2 bg-gray-900/50 border border-gray-700 rounded text-sm font-mono text-gray-400">
                  {editingHost.customDomain || `${editingHost.subdomain}.${config?.baseDomain}`}
                </div>
              </div>

              {/* Target Host */}
              <div>
                <label className="block text-sm text-gray-400 mb-1">Hôte cible</label>
                <input
                  type="text"
                  placeholder="localhost ou 192.168.1.50"
                  value={editForm.targetHost}
                  onChange={e => setEditForm({ ...editForm, targetHost: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                />
                <div className="flex gap-2 mt-2">
                  <button
                    onClick={() => setEditForm({ ...editForm, targetHost: 'localhost' })}
                    className="text-xs px-2 py-1 bg-gray-700 rounded hover:bg-gray-600"
                  >
                    localhost
                  </button>
                  <button
                    onClick={() => setEditForm({ ...editForm, targetHost: '127.0.0.1' })}
                    className="text-xs px-2 py-1 bg-gray-700 rounded hover:bg-gray-600"
                  >
                    127.0.0.1
                  </button>
                </div>
              </div>

              {/* Target Port */}
              <div>
                <label className="block text-sm text-gray-400 mb-1">Port</label>
                <input
                  type="number"
                  placeholder="3000"
                  min="1"
                  max="65535"
                  value={editForm.targetPort}
                  onChange={e => setEditForm({ ...editForm, targetPort: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                />
              </div>

              {/* Local Only Toggle */}
              <div
                onClick={() => setEditForm({ ...editForm, localOnly: !editForm.localOnly })}
                className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
                  editForm.localOnly
                    ? 'bg-yellow-900/30 border-yellow-600 text-yellow-400'
                    : 'bg-gray-900/50 border-gray-700 text-gray-400 hover:border-gray-600'
                }`}
              >
                <Shield className={`w-5 h-5 ${editForm.localOnly ? 'text-yellow-400' : 'text-gray-500'}`} />
                <div className="flex-1">
                  <div className="font-medium text-sm">Réseau local uniquement</div>
                  <div className="text-xs opacity-75">Bloque l&apos;accès depuis Internet</div>
                </div>
                <div className={`w-10 h-6 rounded-full transition-colors ${editForm.localOnly ? 'bg-yellow-600' : 'bg-gray-600'}`}>
                  <div className={`w-4 h-4 bg-white rounded-full mt-1 transition-transform ${editForm.localOnly ? 'translate-x-5' : 'translate-x-1'}`} />
                </div>
              </div>

              {/* Require Auth Toggle */}
              <div
                onClick={() => setEditForm({ ...editForm, requireAuth: !editForm.requireAuth })}
                className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
                  editForm.requireAuth
                    ? 'bg-purple-900/30 border-purple-600 text-purple-400'
                    : 'bg-gray-900/50 border-gray-700 text-gray-400 hover:border-gray-600'
                }`}
              >
                <Key className={`w-5 h-5 ${editForm.requireAuth ? 'text-purple-400' : 'text-gray-500'}`} />
                <div className="flex-1">
                  <div className="font-medium text-sm">Authentification requise</div>
                  <div className="text-xs opacity-75">Demande un login/mot de passe</div>
                </div>
                <div className={`w-10 h-6 rounded-full transition-colors ${editForm.requireAuth ? 'bg-purple-600' : 'bg-gray-600'}`}>
                  <div className={`w-4 h-4 bg-white rounded-full mt-1 transition-transform ${editForm.requireAuth ? 'translate-x-5' : 'translate-x-1'}`} />
                </div>
              </div>
              {editForm.requireAuth && authAccounts.length === 0 && (
                <div className="flex items-center gap-2 text-xs text-yellow-400 bg-yellow-900/30 p-2 rounded">
                  <AlertTriangle className="w-4 h-4" />
                  Créez au moins un compte dans l&apos;onglet Authentification
                </div>
              )}
            </div>

            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => { setShowEditModal(false); setEditingHost(null); }}>
                Annuler
              </Button>
              <Button onClick={handleEditHost} loading={saving}>
                Sauvegarder
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* Add Account Modal */}
      {showAddAccountModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-700">
            <h2 className="text-xl font-bold mb-4 flex items-center gap-2">
              <User className="w-5 h-5 text-purple-400" />
              Nouveau compte
            </h2>

            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Nom d&apos;utilisateur</label>
                <input
                  type="text"
                  placeholder="admin"
                  value={newAccount.username}
                  onChange={e => setNewAccount({ ...newAccount, username: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                />
              </div>

              <div>
                <label className="block text-sm text-gray-400 mb-1">Mot de passe</label>
                <input
                  type="password"
                  placeholder="••••••••"
                  value={newAccount.password}
                  onChange={e => setNewAccount({ ...newAccount, password: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                />
              </div>

              <div className="text-xs text-gray-500 bg-gray-900/50 rounded p-3">
                <p className="flex items-center gap-1">
                  <Key className="w-3 h-3" />
                  Ce compte sera utilisé pour l&apos;authentification HTTP Basic
                </p>
              </div>
            </div>

            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => { setShowAddAccountModal(false); setNewAccount({ username: '', password: '' }); }}>
                Annuler
              </Button>
              <Button onClick={handleAddAccount} loading={saving}>
                Créer
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* Edit Account Modal */}
      {showEditAccountModal && editingAccount && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-700">
            <h2 className="text-xl font-bold mb-4 flex items-center gap-2">
              <Pencil className="w-5 h-5 text-blue-400" />
              Modifier le compte
            </h2>

            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Nom d&apos;utilisateur</label>
                <input
                  type="text"
                  placeholder="admin"
                  value={editAccountForm.username}
                  onChange={e => setEditAccountForm({ ...editAccountForm, username: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                />
              </div>

              <div>
                <label className="block text-sm text-gray-400 mb-1">Nouveau mot de passe (optionnel)</label>
                <input
                  type="password"
                  placeholder="Laisser vide pour conserver l'actuel"
                  value={editAccountForm.password}
                  onChange={e => setEditAccountForm({ ...editAccountForm, password: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                />
                <p className="text-xs text-gray-500 mt-1">
                  Laissez vide pour conserver le mot de passe actuel
                </p>
              </div>
            </div>

            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => { setShowEditAccountModal(false); setEditingAccount(null); }}>
                Annuler
              </Button>
              <Button onClick={handleEditAccount} loading={saving}>
                Sauvegarder
              </Button>
            </div>
          </div>
        </div>
      )}

      {/* Domain Required Modal */}
      {showDomainRequiredModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-700">
            <h2 className="text-xl font-bold mb-4 flex items-center gap-2">
              <AlertTriangle className="w-5 h-5 text-yellow-400" />
              Configuration requise
            </h2>

            <p className="text-gray-300 mb-4">
              Veuillez configurer un domaine de base pour utiliser le reverse proxy.
            </p>

            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Domaine de base</label>
                <input
                  type="text"
                  placeholder="example.com"
                  value={configForm.baseDomain}
                  onChange={e => setConfigForm({ ...configForm, baseDomain: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
                />
                <p className="text-xs text-gray-500 mt-1">
                  Les sous-domaines seront: app.{configForm.baseDomain || 'example.com'}
                </p>
              </div>

              <div className="bg-blue-900/30 border border-blue-700 rounded p-3">
                <p className="text-sm text-blue-300">
                  Une route système <span className="font-mono">proxy.{configForm.baseDomain || 'votre-domaine.com'}</span> sera créée automatiquement pour accéder au dashboard à distance.
                </p>
              </div>
            </div>

            <div className="flex justify-end gap-2 mt-6">
              <Button onClick={handleSaveConfig} loading={saving} disabled={!configForm.baseDomain}>
                Configurer
              </Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default ReverseProxy;
