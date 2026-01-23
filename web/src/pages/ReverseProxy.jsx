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
  FileCode,
  Copy,
  ChevronDown,
  ChevronUp,
  Layers,
  Cloud,
  ArrowRightLeft
} from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import ApplicationCard from '../components/ApplicationCard';
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
  getCertificatesStatus,
  getReverseProxyEnvironments,
  getReverseProxyApplications,
  addReverseProxyApplication,
  updateReverseProxyApplication,
  deleteReverseProxyApplication,
  toggleReverseProxyApplication,
  getMigrationSuggestions,
  executeMigration,
  getCloudflareConfig,
  updateCloudflareConfig
} from '../api/client';

function ReverseProxy() {
  const [config, setConfig] = useState(null);
  const [status, setStatus] = useState(null);
  const [hosts, setHosts] = useState([]);
  const [applications, setApplications] = useState([]);
  const [environments, setEnvironments] = useState([]);
  const [cloudflare, setCloudflare] = useState(null);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState(null);

  // Tab state
  const [activeTab, setActiveTab] = useState('applications');

  // Certificate statuses
  const [certStatuses, setCertStatuses] = useState({});

  // Migration state
  const [migrationSuggestions, setMigrationSuggestions] = useState(null);
  const [showMigrationBanner, setShowMigrationBanner] = useState(false);

  // Modal states
  const [showAddModal, setShowAddModal] = useState(false);
  const [showAddAppModal, setShowAddAppModal] = useState(false);
  const [showConfigModal, setShowConfigModal] = useState(false);
  const [showEditModal, setShowEditModal] = useState(false);
  const [showEditAppModal, setShowEditAppModal] = useState(false);
  const [showDomainRequiredModal, setShowDomainRequiredModal] = useState(false);
  const [showMigrationModal, setShowMigrationModal] = useState(false);
  const [editingHost, setEditingHost] = useState(null);
  const [editingApp, setEditingApp] = useState(null);

  // Form states
  const [hostType, setHostType] = useState('subdomain');
  const [newHost, setNewHost] = useState({ subdomain: '', customDomain: '', targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false });
  const [newApp, setNewApp] = useState({
    name: '',
    slug: '',
    endpoints: {
      prod: {
        enabled: true,
        frontend: { targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false },
        hasApi: false,
        api: { targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false }
      }
    }
  });
  const [editForm, setEditForm] = useState({ targetHost: '', targetPort: '', localOnly: false, requireAuth: false });
  const [editAppForm, setEditAppForm] = useState(null);
  const [configForm, setConfigForm] = useState({ baseDomain: '' });

  // Action states
  const [saving, setSaving] = useState(false);
  const [renewing, setRenewing] = useState(false);
  const [reloading, setReloading] = useState(false);
  const [migrating, setMigrating] = useState(false);

  // Integration section state
  const [showIntegration, setShowIntegration] = useState(false);
  const [instructionsCopied, setInstructionsCopied] = useState(false);

  const authInstructions = `# Authentification

Le cookie \`auth_session\` est partage sur \`*.mynetwk.biz\`.

## Connexion / Deconnexion

Pour connecter un utilisateur, redirige vers :
\`https://auth.mynetwk.biz/login?rd=URL_RETOUR\`

Pour deconnecter un utilisateur, redirige vers :
\`https://auth.mynetwk.biz/logout?rd=URL_RETOUR\`

## Verifier l'utilisateur connecte

### GET https://auth.mynetwk.biz/api/auth/me

**Entree :** Cookie \`auth_session\` (envoye automatiquement)

**Sortie (connecte) :**
- \`success\`: true
- \`user.username\`: nom d'utilisateur
- \`user.displayname\`: nom affiche
- \`user.email\`: email
- \`user.groups\`: liste des groupes (ex: ["users", "admins"])

**Sortie (non connecte) :** \`success\`: false

---

### GET https://auth.mynetwk.biz/api/auth/check

Verification rapide (sans les details utilisateur).

**Entree :** Cookie \`auth_session\`

**Sortie (connecte) :** \`authenticated\`: true
**Sortie (non connecte) :** \`authenticated\`: false

## Groupes disponibles

- \`admins\` : administrateurs
- \`power_users\` : utilisateurs avances
- \`users\` : utilisateurs standards`;

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
      const [configRes, statusRes, hostsRes, certsRes, envsRes, appsRes, cfRes] = await Promise.all([
        getReverseProxyConfig(),
        getReverseProxyStatus(),
        getReverseProxyHosts(),
        getCertificatesStatus(),
        getReverseProxyEnvironments(),
        getReverseProxyApplications(),
        getCloudflareConfig()
      ]);

      if (configRes.data.success) {
        setConfig(configRes.data.config);
        setConfigForm({ baseDomain: configRes.data.config.baseDomain || '' });
        if (!configRes.data.config.baseDomain) {
          setShowDomainRequiredModal(true);
        }
      }
      if (statusRes.data.success) setStatus(statusRes.data);
      if (hostsRes.data.success) {
        setHosts(hostsRes.data.hosts || []);
        // Show migration banner if there are hosts that could be migrated
        if ((hostsRes.data.hosts || []).length > 0 && (appsRes.data.applications || []).length === 0) {
          setShowMigrationBanner(true);
        }
      }
      if (certsRes.data.success) setCertStatuses(certsRes.data.certificates || {});
      if (envsRes.data.success) setEnvironments(envsRes.data.environments || []);
      if (appsRes.data.success) setApplications(appsRes.data.applications || []);
      if (cfRes.data.success) setCloudflare(cfRes.data.cloudflare);
    } catch (error) {
      console.error('Error:', error);
      setMessage({ type: 'error', text: 'Erreur de chargement' });
    } finally {
      setLoading(false);
    }
  }

  // ========== Host handlers ==========
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
      setMessage({ type: 'error', text: 'Domaine personnalise requis' });
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
      if (hostType === 'subdomain') payload.subdomain = newHost.subdomain;
      else payload.customDomain = newHost.customDomain;

      const res = await addReverseProxyHost(payload);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Hote ajoute' });
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
      if (res.data.success) fetchData();
      else setMessage({ type: 'error', text: res.data.error });
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  async function handleDeleteHost(hostId) {
    if (!confirm('Supprimer cet hote ?')) return;
    try {
      const res = await deleteReverseProxyHost(hostId);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Hote supprime' });
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
        setMessage({ type: 'success', text: 'Hote modifie' });
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

  // ========== Application handlers ==========
  async function handleAddApp() {
    if (!newApp.name || !newApp.slug) {
      setMessage({ type: 'error', text: 'Nom et slug requis' });
      return;
    }

    // Validate at least one environment is enabled with frontend
    const enabledEnvs = Object.entries(newApp.endpoints).filter(([, e]) => e.enabled);
    if (enabledEnvs.length === 0) {
      setMessage({ type: 'error', text: 'Au moins un environnement doit etre active' });
      return;
    }
    for (const [envId, envData] of enabledEnvs) {
      if (!envData.frontend.targetHost || !envData.frontend.targetPort) {
        setMessage({ type: 'error', text: `Cible frontend requise pour ${envId}` });
        return;
      }
    }

    setSaving(true);
    try {
      // Build endpoints payload
      const endpoints = {};
      for (const [envId, envData] of Object.entries(newApp.endpoints)) {
        if (envData.enabled) {
          endpoints[envId] = {
            frontend: {
              targetHost: envData.frontend.targetHost,
              targetPort: parseInt(envData.frontend.targetPort),
              localOnly: envData.frontend.localOnly || false,
              requireAuth: envData.frontend.requireAuth || false
            },
            api: envData.hasApi && envData.api?.targetPort ? {
              targetHost: envData.api.targetHost,
              targetPort: parseInt(envData.api.targetPort),
              localOnly: envData.api.localOnly || false,
              requireAuth: envData.api.requireAuth || false
            } : null
          };
        }
      }

      const payload = {
        name: newApp.name,
        slug: newApp.slug.toLowerCase(),
        endpoints
      };

      const res = await addReverseProxyApplication(payload);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Application ajoutee' });
        setShowAddAppModal(false);
        // Reset form
        setNewApp({
          name: '',
          slug: '',
          endpoints: {
            prod: {
              enabled: true,
              frontend: { targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false },
              hasApi: false,
              api: { targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false }
            }
          }
        });
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

  async function handleToggleApp(appId, enabled) {
    try {
      const res = await toggleReverseProxyApplication(appId, enabled);
      if (res.data.success) fetchData();
      else setMessage({ type: 'error', text: res.data.error });
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  async function handleDeleteApp(appId) {
    if (!confirm('Supprimer cette application ?')) return;
    try {
      const res = await deleteReverseProxyApplication(appId);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Application supprimee' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  // Helper to get domain for display
  function getAppDomain(slug, type, env, baseDomain, apiSlug = '') {
    if (type === 'api') {
      // Format: {app}-{slug}.{apiPrefix}.{baseDomain} or {app}.{apiPrefix}.{baseDomain}
      const hostPart = apiSlug ? `${slug}-${apiSlug}` : slug;
      return `${hostPart}.${env.apiPrefix}.${baseDomain}`;
    }
    return env.prefix ? `${slug}.${env.prefix}.${baseDomain}` : `${slug}.${baseDomain}`;
  }

  function openEditAppModal(app) {
    setEditingApp(app);
    // Convert endpoints to form structure with enabled flags per environment
    const formEndpoints = {};
    for (const env of environments) {
      const existing = app.endpoints?.[env.id];
      // Convert existing apis[] or legacy api to apis[] array
      let apis = [];
      if (existing?.apis && Array.isArray(existing.apis)) {
        apis = existing.apis.map(api => ({ ...api }));
      } else if (existing?.api) {
        // Legacy: convert single api to apis[]
        apis = [{ slug: '', ...existing.api }];
      }
      formEndpoints[env.id] = {
        enabled: !!existing,
        frontend: existing?.frontend ? { ...existing.frontend } : { targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false },
        apis
      };
    }
    setEditAppForm({
      name: app.name,
      endpoints: formEndpoints
    });
    setShowEditAppModal(true);
  }

  async function handleEditApp() {
    // Validate at least one environment is enabled with frontend
    const enabledEnvs = Object.entries(editAppForm.endpoints).filter(([, e]) => e.enabled);
    if (enabledEnvs.length === 0) {
      setMessage({ type: 'error', text: 'Au moins un environnement doit etre active' });
      return;
    }
    for (const [envId, envData] of enabledEnvs) {
      if (!envData.frontend.targetHost || !envData.frontend.targetPort) {
        setMessage({ type: 'error', text: `Cible frontend requise pour ${envId}` });
        return;
      }
    }

    setSaving(true);
    try {
      // Build endpoints payload with apis[] array
      const endpoints = {};
      for (const [envId, envData] of Object.entries(editAppForm.endpoints)) {
        if (envData.enabled) {
          // Filter out APIs without port
          const validApis = (envData.apis || [])
            .filter(api => api.targetPort)
            .map(api => ({
              slug: (api.slug || '').toLowerCase().replace(/[^a-z0-9-]/g, ''),
              targetHost: api.targetHost || 'localhost',
              targetPort: parseInt(api.targetPort),
              localOnly: api.localOnly || false,
              requireAuth: api.requireAuth || false
            }));

          endpoints[envId] = {
            frontend: {
              targetHost: envData.frontend.targetHost,
              targetPort: parseInt(envData.frontend.targetPort),
              localOnly: envData.frontend.localOnly || false,
              requireAuth: envData.frontend.requireAuth || false
            },
            apis: validApis
          };
        } else {
          endpoints[envId] = null; // Remove environment
        }
      }

      const payload = {
        name: editAppForm.name,
        endpoints
      };

      const res = await updateReverseProxyApplication(editingApp.id, payload);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Application modifiee' });
        setShowEditAppModal(false);
        setEditingApp(null);
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

  // ========== Migration handlers ==========
  async function loadMigrationSuggestions() {
    try {
      const res = await getMigrationSuggestions();
      if (res.data.success) {
        setMigrationSuggestions(res.data.suggestions);
        setShowMigrationModal(true);
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur chargement suggestions' });
    }
  }

  async function handleMigration() {
    if (!migrationSuggestions) return;
    setMigrating(true);
    try {
      const res = await executeMigration(migrationSuggestions);
      if (res.data.success) {
        setMessage({ type: 'success', text: res.data.message });
        setShowMigrationModal(false);
        setShowMigrationBanner(false);
        setMigrationSuggestions(null);
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur migration' });
    } finally {
      setMigrating(false);
    }
  }

  // ========== Config handlers ==========
  async function handleSaveConfig() {
    setSaving(true);
    setMessage(null);
    try {
      if (configForm.baseDomain !== config?.baseDomain) {
        const domainRes = await updateBaseDomain(configForm.baseDomain);
        if (!domainRes.data.success) {
          setMessage({ type: 'error', text: domainRes.data.error });
          setSaving(false);
          return;
        }
      }
      setMessage({ type: 'success', text: 'Configuration sauvegardee' });
      setShowConfigModal(false);
      setShowDomainRequiredModal(false);
      fetchData();
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur de sauvegarde' });
    } finally {
      setSaving(false);
    }
  }

  async function handleToggleCloudflare(enabled) {
    try {
      const res = await updateCloudflareConfig({ enabled });
      if (res.data.success) {
        setCloudflare(res.data.cloudflare);
        setMessage({ type: 'success', text: enabled ? 'Cloudflare active' : 'Cloudflare desactive' });
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
        setMessage({ type: 'success', text: 'Renouvellement declenche' });
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
        setMessage({ type: 'success', text: 'Caddy recharge' });
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
  const activeApps = applications.filter(a => a.enabled).length;
  const caddyRunning = status?.caddy?.running;

  const tabs = [
    { id: 'applications', label: 'Applications', icon: Layers, count: applications.length },
    { id: 'standalone', label: 'Standalone', icon: Globe, count: hosts.length },
    { id: 'config', label: 'Configuration', icon: Settings }
  ];

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
          {activeTab === 'applications' && (
            <Button onClick={() => setShowAddAppModal(true)} disabled={!config?.baseDomain}>
              <Plus className="w-4 h-4" />
              Nouvelle app
            </Button>
          )}
          {activeTab === 'standalone' && (
            <Button onClick={() => setShowAddModal(true)} disabled={!config?.baseDomain}>
              <Plus className="w-4 h-4" />
              Ajouter hote
            </Button>
          )}
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

      {/* Migration Banner */}
      {showMigrationBanner && hosts.length > 0 && applications.length === 0 && (
        <div className="bg-blue-900/30 border border-blue-700 rounded-lg p-4 flex items-center justify-between">
          <div className="flex items-center gap-3">
            <ArrowRightLeft className="w-5 h-5 text-blue-400" />
            <div>
              <p className="font-medium text-blue-300">Migration disponible</p>
              <p className="text-sm text-blue-400/70">Groupez vos hosts existants en applications (frontend + API)</p>
            </div>
          </div>
          <Button onClick={loadMigrationSuggestions} variant="secondary">
            Voir les suggestions
          </Button>
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
            {config?.baseDomain || 'Non configure'}
          </div>
        </Card>

        <Card title="Certificats" icon={Lock}>
          <div className="flex items-center gap-2">
            <div className={`w-3 h-3 rounded-full ${cloudflare?.enabled ? 'bg-orange-400' : 'bg-green-400'}`} />
            <span className={cloudflare?.enabled ? 'text-orange-400' : 'text-green-400'}>
              {cloudflare?.enabled ? 'Cloudflare Wildcard' : "Let's Encrypt"}
            </span>
          </div>
        </Card>

        <Card title="Services" icon={Wifi}>
          <div className="text-2xl font-bold text-green-400">
            {activeApps + activeHosts}
          </div>
          <p className="text-xs text-gray-500">{activeApps} apps, {activeHosts} standalone</p>
        </Card>
      </div>

      {/* Tabs */}
      <div className="border-b border-gray-700">
        <div className="flex gap-1">
          {tabs.map(tab => (
            <button
              key={tab.id}
              onClick={() => setActiveTab(tab.id)}
              className={`flex items-center gap-2 px-4 py-3 text-sm font-medium border-b-2 transition-colors ${
                activeTab === tab.id
                  ? 'border-blue-500 text-blue-400'
                  : 'border-transparent text-gray-400 hover:text-gray-300'
              }`}
            >
              <tab.icon className="w-4 h-4" />
              {tab.label}
              {tab.count !== undefined && (
                <span className="text-xs bg-gray-700 px-2 py-0.5 rounded-full">{tab.count}</span>
              )}
            </button>
          ))}
        </div>
      </div>

      {/* Tab Content */}
      {activeTab === 'applications' && (
        <div className="space-y-4">
          {applications.length === 0 ? (
            <Card>
              <div className="text-center py-8 text-gray-500">
                <Layers className="w-12 h-12 mx-auto mb-2 opacity-50" />
                <p>Aucune application configuree</p>
                <p className="text-xs mt-2">Les applications groupent un frontend et son API</p>
                {hosts.length > 0 && (
                  <Button onClick={loadMigrationSuggestions} variant="secondary" className="mt-4">
                    Migrer depuis les hosts existants
                  </Button>
                )}
              </div>
            </Card>
          ) : (
            <div className="space-y-3">
              {applications.map(app => (
                <ApplicationCard
                  key={app.id}
                  app={app}
                  environments={environments}
                  baseDomain={config?.baseDomain}
                  certStatuses={certStatuses}
                  onToggle={handleToggleApp}
                  onEdit={openEditAppModal}
                  onDelete={handleDeleteApp}
                />
              ))}
            </div>
          )}
        </div>
      )}

      {activeTab === 'standalone' && (
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          <div className="lg:col-span-2">
            <Card title="Hotes standalone" icon={Globe}>
              {hosts.length === 0 ? (
                <div className="text-center py-8 text-gray-500">
                  <Globe className="w-12 h-12 mx-auto mb-2 opacity-50" />
                  <p>Aucun hote standalone</p>
                  <p className="text-xs mt-2">Pour les services qui ne suivent pas le pattern frontend/API</p>
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
                                  <span className="flex items-center gap-1 text-xs text-yellow-400 bg-yellow-900/30 px-2 py-0.5 rounded">
                                    <Shield className="w-3 h-3" />
                                    Local
                                  </span>
                                )}
                                {host.requireAuth && (
                                  <span className="flex items-center gap-1 text-xs text-purple-400 bg-purple-900/30 px-2 py-0.5 rounded">
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
                                <span className={`flex items-center gap-1 text-xs px-2 py-0.5 rounded ${
                                  certStatus.valid
                                    ? certStatus.daysRemaining <= 14 ? 'text-yellow-400 bg-yellow-900/30' : 'text-green-400 bg-green-900/30'
                                    : 'text-red-400 bg-red-900/30'
                                }`}>
                                  <Lock className="w-3 h-3" />
                                  {certStatus.valid ? (certStatus.daysRemaining <= 14 ? `${certStatus.daysRemaining}j` : 'OK') : 'Erreur'}
                                </span>
                              ) : <span className="text-xs text-gray-500">-</span>}
                            </td>
                            <td className="py-3">
                              <button
                                onClick={() => handleToggleHost(host.id, !host.enabled)}
                                className={`p-1.5 rounded transition-colors ${
                                  host.enabled ? 'text-green-400 bg-green-900/30 hover:bg-green-900/50' : 'text-gray-500 bg-gray-700/30 hover:bg-gray-700/50'
                                }`}
                              >
                                <Power className="w-4 h-4" />
                              </button>
                            </td>
                            <td className="py-3 text-right">
                              <div className="flex justify-end gap-1">
                                <button onClick={() => openEditModal(host)} className="text-blue-400 hover:text-blue-300 p-1">
                                  <Pencil className="w-4 h-4" />
                                </button>
                                <button onClick={() => handleDeleteHost(host.id)} className="text-red-400 hover:text-red-300 p-1">
                                  <Trash2 className="w-4 h-4" />
                                </button>
                              </div>
                            </td>
                          </tr>
                        );
                      })}
                    </tbody>
                  </table>
                </div>
              )}
            </Card>
          </div>

          {/* Integration Card */}
          <div>
            <Card
              title="Integration Auth"
              icon={FileCode}
              actions={
                <div className="flex gap-2">
                  <Button onClick={copyInstructions} variant="secondary" className="text-sm">
                    {instructionsCopied ? <><CheckCircle className="w-4 h-4" /> Copie !</> : <><Copy className="w-4 h-4" /> Copier</>}
                  </Button>
                  <button onClick={() => setShowIntegration(!showIntegration)} className="text-gray-400 hover:text-gray-300 p-2">
                    {showIntegration ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
                  </button>
                </div>
              }
            >
              <p className="text-sm text-gray-400 mb-3">Instructions pour integrer l&apos;authentification.</p>
              {showIntegration && (
                <pre className="text-xs text-gray-300 whitespace-pre-wrap font-mono overflow-x-auto max-h-96 overflow-y-auto bg-gray-900 rounded p-4">
                  {authInstructions}
                </pre>
              )}
            </Card>
          </div>
        </div>
      )}

      {activeTab === 'config' && (
        <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
          {/* Domain Config */}
          <Card title="Domaine de base" icon={Globe}>
            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Domaine</label>
                <div className="flex gap-2">
                  <input
                    type="text"
                    value={configForm.baseDomain}
                    onChange={e => setConfigForm({ ...configForm, baseDomain: e.target.value })}
                    className="flex-1 px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm"
                    placeholder="example.com"
                  />
                  <Button onClick={handleSaveConfig} loading={saving} disabled={configForm.baseDomain === config?.baseDomain}>
                    Sauver
                  </Button>
                </div>
              </div>
              <div className="text-xs text-gray-500 space-y-1">
                <p>Dashboard: <span className="font-mono text-blue-400">proxy.{configForm.baseDomain || 'domain.com'}</span></p>
                <p>Auth: <span className="font-mono text-blue-400">auth.{configForm.baseDomain || 'domain.com'}</span></p>
              </div>
            </div>
          </Card>

          {/* Cloudflare Config */}
          <Card title="Certificats Wildcard" icon={Cloud}>
            <div className="space-y-4">
              <div
                onClick={() => handleToggleCloudflare(!cloudflare?.enabled)}
                className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer transition-colors ${
                  cloudflare?.enabled ? 'bg-orange-900/30 border-orange-600 text-orange-400' : 'bg-gray-900/50 border-gray-700 text-gray-400'
                }`}
              >
                <Cloud className={`w-5 h-5 ${cloudflare?.enabled ? 'text-orange-400' : 'text-gray-500'}`} />
                <div className="flex-1">
                  <div className="font-medium text-sm">Cloudflare DNS Challenge</div>
                  <div className="text-xs opacity-75">Certificats wildcard via API Cloudflare</div>
                </div>
                <div className={`w-10 h-6 rounded-full transition-colors ${cloudflare?.enabled ? 'bg-orange-600' : 'bg-gray-600'}`}>
                  <div className={`w-4 h-4 bg-white rounded-full mt-1 transition-transform ${cloudflare?.enabled ? 'translate-x-5' : 'translate-x-1'}`} />
                </div>
              </div>

              {cloudflare?.enabled && (
                <div className="text-xs space-y-2">
                  <p className={`flex items-center gap-2 ${cloudflare.hasToken ? 'text-green-400' : 'text-red-400'}`}>
                    {cloudflare.hasToken ? <CheckCircle className="w-4 h-4" /> : <XCircle className="w-4 h-4" />}
                    {cloudflare.hasToken ? 'CF_API_TOKEN configure' : 'CF_API_TOKEN manquant dans .env'}
                  </p>
                  {cloudflare.wildcardDomains?.length > 0 && (
                    <div className="bg-gray-900 rounded p-2 space-y-1">
                      <p className="text-gray-500">Domaines wildcard:</p>
                      {cloudflare.wildcardDomains.map(d => (
                        <p key={d} className="font-mono text-orange-400">{d}</p>
                      ))}
                    </div>
                  )}
                </div>
              )}

              {!cloudflare?.enabled && (
                <p className="text-xs text-gray-500">
                  Mode actuel: certificats individuels Let&apos;s Encrypt via HTTP challenge
                </p>
              )}
            </div>
          </Card>

          {/* Environments */}
          <Card title="Environnements" icon={Layers}>
            <div className="space-y-2">
              {environments.map(env => (
                <div key={env.id} className="p-3 bg-gray-900/50 rounded border border-gray-700">
                  <div className="flex items-center justify-between mb-2">
                    <div className="font-medium">{env.name}</div>
                    {env.isDefault && (
                      <span className="text-xs bg-blue-900/50 text-blue-400 px-2 py-1 rounded">Default</span>
                    )}
                  </div>
                  <div className="space-y-1 text-xs">
                    <div className="flex items-center gap-2">
                      <Globe className="w-3 h-3 text-blue-400" />
                      <span className="text-gray-400">Frontend:</span>
                      <span className="font-mono text-blue-400">
                        {env.prefix ? `*.${env.prefix}.${config?.baseDomain}` : `*.${config?.baseDomain}`}
                      </span>
                    </div>
                    <div className="flex items-center gap-2">
                      <Server className="w-3 h-3 text-green-400" />
                      <span className="text-gray-400">API:</span>
                      <span className="font-mono text-green-400">
                        *.{env.apiPrefix}.{config?.baseDomain}
                      </span>
                    </div>
                  </div>
                </div>
              ))}
            </div>
          </Card>
        </div>
      )}

      {/* Add Host Modal */}
      {showAddModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-700">
            <h2 className="text-xl font-bold mb-4">Ajouter un hote standalone</h2>
            <div className="space-y-4">
              <div className="flex gap-2">
                <button onClick={() => setHostType('subdomain')} className={`flex-1 py-2 rounded text-sm ${hostType === 'subdomain' ? 'bg-blue-600 text-white' : 'bg-gray-700 text-gray-300'}`}>Sous-domaine</button>
                <button onClick={() => setHostType('custom')} className={`flex-1 py-2 rounded text-sm ${hostType === 'custom' ? 'bg-blue-600 text-white' : 'bg-gray-700 text-gray-300'}`}>Domaine perso</button>
              </div>

              {hostType === 'subdomain' ? (
                <div>
                  <label className="block text-sm text-gray-400 mb-1">Sous-domaine</label>
                  <div className="flex">
                    <input type="text" placeholder="app" value={newHost.subdomain} onChange={e => setNewHost({ ...newHost, subdomain: e.target.value })} className="flex-1 px-3 py-2 bg-gray-900 border border-gray-600 rounded-l text-sm" />
                    <span className="px-3 py-2 bg-gray-700 border border-l-0 border-gray-600 rounded-r text-gray-400 text-sm">.{config?.baseDomain}</span>
                  </div>
                </div>
              ) : (
                <div>
                  <label className="block text-sm text-gray-400 mb-1">Domaine complet</label>
                  <input type="text" placeholder="app.example.com" value={newHost.customDomain} onChange={e => setNewHost({ ...newHost, customDomain: e.target.value })} className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm" />
                </div>
              )}

              <div>
                <label className="block text-sm text-gray-400 mb-1">Hote cible</label>
                <input type="text" placeholder="localhost" value={newHost.targetHost} onChange={e => setNewHost({ ...newHost, targetHost: e.target.value })} className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm" />
              </div>

              <div>
                <label className="block text-sm text-gray-400 mb-1">Port</label>
                <input type="number" placeholder="3000" value={newHost.targetPort} onChange={e => setNewHost({ ...newHost, targetPort: e.target.value })} className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm" />
              </div>

              <div onClick={() => setNewHost({ ...newHost, localOnly: !newHost.localOnly })} className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer ${newHost.localOnly ? 'bg-yellow-900/30 border-yellow-600' : 'bg-gray-900/50 border-gray-700'}`}>
                <Shield className="w-5 h-5" />
                <div className="flex-1"><div className="text-sm">Reseau local uniquement</div></div>
                <div className={`w-10 h-6 rounded-full ${newHost.localOnly ? 'bg-yellow-600' : 'bg-gray-600'}`}><div className={`w-4 h-4 bg-white rounded-full mt-1 ${newHost.localOnly ? 'translate-x-5' : 'translate-x-1'}`} /></div>
              </div>

              <div onClick={() => setNewHost({ ...newHost, requireAuth: !newHost.requireAuth })} className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer ${newHost.requireAuth ? 'bg-purple-900/30 border-purple-600' : 'bg-gray-900/50 border-gray-700'}`}>
                <Key className="w-5 h-5" />
                <div className="flex-1"><div className="text-sm">Authentification requise</div></div>
                <div className={`w-10 h-6 rounded-full ${newHost.requireAuth ? 'bg-purple-600' : 'bg-gray-600'}`}><div className={`w-4 h-4 bg-white rounded-full mt-1 ${newHost.requireAuth ? 'translate-x-5' : 'translate-x-1'}`} /></div>
              </div>
            </div>
            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => setShowAddModal(false)}>Annuler</Button>
              <Button onClick={handleAddHost} loading={saving}>Ajouter</Button>
            </div>
          </div>
        </div>
      )}

      {/* Add App Modal */}
      {showAddAppModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-2xl border border-gray-700 max-h-[90vh] overflow-y-auto">
            <h2 className="text-xl font-bold mb-4">Nouvelle application</h2>
            <div className="space-y-4">
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="block text-sm text-gray-400 mb-1">Nom</label>
                  <input type="text" placeholder="Mon App" value={newApp.name} onChange={e => setNewApp({ ...newApp, name: e.target.value, slug: newApp.slug || e.target.value.toLowerCase().replace(/[^a-z0-9]/g, '') })} className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm" />
                </div>
                <div>
                  <label className="block text-sm text-gray-400 mb-1">Slug</label>
                  <input type="text" placeholder="monapp" value={newApp.slug} onChange={e => setNewApp({ ...newApp, slug: e.target.value.toLowerCase() })} className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm font-mono" />
                </div>
              </div>

              {/* Endpoints par environnement */}
              {environments.map(env => {
                // Initialize env endpoint if not exists
                const envData = newApp.endpoints[env.id] || {
                  enabled: false,
                  frontend: { targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false },
                  hasApi: false,
                  api: { targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false }
                };

                return (
                  <div key={env.id} className={`border rounded-lg overflow-hidden ${envData.enabled ? 'border-gray-600' : 'border-gray-800 opacity-60'}`}>
                    <div className="bg-gray-900/50 px-4 py-2 flex items-center justify-between">
                      <label className="flex items-center gap-2 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={envData.enabled}
                          onChange={e => setNewApp({
                            ...newApp,
                            endpoints: {
                              ...newApp.endpoints,
                              [env.id]: { ...envData, enabled: e.target.checked }
                            }
                          })}
                          className="rounded"
                        />
                        <Layers className="w-4 h-4" />
                        <span className="font-medium">{env.name}</span>
                      </label>
                    </div>

                    {envData.enabled && (
                      <div className="p-4 space-y-4">
                        {/* Frontend */}
                        <div>
                          <div className="text-xs text-gray-400 mb-2 font-mono">
                            <Globe className="w-3 h-3 inline mr-1" />
                            {newApp.slug || 'slug'}{env.prefix ? `.${env.prefix}` : ''}.{config?.baseDomain}
                          </div>
                          <div className="grid grid-cols-2 gap-3">
                            <div>
                              <label className="block text-xs text-gray-500 mb-1">Host</label>
                              <input
                                type="text"
                                value={envData.frontend.targetHost}
                                onChange={e => setNewApp({
                                  ...newApp,
                                  endpoints: {
                                    ...newApp.endpoints,
                                    [env.id]: { ...envData, frontend: { ...envData.frontend, targetHost: e.target.value } }
                                  }
                                })}
                                className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 rounded text-sm"
                              />
                            </div>
                            <div>
                              <label className="block text-xs text-gray-500 mb-1">Port</label>
                              <input
                                type="number"
                                placeholder="3000"
                                value={envData.frontend.targetPort}
                                onChange={e => setNewApp({
                                  ...newApp,
                                  endpoints: {
                                    ...newApp.endpoints,
                                    [env.id]: { ...envData, frontend: { ...envData.frontend, targetPort: e.target.value } }
                                  }
                                })}
                                className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 rounded text-sm"
                              />
                            </div>
                          </div>
                          <div className="flex gap-4 mt-2">
                            <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                              <input
                                type="checkbox"
                                checked={envData.frontend.requireAuth}
                                onChange={e => setNewApp({
                                  ...newApp,
                                  endpoints: {
                                    ...newApp.endpoints,
                                    [env.id]: { ...envData, frontend: { ...envData.frontend, requireAuth: e.target.checked } }
                                  }
                                })}
                                className="rounded"
                              />
                              <Key className="w-3 h-3 text-purple-400" /> Auth
                            </label>
                            <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                              <input
                                type="checkbox"
                                checked={envData.frontend.localOnly}
                                onChange={e => setNewApp({
                                  ...newApp,
                                  endpoints: {
                                    ...newApp.endpoints,
                                    [env.id]: { ...envData, frontend: { ...envData.frontend, localOnly: e.target.checked } }
                                  }
                                })}
                                className="rounded"
                              />
                              <Shield className="w-3 h-3 text-yellow-400" /> Local
                            </label>
                          </div>
                        </div>

                        {/* API */}
                        <div className="border-t border-gray-700 pt-3">
                          <label className="flex items-center gap-2 mb-2 cursor-pointer">
                            <input
                              type="checkbox"
                              checked={envData.hasApi}
                              onChange={e => setNewApp({
                                ...newApp,
                                endpoints: {
                                  ...newApp.endpoints,
                                  [env.id]: { ...envData, hasApi: e.target.checked }
                                }
                              })}
                              className="rounded"
                            />
                            <Server className="w-4 h-4" />
                            <span className="text-sm">API</span>
                          </label>
                          {envData.hasApi && (
                            <>
                              <div className="text-xs text-gray-400 mb-2 font-mono">
                                <Server className="w-3 h-3 inline mr-1" />
                                {newApp.slug || 'slug'}.{env.apiPrefix}.{config?.baseDomain}
                              </div>
                              <div className="grid grid-cols-2 gap-3">
                                <div>
                                  <label className="block text-xs text-gray-500 mb-1">Host</label>
                                  <input
                                    type="text"
                                    value={envData.api.targetHost}
                                    onChange={e => setNewApp({
                                      ...newApp,
                                      endpoints: {
                                        ...newApp.endpoints,
                                        [env.id]: { ...envData, api: { ...envData.api, targetHost: e.target.value } }
                                      }
                                    })}
                                    className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 rounded text-sm"
                                  />
                                </div>
                                <div>
                                  <label className="block text-xs text-gray-500 mb-1">Port</label>
                                  <input
                                    type="number"
                                    placeholder="3001"
                                    value={envData.api.targetPort}
                                    onChange={e => setNewApp({
                                      ...newApp,
                                      endpoints: {
                                        ...newApp.endpoints,
                                        [env.id]: { ...envData, api: { ...envData.api, targetPort: e.target.value } }
                                      }
                                    })}
                                    className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 rounded text-sm"
                                  />
                                </div>
                              </div>
                              <div className="flex gap-4 mt-2">
                                <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                                  <input
                                    type="checkbox"
                                    checked={envData.api.requireAuth}
                                    onChange={e => setNewApp({
                                      ...newApp,
                                      endpoints: {
                                        ...newApp.endpoints,
                                        [env.id]: { ...envData, api: { ...envData.api, requireAuth: e.target.checked } }
                                      }
                                    })}
                                    className="rounded"
                                  />
                                  <Key className="w-3 h-3 text-purple-400" /> Auth
                                </label>
                                <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                                  <input
                                    type="checkbox"
                                    checked={envData.api.localOnly}
                                    onChange={e => setNewApp({
                                      ...newApp,
                                      endpoints: {
                                        ...newApp.endpoints,
                                        [env.id]: { ...envData, api: { ...envData.api, localOnly: e.target.checked } }
                                      }
                                    })}
                                    className="rounded"
                                  />
                                  <Shield className="w-3 h-3 text-yellow-400" /> Local
                                </label>
                              </div>
                            </>
                          )}
                        </div>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => setShowAddAppModal(false)}>Annuler</Button>
              <Button onClick={handleAddApp} loading={saving}>Creer</Button>
            </div>
          </div>
        </div>
      )}

      {/* Edit Host Modal */}
      {showEditModal && editingHost && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-700">
            <h2 className="text-xl font-bold mb-4">Modifier l&apos;hote</h2>
            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Domaine</label>
                <div className="px-3 py-2 bg-gray-900/50 border border-gray-700 rounded text-sm font-mono text-gray-400">
                  {editingHost.customDomain || `${editingHost.subdomain}.${config?.baseDomain}`}
                </div>
              </div>
              <div>
                <label className="block text-sm text-gray-400 mb-1">Hote cible</label>
                <input type="text" value={editForm.targetHost} onChange={e => setEditForm({ ...editForm, targetHost: e.target.value })} className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm" />
              </div>
              <div>
                <label className="block text-sm text-gray-400 mb-1">Port</label>
                <input type="number" value={editForm.targetPort} onChange={e => setEditForm({ ...editForm, targetPort: e.target.value })} className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm" />
              </div>
              <div onClick={() => setEditForm({ ...editForm, localOnly: !editForm.localOnly })} className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer ${editForm.localOnly ? 'bg-yellow-900/30 border-yellow-600' : 'bg-gray-900/50 border-gray-700'}`}>
                <Shield className="w-5 h-5" />
                <div className="flex-1"><div className="text-sm">Reseau local uniquement</div></div>
                <div className={`w-10 h-6 rounded-full ${editForm.localOnly ? 'bg-yellow-600' : 'bg-gray-600'}`}><div className={`w-4 h-4 bg-white rounded-full mt-1 ${editForm.localOnly ? 'translate-x-5' : 'translate-x-1'}`} /></div>
              </div>
              <div onClick={() => setEditForm({ ...editForm, requireAuth: !editForm.requireAuth })} className={`flex items-center gap-3 p-3 rounded-lg border cursor-pointer ${editForm.requireAuth ? 'bg-purple-900/30 border-purple-600' : 'bg-gray-900/50 border-gray-700'}`}>
                <Key className="w-5 h-5" />
                <div className="flex-1"><div className="text-sm">Authentification requise</div></div>
                <div className={`w-10 h-6 rounded-full ${editForm.requireAuth ? 'bg-purple-600' : 'bg-gray-600'}`}><div className={`w-4 h-4 bg-white rounded-full mt-1 ${editForm.requireAuth ? 'translate-x-5' : 'translate-x-1'}`} /></div>
              </div>
            </div>
            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => { setShowEditModal(false); setEditingHost(null); }}>Annuler</Button>
              <Button onClick={handleEditHost} loading={saving}>Sauvegarder</Button>
            </div>
          </div>
        </div>
      )}

      {/* Edit App Modal */}
      {showEditAppModal && editingApp && editAppForm && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-4xl border border-gray-700 max-h-[90vh] overflow-y-auto">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-xl font-bold">Modifier {editingApp.name}</h2>
              <span className="text-xs text-gray-500 bg-gray-900/50 px-2 py-1 rounded font-mono">
                {editingApp.slug}
              </span>
            </div>
            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Nom</label>
                <input type="text" value={editAppForm.name} onChange={e => setEditAppForm({ ...editAppForm, name: e.target.value })} className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm" />
              </div>

              {/* Endpoints par environnement - Layout optimise */}
              {environments.map(env => {
                const envData = editAppForm.endpoints[env.id];
                if (!envData) return null;

                // Helper to update an API in the apis array
                const updateApi = (apiIndex, updates) => {
                  const newApis = [...envData.apis];
                  newApis[apiIndex] = { ...newApis[apiIndex], ...updates };
                  setEditAppForm({
                    ...editAppForm,
                    endpoints: {
                      ...editAppForm.endpoints,
                      [env.id]: { ...envData, apis: newApis }
                    }
                  });
                };

                // Helper to add a new API
                const addApi = () => {
                  const slug = prompt('Slug de l\'API (ex: cdn, ws, v2):');
                  if (slug === null) return;
                  const cleanSlug = slug.toLowerCase().replace(/[^a-z0-9-]/g, '');
                  // Check if slug already exists
                  if (envData.apis.some(a => a.slug === cleanSlug)) {
                    setMessage({ type: 'error', text: `API "${cleanSlug}" existe deja` });
                    return;
                  }
                  setEditAppForm({
                    ...editAppForm,
                    endpoints: {
                      ...editAppForm.endpoints,
                      [env.id]: {
                        ...envData,
                        apis: [...envData.apis, { slug: cleanSlug, targetHost: 'localhost', targetPort: '', localOnly: false, requireAuth: false }]
                      }
                    }
                  });
                };

                // Helper to remove an API
                const removeApi = (apiIndex) => {
                  const newApis = envData.apis.filter((_, i) => i !== apiIndex);
                  setEditAppForm({
                    ...editAppForm,
                    endpoints: {
                      ...editAppForm.endpoints,
                      [env.id]: { ...envData, apis: newApis }
                    }
                  });
                };

                return (
                  <div key={env.id} className={`border rounded-lg overflow-hidden ${envData.enabled ? 'border-gray-600' : 'border-gray-800 opacity-60'}`}>
                    <div className="bg-gray-900/50 px-4 py-2 flex items-center justify-between">
                      <label className="flex items-center gap-2 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={envData.enabled}
                          onChange={e => setEditAppForm({
                            ...editAppForm,
                            endpoints: {
                              ...editAppForm.endpoints,
                              [env.id]: { ...envData, enabled: e.target.checked }
                            }
                          })}
                          className="rounded"
                        />
                        <Layers className="w-4 h-4" />
                        <span className="font-medium">{env.name}</span>
                      </label>
                    </div>

                    {envData.enabled && (
                      <div className="p-4">
                        {/* Grille 2 colonnes pour Frontend + APIs */}
                        <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                          {/* Frontend - toujours present */}
                          <div className="bg-gray-900/30 rounded-lg p-3 border border-gray-700">
                            <div className="text-xs text-blue-400 mb-2 font-mono flex items-center gap-1">
                              <Globe className="w-3 h-3" />
                              {getAppDomain(editingApp.slug, 'frontend', env, config?.baseDomain)}
                            </div>
                            <div className="flex gap-2 mb-2">
                              <input
                                type="text"
                                value={envData.frontend.targetHost}
                                onChange={e => setEditAppForm({
                                  ...editAppForm,
                                  endpoints: {
                                    ...editAppForm.endpoints,
                                    [env.id]: { ...envData, frontend: { ...envData.frontend, targetHost: e.target.value } }
                                  }
                                })}
                                placeholder="localhost"
                                className="flex-1 px-2 py-1 bg-gray-900 border border-gray-700 rounded text-sm"
                              />
                              <input
                                type="number"
                                value={envData.frontend.targetPort}
                                onChange={e => setEditAppForm({
                                  ...editAppForm,
                                  endpoints: {
                                    ...editAppForm.endpoints,
                                    [env.id]: { ...envData, frontend: { ...envData.frontend, targetPort: e.target.value } }
                                  }
                                })}
                                placeholder="3000"
                                className="w-20 px-2 py-1 bg-gray-900 border border-gray-700 rounded text-sm"
                              />
                            </div>
                            <div className="flex gap-3">
                              <label className="flex items-center gap-1 text-xs cursor-pointer">
                                <input
                                  type="checkbox"
                                  checked={envData.frontend.requireAuth}
                                  onChange={e => setEditAppForm({
                                    ...editAppForm,
                                    endpoints: {
                                      ...editAppForm.endpoints,
                                      [env.id]: { ...envData, frontend: { ...envData.frontend, requireAuth: e.target.checked } }
                                    }
                                  })}
                                  className="rounded"
                                />
                                <Key className="w-3 h-3 text-purple-400" />
                              </label>
                              <label className="flex items-center gap-1 text-xs cursor-pointer">
                                <input
                                  type="checkbox"
                                  checked={envData.frontend.localOnly}
                                  onChange={e => setEditAppForm({
                                    ...editAppForm,
                                    endpoints: {
                                      ...editAppForm.endpoints,
                                      [env.id]: { ...envData, frontend: { ...envData.frontend, localOnly: e.target.checked } }
                                    }
                                  })}
                                  className="rounded"
                                />
                                <Shield className="w-3 h-3 text-yellow-400" />
                              </label>
                            </div>
                          </div>

                          {/* APIs */}
                          {envData.apis.map((api, apiIndex) => (
                            <div key={apiIndex} className="bg-gray-900/30 rounded-lg p-3 border border-gray-700 relative">
                              {/* Bouton supprimer (sauf API par defaut) */}
                              {api.slug !== '' && (
                                <button
                                  onClick={() => removeApi(apiIndex)}
                                  className="absolute top-2 right-2 text-gray-500 hover:text-red-400 p-1"
                                  title="Supprimer"
                                >
                                  <Trash2 className="w-3 h-3" />
                                </button>
                              )}
                              <div className="text-xs text-green-400 mb-2 font-mono flex items-center gap-1">
                                <Server className="w-3 h-3" />
                                {getAppDomain(editingApp.slug, 'api', env, config?.baseDomain, api.slug)}
                              </div>
                              <div className="flex gap-2 mb-2">
                                <input
                                  type="text"
                                  value={api.targetHost}
                                  onChange={e => updateApi(apiIndex, { targetHost: e.target.value })}
                                  placeholder="localhost"
                                  className="flex-1 px-2 py-1 bg-gray-900 border border-gray-700 rounded text-sm"
                                />
                                <input
                                  type="number"
                                  value={api.targetPort}
                                  onChange={e => updateApi(apiIndex, { targetPort: e.target.value })}
                                  placeholder="3001"
                                  className="w-20 px-2 py-1 bg-gray-900 border border-gray-700 rounded text-sm"
                                />
                              </div>
                              <div className="flex gap-3">
                                <label className="flex items-center gap-1 text-xs cursor-pointer">
                                  <input
                                    type="checkbox"
                                    checked={api.requireAuth}
                                    onChange={e => updateApi(apiIndex, { requireAuth: e.target.checked })}
                                    className="rounded"
                                  />
                                  <Key className="w-3 h-3 text-purple-400" />
                                </label>
                                <label className="flex items-center gap-1 text-xs cursor-pointer">
                                  <input
                                    type="checkbox"
                                    checked={api.localOnly}
                                    onChange={e => updateApi(apiIndex, { localOnly: e.target.checked })}
                                    className="rounded"
                                  />
                                  <Shield className="w-3 h-3 text-yellow-400" />
                                </label>
                              </div>
                            </div>
                          ))}

                          {/* Bouton ajouter API */}
                          <button
                            onClick={addApi}
                            className="flex items-center justify-center gap-2 bg-gray-900/30 rounded-lg p-3 border border-dashed border-gray-700 text-gray-500 hover:text-gray-300 hover:border-gray-500 transition-colors"
                          >
                            <Plus className="w-4 h-4" />
                            <span className="text-sm">Ajouter API</span>
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => { setShowEditAppModal(false); setEditingApp(null); }}>Annuler</Button>
              <Button onClick={handleEditApp} loading={saving}>Sauvegarder</Button>
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
            <p className="text-gray-300 mb-4">Veuillez configurer un domaine de base.</p>
            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Domaine de base</label>
                <input type="text" placeholder="example.com" value={configForm.baseDomain} onChange={e => setConfigForm({ ...configForm, baseDomain: e.target.value })} className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm" />
              </div>
            </div>
            <div className="flex justify-end gap-2 mt-6">
              <Button onClick={handleSaveConfig} loading={saving} disabled={!configForm.baseDomain}>Configurer</Button>
            </div>
          </div>
        </div>
      )}

      {/* Migration Modal */}
      {showMigrationModal && migrationSuggestions && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-2xl border border-gray-700 max-h-[90vh] overflow-y-auto">
            <h2 className="text-xl font-bold mb-4 flex items-center gap-2">
              <ArrowRightLeft className="w-5 h-5 text-blue-400" />
              Migration des hosts
            </h2>
            <p className="text-gray-400 mb-4">Ces hosts seront groupes en applications:</p>

            <div className="space-y-3 mb-6">
              {migrationSuggestions.filter(s => s.type === 'application').map((s, i) => (
                <div key={i} className="bg-gray-900/50 rounded-lg p-4 border border-gray-700">
                  <div className="font-medium text-blue-400 mb-2">{s.name}</div>
                  <div className="grid grid-cols-2 gap-4 text-sm">
                    <div>
                      <span className="text-gray-500">Frontend:</span>
                      <span className="font-mono ml-2">{s.frontend.subdomain}.{config?.baseDomain}</span>
                      <span className="text-gray-500 ml-2">:{s.frontend.targetPort}</span>
                    </div>
                    <div>
                      <span className="text-gray-500">API:</span>
                      <span className="font-mono ml-2">{s.api.subdomain}.{config?.baseDomain}</span>
                      <span className="text-gray-500 ml-2">:{s.api.targetPort}</span>
                    </div>
                  </div>
                </div>
              ))}
            </div>

            {migrationSuggestions.filter(s => s.type === 'standalone').length > 0 && (
              <>
                <p className="text-gray-400 mb-2">Ces hosts resteront standalone:</p>
                <div className="flex flex-wrap gap-2 mb-6">
                  {migrationSuggestions.filter(s => s.type === 'standalone').map((s, i) => (
                    <span key={i} className="text-sm font-mono bg-gray-700 px-2 py-1 rounded">{s.host.subdomain}</span>
                  ))}
                </div>
              </>
            )}

            <div className="flex justify-end gap-2">
              <Button variant="secondary" onClick={() => setShowMigrationModal(false)}>Annuler</Button>
              <Button onClick={handleMigration} loading={migrating}>Migrer</Button>
            </div>
          </div>
        </div>
      )}

      {/* Config Modal (legacy, kept for edit button) */}
      {showConfigModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md border border-gray-700">
            <h2 className="text-xl font-bold mb-4">Configuration</h2>
            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Domaine de base</label>
                <input type="text" placeholder="example.com" value={configForm.baseDomain} onChange={e => setConfigForm({ ...configForm, baseDomain: e.target.value })} className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm" />
              </div>
            </div>
            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => setShowConfigModal(false)}>Annuler</Button>
              <Button onClick={handleSaveConfig} loading={saving}>Sauvegarder</Button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

export default ReverseProxy;
