import { useState, useEffect, useRef, useCallback } from 'react';
import {
  Boxes,
  Plus,
  Trash2,
  Pencil,
  Power,
  CheckCircle,
  XCircle,
  Server,
  Globe,
  Shield,
  Key,
  Wifi,
  WifiOff,
  Clock,
  Container,
  RefreshCw,
  Copy,
  AlertTriangle,
  X,
  Terminal,
  Code2,
  Loader2,
  Play,
  Square,
  Cpu,
  HardDrive,
  Database,
} from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import PageHeader from '../components/PageHeader';
import {
  getApplications,
  createApplication,
  updateApplication,
  deleteApplication,
  toggleApplication,
  getReverseProxyConfig,
  getUserGroups,
  startApplicationService,
  stopApplicationService,
} from '../api/client';

const STATUS_BADGES = {
  connected: { color: 'text-green-400 bg-green-900/30', icon: Wifi, label: 'Connecte' },
  deploying: { color: 'text-blue-400 bg-blue-900/30', icon: Loader2, label: 'Deploiement', spin: true },
  pending: { color: 'text-yellow-400 bg-yellow-900/30', icon: Clock, label: 'En attente' },
  disconnected: { color: 'text-red-400 bg-red-900/30', icon: WifiOff, label: 'Deconnecte' },
  error: { color: 'text-red-400 bg-red-900/30', icon: AlertTriangle, label: 'Erreur' },
};

function StatusBadge({ status, message }) {
  const badge = STATUS_BADGES[status] || STATUS_BADGES.disconnected;
  const Icon = badge.icon;
  return (
    <span className={`flex items-center gap-1 text-xs px-2 py-0.5 ${badge.color}`}>
      <Icon className={`w-3 h-3 ${badge.spin ? 'animate-spin' : ''}`} />
      {message || badge.label}
    </span>
  );
}

function Applications() {
  const [applications, setApplications] = useState([]);
  const [baseDomain, setBaseDomain] = useState('');
  const [userGroups, setUserGroups] = useState([]);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState(null);
  const [saving, setSaving] = useState(false);

  // Modal states
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [showEditModal, setShowEditModal] = useState(false);
  const [editingApp, setEditingApp] = useState(null);
  const [tokenModal, setTokenModal] = useState(null); // { name, token }
  const [terminalApp, setTerminalApp] = useState(null); // app object for terminal modal

  // Create form
  const [createForm, setCreateForm] = useState({
    name: '',
    slug: '',
    frontend: { target_port: '', auth_required: false, allowed_groups: [], local_only: false },
    apis: [],
    code_server_enabled: true,
  });

  // Edit form
  const [editForm, setEditForm] = useState(null);

  // Agent metrics state: { [appId]: { codeServerStatus, appStatus, dbStatus, memoryBytes, cpuPercent, ... } }
  const [appMetrics, setAppMetrics] = useState({});

  // WebSocket for real-time updates
  const wsRef = useRef(null);

  const fetchData = useCallback(async () => {
    try {
      const [appsRes, configRes, groupsRes] = await Promise.all([
        getApplications(),
        getReverseProxyConfig(),
        getUserGroups().catch(() => ({ data: { success: false } })),
      ]);
      if (appsRes.data.success) setApplications(appsRes.data.applications || []);
      if (configRes.data.success) setBaseDomain(configRes.data.config?.baseDomain || '');
      if (groupsRes.data?.success) setUserGroups(groupsRes.data.groups || []);
    } catch (error) {
      console.error('Error:', error);
      setMessage({ type: 'error', text: 'Erreur de chargement' });
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  // WebSocket connection for agent status updates
  const fetchDataRef = useRef(fetchData);
  fetchDataRef.current = fetchData;

  useEffect(() => {
    const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const ws = new WebSocket(`${proto}//${window.location.host}/api/ws`);
    wsRef.current = ws;

    ws.onmessage = (event) => {
      try {
        const msg = JSON.parse(event.data);
        if (msg.type === 'agent:status') {
          const { appId, status, message: stepMsg } = msg.data;
          setApplications(prev => {
            const old = prev.find(a => a.id === appId);
            const wasDeploying = old && (old.status === 'deploying' || old._deployMessage);
            const nowReady = status === 'connected' || (status === 'pending' && wasDeploying);
            // If transitioning out of deploying, refresh full data to get IP, version, etc.
            if (wasDeploying && nowReady) {
              setTimeout(() => fetchDataRef.current(), 500);
            }
            return prev.map(app =>
              app.id === appId
                ? { ...app, status, _deployMessage: status === 'deploying' ? (stepMsg || null) : null }
                : app
            );
          });
        } else if (msg.type === 'agent:metrics') {
          // Update agent metrics for an application
          const { appId, codeServerStatus, appStatus, dbStatus, memoryBytes, cpuPercent, codeServerIdleSecs, appIdleSecs } = msg.data;
          setAppMetrics(prev => ({
            ...prev,
            [appId]: { codeServerStatus, appStatus, dbStatus, memoryBytes, cpuPercent, codeServerIdleSecs, appIdleSecs }
          }));
        } else if (msg.type === 'agent:service-command') {
          // Service state changed - update metrics immediately for instant UI feedback
          const { appId, serviceType, action, success } = msg.data;
          if (success && appId) {
            // Map action to status: started->running, stopped->stopped, starting->starting, stopping->stopping
            const statusMap = { started: 'running', stopped: 'stopped', starting: 'starting', stopping: 'stopping' };
            const newStatus = statusMap[action] || action;

            // Update the correct status field based on serviceType
            setAppMetrics(prev => {
              const current = prev[appId] || {};
              const updated = { ...current };
              if (serviceType === 'app') updated.appStatus = newStatus;
              else if (serviceType === 'db') updated.dbStatus = newStatus;
              else if (serviceType === 'codeserver') updated.codeServerStatus = newStatus;
              return { ...prev, [appId]: updated };
            });
          }
        }
      } catch {}
    };

    return () => {
      ws.close();
    };
  }, []);

  // Auto-dismiss messages
  useEffect(() => {
    if (message) {
      const timer = setTimeout(() => setMessage(null), 4000);
      return () => clearTimeout(timer);
    }
  }, [message]);

  async function handleCreate() {
    if (!createForm.name || !createForm.slug) {
      setMessage({ type: 'error', text: 'Nom et slug requis' });
      return;
    }
    if (!createForm.frontend.target_port) {
      setMessage({ type: 'error', text: 'Port frontend requis' });
      return;
    }
    const port = parseInt(createForm.frontend.target_port);
    if (isNaN(port) || port < 1 || port > 65535) {
      setMessage({ type: 'error', text: 'Port frontend invalide (1-65535)' });
      return;
    }
    for (const api of createForm.apis) {
      if (!api.slug || !api.target_port) {
        setMessage({ type: 'error', text: 'Slug et port requis pour chaque API' });
        return;
      }
      const apiPort = parseInt(api.target_port);
      if (isNaN(apiPort) || apiPort < 1 || apiPort > 65535) {
        setMessage({ type: 'error', text: `Port API invalide pour "${api.slug}" (1-65535)` });
        return;
      }
    }

    setSaving(true);
    try {
      const payload = {
        name: createForm.name,
        slug: createForm.slug.toLowerCase(),
        frontend: {
          target_port: parseInt(createForm.frontend.target_port),
          auth_required: createForm.frontend.auth_required,
          allowed_groups: createForm.frontend.allowed_groups,
          local_only: createForm.frontend.local_only,
        },
        apis: createForm.apis.map(a => ({
          slug: a.slug.toLowerCase(),
          target_port: parseInt(a.target_port),
          auth_required: a.auth_required,
          allowed_groups: a.allowed_groups || [],
          local_only: a.local_only || false,
        })),
        code_server_enabled: createForm.code_server_enabled,
      };

      const res = await createApplication(payload);
      if (res.data.success) {
        setShowCreateModal(false);
        setCreateForm({
          name: '', slug: '',
          frontend: { target_port: '', auth_required: false, allowed_groups: [], local_only: false },
          apis: [],
          code_server_enabled: true,
        });
        if (res.data.token) {
          setTokenModal({ name: createForm.name, token: res.data.token });
        }
        setMessage({ type: 'success', text: 'Application creee' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error || 'Erreur' });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur' });
    } finally {
      setSaving(false);
    }
  }

  async function handleEdit() {
    if (!editForm) return;
    setSaving(true);
    try {
      // Parse comma-separated services into arrays
      const parseServices = (str) => str ? str.split(',').map(s => s.trim()).filter(Boolean) : [];

      const payload = {
        name: editForm.name,
        frontend: {
          target_port: parseInt(editForm.frontend.target_port),
          auth_required: editForm.frontend.auth_required,
          allowed_groups: editForm.frontend.allowed_groups,
          local_only: editForm.frontend.local_only,
        },
        apis: editForm.apis.map(a => ({
          slug: a.slug.toLowerCase(),
          target_port: parseInt(a.target_port),
          auth_required: a.auth_required,
          allowed_groups: a.allowed_groups || [],
          local_only: a.local_only || false,
        })),
        code_server_enabled: editForm.code_server_enabled,
        services: {
          app: parseServices(editForm.services?.app),
          db: parseServices(editForm.services?.db),
        },
        power_policy: {
          code_server_idle_timeout_secs: editForm.powerPolicy?.codeServerTimeoutMins
            ? editForm.powerPolicy.codeServerTimeoutMins * 60
            : null,
          app_idle_timeout_secs: editForm.powerPolicy?.appTimeoutMins
            ? editForm.powerPolicy.appTimeoutMins * 60
            : null,
        },
      };

      const res = await updateApplication(editingApp.id, payload);
      if (res.data.success) {
        setShowEditModal(false);
        setEditingApp(null);
        setEditForm(null);
        setMessage({ type: 'success', text: 'Application modifiee' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error || 'Erreur' });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur' });
    } finally {
      setSaving(false);
    }
  }

  async function handleToggle(id, enabled) {
    try {
      const res = await toggleApplication(id, enabled);
      if (res.data.success) fetchData();
      else setMessage({ type: 'error', text: res.data.error });
    } catch {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  async function handleDelete(id, name) {
    if (!confirm(`Supprimer "${name}" ?\nCeci detruira le conteneur LXC, les enregistrements DNS et les certificats.`)) return;
    try {
      const res = await deleteApplication(id);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Application supprimee' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  function openEditModal(app) {
    setEditingApp(app);
    setEditForm({
      name: app.name,
      frontend: { ...app.frontend, target_port: String(app.frontend.target_port) },
      apis: (app.apis || []).map(a => ({ ...a, target_port: String(a.target_port) })),
      code_server_enabled: app.code_server_enabled !== false,
      services: {
        app: (app.services?.app || []).join(', '),
        db: (app.services?.db || []).join(', '),
      },
      powerPolicy: {
        codeServerTimeoutMins: Math.floor((app.power_policy?.code_server_idle_timeout_secs || 0) / 60),
        appTimeoutMins: Math.floor((app.power_policy?.app_idle_timeout_secs || 0) / 60),
      },
    });
    setShowEditModal(true);
  }

  async function copyToken(token) {
    try {
      await navigator.clipboard.writeText(token);
      setMessage({ type: 'success', text: 'Token copie' });
    } catch {
      setMessage({ type: 'error', text: 'Echec de la copie' });
    }
  }

  async function handleServiceStart(appId, serviceType) {
    try {
      const res = await startApplicationService(appId, serviceType);
      if (!res.data.success) {
        setMessage({ type: 'error', text: res.data.error || 'Erreur' });
      }
      // Success notification will come via WebSocket
    } catch {
      setMessage({ type: 'error', text: 'Erreur de connexion' });
    }
  }

  async function handleServiceStop(appId, serviceType) {
    try {
      const res = await stopApplicationService(appId, serviceType);
      if (!res.data.success) {
        setMessage({ type: 'error', text: res.data.error || 'Erreur' });
      }
      // Success notification will come via WebSocket
    } catch {
      setMessage({ type: 'error', text: 'Erreur de connexion' });
    }
  }

  // Start stack: db first, then app (with delay)
  async function handleStackStart(appId) {
    try {
      // Start DB first
      await startApplicationService(appId, 'db');
      // Wait 500ms then start app
      setTimeout(async () => {
        await startApplicationService(appId, 'app');
      }, 500);
    } catch {
      setMessage({ type: 'error', text: 'Erreur de connexion' });
    }
  }

  // Stop stack: app first, then db (with delay)
  async function handleStackStop(appId) {
    try {
      // Stop app first
      await stopApplicationService(appId, 'app');
      // Wait 500ms then stop db
      setTimeout(async () => {
        await stopApplicationService(appId, 'db');
      }, 500);
    } catch {
      setMessage({ type: 'error', text: 'Erreur de connexion' });
    }
  }

  // Get combined stack status from app and db
  function getStackStatus(appStatus, dbStatus) {
    if (appStatus === 'running' && dbStatus === 'running') return 'running';
    if (appStatus === 'stopped' && dbStatus === 'stopped') return 'stopped';
    if (appStatus === 'starting' || dbStatus === 'starting') return 'starting';
    if (appStatus === 'stopping' || dbStatus === 'stopping') return 'stopping';
    // Mixed state (one running, one stopped) - show as partial
    if (appStatus === 'running' || dbStatus === 'running') return 'partial';
    return 'stopped';
  }

  // Format bytes to human readable
  function formatBytes(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  return (
    <div>
      <PageHeader title="Applications" icon={Boxes}>
        <Button onClick={fetchData} variant="secondary">
          <RefreshCw className="w-4 h-4" />
          Rafraichir
        </Button>
        <Button onClick={() => setShowCreateModal(true)} disabled={!baseDomain}>
          <Plus className="w-4 h-4" />
          Nouvelle application
        </Button>
      </PageHeader>

      {/* Message */}
      {message && (
        <div className={`p-4 flex items-center gap-2 ${
          message.type === 'success' ? 'bg-green-900/50 text-green-400' : 'bg-red-900/50 text-red-400'
        }`}>
          {message.type === 'success' ? <CheckCircle className="w-5 h-5" /> : <XCircle className="w-5 h-5" />}
          {message.text}
        </div>
      )}

      {/* Stats */}
      <div className="grid grid-cols-1 md:grid-cols-3 gap-px">
        <Card title="Applications" icon={Boxes}>
          <div className="text-2xl font-bold">{applications.length}</div>
        </Card>
        <Card title="Agents connectes" icon={Wifi}>
          <div className="text-2xl font-bold text-green-400">
            {applications.filter(a => a.status === 'connected').length}
          </div>
        </Card>
        <Card title="Domaine" icon={Globe}>
          <div className="text-lg font-mono text-blue-400 truncate">
            {baseDomain || 'Non configure'}
          </div>
        </Card>
      </div>

      {/* Applications Table */}
      <Card>
        {applications.length === 0 ? (
          <div className="text-center py-8 text-gray-500">
            <Boxes className="w-12 h-12 mx-auto mb-2 opacity-50" />
            <p>Aucune application</p>
            <p className="text-xs mt-2">Creez une application pour deployer un conteneur LXC avec un agent</p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full">
              <thead>
                <tr className="text-xs text-gray-500 border-b border-gray-700">
                  <th className="text-left py-2 px-3 font-medium">Application</th>
                  <th className="text-left py-2 px-3 font-medium">Status</th>
                  <th className="text-left py-2 px-3 font-medium">Services</th>
                  <th className="text-right py-2 px-3 font-medium">CPU</th>
                  <th className="text-right py-2 px-3 font-medium">RAM</th>
                  <th className="text-right py-2 px-3 font-medium">Actions</th>
                </tr>
              </thead>
              <tbody>
                {applications.map(app => {
                  const isDeploying = app.status === 'deploying';
                  const metrics = appMetrics[app.id];
                  return (
                    <tr key={app.id} className="border-b border-gray-800 hover:bg-gray-800/30">
                      {/* Application info */}
                      <td className="py-3 px-3">
                        <div className="flex items-center gap-2">
                          {isDeploying ? (
                            <Loader2 className="w-4 h-4 text-blue-400 animate-spin flex-shrink-0" />
                          ) : (
                            <Container className="w-4 h-4 text-blue-400 flex-shrink-0" />
                          )}
                          <div className="min-w-0">
                            <div className="flex items-center gap-2">
                              <span className="font-medium truncate">{app.name}</span>
                              {!app.enabled && (
                                <span className="text-xs text-gray-500 bg-gray-800 px-1.5 py-0.5">off</span>
                              )}
                            </div>
                            <div className="flex items-center gap-2 text-xs text-gray-500">
                              <a
                                href={`https://${app.slug}.${baseDomain}`}
                                target="_blank"
                                rel="noopener noreferrer"
                                className="font-mono hover:text-blue-400"
                              >
                                {app.slug}.{baseDomain}
                              </a>
                              {app.frontend.auth_required && <Key className="w-3 h-3 text-purple-400" />}
                              {app.frontend.local_only && <Shield className="w-3 h-3 text-yellow-400" />}
                            </div>
                          </div>
                        </div>
                      </td>

                      {/* Status */}
                      <td className="py-3 px-3">
                        {isDeploying ? (
                          <div>
                            <span className="text-xs px-2 py-0.5 text-blue-400 bg-blue-900/30">Deploiement</span>
                            <p className="text-xs text-gray-500 mt-1 truncate max-w-32">{app._deployMessage}</p>
                          </div>
                        ) : (
                          <StatusBadge status={app.status} />
                        )}
                      </td>

                      {/* Services */}
                      <td className="py-3 px-3">
                        {!metrics ? (
                          <span className="text-xs text-gray-600">-</span>
                        ) : (
                          <div className="flex items-center gap-2 text-xs">
                            {/* code-server */}
                            {app.code_server_enabled !== false && (
                              <button
                                onClick={() => metrics.codeServerStatus === 'running'
                                  ? handleServiceStop(app.id, 'code-server')
                                  : handleServiceStart(app.id, 'code-server')
                                }
                                className={`flex items-center gap-1 px-1.5 py-0.5 transition-colors ${
                                  metrics.codeServerStatus === 'running'
                                    ? 'text-green-400 bg-green-900/30 hover:bg-green-900/50'
                                    : metrics.codeServerStatus === 'starting'
                                    ? 'text-blue-400 bg-blue-900/30'
                                    : 'text-gray-400 bg-gray-700/30 hover:bg-gray-700/50'
                                }`}
                                title={`code-server: ${metrics.codeServerStatus}`}
                              >
                                <Code2 className="w-3 h-3" />
                                {metrics.codeServerStatus === 'running' ? <Square className="w-2.5 h-2.5" /> : <Play className="w-2.5 h-2.5" />}
                              </button>
                            )}
                            {/* Stack (App + DB combined) */}
                            {(app.services?.app?.length > 0 || app.services?.db?.length > 0) && (() => {
                              const stackStatus = getStackStatus(metrics.appStatus, metrics.dbStatus);
                              const isRunning = stackStatus === 'running' || stackStatus === 'partial';
                              return (
                                <button
                                  onClick={() => isRunning
                                    ? handleStackStop(app.id)
                                    : handleStackStart(app.id)
                                  }
                                  className={`flex items-center gap-1 px-1.5 py-0.5 transition-colors ${
                                    stackStatus === 'running'
                                      ? 'text-green-400 bg-green-900/30 hover:bg-green-900/50'
                                      : stackStatus === 'partial'
                                      ? 'text-yellow-400 bg-yellow-900/30 hover:bg-yellow-900/50'
                                      : stackStatus === 'starting' || stackStatus === 'stopping'
                                      ? 'text-blue-400 bg-blue-900/30'
                                      : 'text-gray-400 bg-gray-700/30 hover:bg-gray-700/50'
                                  }`}
                                  title={`stack: app=${metrics.appStatus}, db=${metrics.dbStatus}`}
                                >
                                  <Server className="w-3 h-3" />
                                  <Database className="w-3 h-3" />
                                  {isRunning ? <Square className="w-2.5 h-2.5" /> : <Play className="w-2.5 h-2.5" />}
                                </button>
                              );
                            })()}
                          </div>
                        )}
                      </td>

                      {/* CPU */}
                      <td className="py-3 px-3 text-right">
                        <span className={`font-mono text-sm ${
                          metrics?.cpuPercent > 80 ? 'text-red-400' :
                          metrics?.cpuPercent > 50 ? 'text-yellow-400' :
                          metrics?.cpuPercent > 0 ? 'text-green-400' : 'text-gray-600'
                        }`}>
                          {metrics?.cpuPercent !== undefined ? `${metrics.cpuPercent.toFixed(1)}%` : '-'}
                        </span>
                      </td>

                      {/* RAM */}
                      <td className="py-3 px-3 text-right">
                        <span className="font-mono text-sm text-gray-400">
                          {metrics?.memoryBytes ? formatBytes(metrics.memoryBytes) : '-'}
                        </span>
                      </td>

                      {/* Actions */}
                      <td className="py-3 px-3">
                        <div className="flex items-center justify-end gap-1">
                          {app.code_server_enabled !== false && baseDomain && (
                            <a
                              href={`https://${app.slug}.code.${baseDomain}`}
                              target="_blank"
                              rel="noopener noreferrer"
                              className="p-1.5 text-cyan-400 hover:text-cyan-300 hover:bg-cyan-900/30 transition-colors"
                              title="IDE"
                            >
                              <Code2 className="w-4 h-4" />
                            </a>
                          )}
                          <button
                            onClick={() => setTerminalApp(app)}
                            className="p-1.5 text-emerald-400 hover:text-emerald-300 hover:bg-emerald-900/30 transition-colors"
                            title="Terminal"
                          >
                            <Terminal className="w-4 h-4" />
                          </button>
                          <button
                            onClick={() => handleToggle(app.id, !app.enabled)}
                            className={`p-1.5 transition-colors ${
                              app.enabled ? 'text-green-400 hover:bg-green-900/30' : 'text-gray-500 hover:bg-gray-700/30'
                            }`}
                            title={app.enabled ? 'Desactiver' : 'Activer'}
                          >
                            <Power className="w-4 h-4" />
                          </button>
                          <button
                            onClick={() => openEditModal(app)}
                            className="p-1.5 text-blue-400 hover:text-blue-300 hover:bg-blue-900/30 transition-colors"
                            title="Modifier"
                          >
                            <Pencil className="w-4 h-4" />
                          </button>
                          <button
                            onClick={() => handleDelete(app.id, app.name)}
                            className="p-1.5 text-red-400 hover:text-red-300 hover:bg-red-900/30 transition-colors"
                            title="Supprimer"
                          >
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

      {/* Create Modal */}
      {showCreateModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 p-6 w-full max-w-2xl border border-gray-700 max-h-[90vh] overflow-y-auto">
            <h2 className="text-xl font-bold mb-4">Nouvelle application</h2>
            <div className="space-y-4">
              {/* Name + Slug */}
              <div className="grid grid-cols-2 gap-4">
                <div>
                  <label className="block text-sm text-gray-400 mb-1">Nom</label>
                  <input
                    type="text"
                    placeholder="Mon App"
                    value={createForm.name}
                    onChange={e => {
                      const name = e.target.value;
                      const autoSlug = name.toLowerCase().replace(/[\s_]+/g, '-').replace(/[^a-z0-9-]/g, '').replace(/-+/g, '-').replace(/^-|-$/g, '');
                      setCreateForm(f => ({ ...f, name, slug: f.slug === '' || f.slug === f.name.toLowerCase().replace(/[\s_]+/g, '-').replace(/[^a-z0-9-]/g, '').replace(/-+/g, '-').replace(/^-|-$/g, '') ? autoSlug : f.slug }));
                    }}
                    className="w-full px-3 py-2 bg-gray-900 border border-gray-600 text-sm"
                  />
                </div>
                <div>
                  <label className="block text-sm text-gray-400 mb-1">Slug</label>
                  <input
                    type="text"
                    placeholder="mon-app"
                    value={createForm.slug}
                    onChange={e => setCreateForm({ ...createForm, slug: e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '') })}
                    className="w-full px-3 py-2 bg-gray-900 border border-gray-600 text-sm font-mono"
                  />
                  {createForm.slug && baseDomain && (
                    <p className="text-xs text-gray-500 mt-1 font-mono">{createForm.slug}.{baseDomain}</p>
                  )}
                </div>
              </div>

              {/* Frontend */}
              <div className="border border-gray-700 p-4">
                <div className="text-sm font-medium mb-3 flex items-center gap-2">
                  <Globe className="w-4 h-4 text-blue-400" />
                  Frontend
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Port local</label>
                    <input
                      type="number"
                      placeholder="3000"
                      value={createForm.frontend.target_port}
                      onChange={e => setCreateForm({ ...createForm, frontend: { ...createForm.frontend, target_port: e.target.value } })}
                      className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 text-sm"
                    />
                  </div>
                  <div className="flex items-end gap-4">
                    <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                      <input
                        type="checkbox"
                        checked={createForm.frontend.auth_required}
                        onChange={e => setCreateForm({ ...createForm, frontend: { ...createForm.frontend, auth_required: e.target.checked } })}
                        className="rounded"
                      />
                      <Key className="w-3 h-3 text-purple-400" /> Auth
                    </label>
                    <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                      <input
                        type="checkbox"
                        checked={createForm.frontend.local_only}
                        onChange={e => setCreateForm({ ...createForm, frontend: { ...createForm.frontend, local_only: e.target.checked } })}
                        className="rounded"
                      />
                      <Shield className="w-3 h-3 text-yellow-400" /> Local
                    </label>
                  </div>
                </div>
              </div>

              {/* code-server */}
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input
                  type="checkbox"
                  checked={createForm.code_server_enabled}
                  onChange={e => setCreateForm({ ...createForm, code_server_enabled: e.target.checked })}
                  className="rounded"
                />
                <Code2 className="w-4 h-4 text-cyan-400" />
                code-server IDE
                {createForm.slug && baseDomain && createForm.code_server_enabled && (
                  <span className="text-xs text-gray-500 font-mono ml-2">{createForm.slug}.code.{baseDomain}</span>
                )}
              </label>

              {/* APIs */}
              <div className="border border-gray-700 p-4">
                <div className="flex items-center justify-between mb-3">
                  <div className="text-sm font-medium flex items-center gap-2">
                    <Server className="w-4 h-4 text-green-400" />
                    APIs
                  </div>
                  <Button
                    variant="secondary"
                    onClick={() => setCreateForm({
                      ...createForm,
                      apis: [...createForm.apis, { slug: '', target_port: '', auth_required: false, allowed_groups: [], local_only: false }]
                    })}
                  >
                    <Plus className="w-3 h-3" /> Ajouter API
                  </Button>
                </div>
                {createForm.apis.length === 0 ? (
                  <p className="text-xs text-gray-500">Aucune API configuree</p>
                ) : (
                  <div className="space-y-3">
                    {createForm.apis.map((api, i) => (
                      <div key={i} className="flex items-start gap-3 bg-gray-900/30 p-3 border border-gray-700">
                        <div className="flex-1 grid grid-cols-3 gap-2">
                          <div>
                            <label className="block text-xs text-gray-500 mb-1">Slug</label>
                            <input
                              type="text"
                              placeholder="api"
                              value={api.slug}
                              onChange={e => {
                                const apis = [...createForm.apis];
                                apis[i] = { ...apis[i], slug: e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '') };
                                setCreateForm({ ...createForm, apis });
                              }}
                              className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 text-sm font-mono"
                            />
                            {api.slug && createForm.slug && (
                              <p className="text-xs text-gray-500 mt-0.5 font-mono">{createForm.slug}-{api.slug}.{baseDomain}</p>
                            )}
                          </div>
                          <div>
                            <label className="block text-xs text-gray-500 mb-1">Port</label>
                            <input
                              type="number"
                              placeholder="3001"
                              value={api.target_port}
                              onChange={e => {
                                const apis = [...createForm.apis];
                                apis[i] = { ...apis[i], target_port: e.target.value };
                                setCreateForm({ ...createForm, apis });
                              }}
                              className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 text-sm"
                            />
                          </div>
                          <div className="flex items-end gap-3">
                            <label className="flex items-center gap-1 text-xs cursor-pointer">
                              <input
                                type="checkbox"
                                checked={api.auth_required}
                                onChange={e => {
                                  const apis = [...createForm.apis];
                                  apis[i] = { ...apis[i], auth_required: e.target.checked };
                                  setCreateForm({ ...createForm, apis });
                                }}
                                className="rounded"
                              />
                              <Key className="w-3 h-3 text-purple-400" />
                            </label>
                            <label className="flex items-center gap-1 text-xs cursor-pointer">
                              <input
                                type="checkbox"
                                checked={api.local_only || false}
                                onChange={e => {
                                  const apis = [...createForm.apis];
                                  apis[i] = { ...apis[i], local_only: e.target.checked };
                                  setCreateForm({ ...createForm, apis });
                                }}
                                className="rounded"
                              />
                              <Shield className="w-3 h-3 text-yellow-400" />
                            </label>
                          </div>
                        </div>
                        <button
                          onClick={() => setCreateForm({ ...createForm, apis: createForm.apis.filter((_, j) => j !== i) })}
                          className="text-gray-500 hover:text-red-400 p-1 mt-5"
                        >
                          <Trash2 className="w-4 h-4" />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              {/* Allowed Groups */}
              {userGroups.length > 0 && (
                <div>
                  <label className="block text-sm text-gray-400 mb-2">Acces restreint par groupe (frontend)</label>
                  <div className="flex flex-wrap gap-2">
                    {userGroups.filter(g => g.id !== 'admins').map(group => {
                      const selected = createForm.frontend.allowed_groups.includes(group.id);
                      return (
                        <button
                          key={group.id}
                          type="button"
                          onClick={() => {
                            const groups = selected
                              ? createForm.frontend.allowed_groups.filter(id => id !== group.id)
                              : [...createForm.frontend.allowed_groups, group.id];
                            setCreateForm({ ...createForm, frontend: { ...createForm.frontend, allowed_groups: groups } });
                          }}
                          className={`flex items-center gap-1.5 px-3 py-1.5 text-xs border transition-all ${
                            selected
                              ? 'border-white/30 bg-white/10 text-white'
                              : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-500'
                          }`}
                        >
                          <span className="w-2.5 h-2.5" style={{ backgroundColor: group.color }} />
                          {group.name}
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}
            </div>
            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => setShowCreateModal(false)}>Annuler</Button>
              <Button onClick={handleCreate} loading={saving}>Creer</Button>
            </div>
          </div>
        </div>
      )}

      {/* Edit Modal */}
      {showEditModal && editingApp && editForm && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 p-6 w-full max-w-2xl border border-gray-700 max-h-[90vh] overflow-y-auto">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-xl font-bold">Modifier {editingApp.name}</h2>
              <span className="text-xs text-gray-500 bg-gray-900/50 px-2 py-1 font-mono">{editingApp.slug}</span>
            </div>
            <div className="space-y-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Nom d&apos;affichage</label>
                <input
                  type="text"
                  value={editForm.name}
                  onChange={e => setEditForm({ ...editForm, name: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 text-sm"
                />
              </div>

              {/* Frontend */}
              <div className="border border-gray-700 p-4">
                <div className="text-xs text-blue-400 mb-2 font-mono flex items-center gap-1">
                  <Globe className="w-3 h-3" />
                  {editingApp.slug}.{baseDomain}
                </div>
                <div className="grid grid-cols-2 gap-3">
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Port local</label>
                    <input
                      type="number"
                      value={editForm.frontend.target_port}
                      onChange={e => setEditForm({ ...editForm, frontend: { ...editForm.frontend, target_port: e.target.value } })}
                      className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 text-sm"
                    />
                  </div>
                  <div className="flex items-end gap-4">
                    <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                      <input
                        type="checkbox"
                        checked={editForm.frontend.auth_required}
                        onChange={e => setEditForm({ ...editForm, frontend: { ...editForm.frontend, auth_required: e.target.checked } })}
                        className="rounded"
                      />
                      <Key className="w-3 h-3 text-purple-400" /> Auth
                    </label>
                    <label className="flex items-center gap-1.5 text-xs cursor-pointer">
                      <input
                        type="checkbox"
                        checked={editForm.frontend.local_only}
                        onChange={e => setEditForm({ ...editForm, frontend: { ...editForm.frontend, local_only: e.target.checked } })}
                        className="rounded"
                      />
                      <Shield className="w-3 h-3 text-yellow-400" /> Local
                    </label>
                  </div>
                </div>
              </div>

              {/* code-server */}
              <label className="flex items-center gap-2 text-sm cursor-pointer">
                <input
                  type="checkbox"
                  checked={editForm.code_server_enabled}
                  onChange={e => setEditForm({ ...editForm, code_server_enabled: e.target.checked })}
                  className="rounded"
                />
                <Code2 className="w-4 h-4 text-cyan-400" />
                code-server IDE
                {baseDomain && editForm.code_server_enabled && (
                  <span className="text-xs text-gray-500 font-mono ml-2">{editingApp.slug}.code.{baseDomain}</span>
                )}
              </label>

              {/* APIs */}
              <div className="border border-gray-700 p-4">
                <div className="flex items-center justify-between mb-3">
                  <div className="text-sm font-medium flex items-center gap-2">
                    <Server className="w-4 h-4 text-green-400" />
                    APIs
                  </div>
                  <Button
                    variant="secondary"
                    onClick={() => setEditForm({
                      ...editForm,
                      apis: [...editForm.apis, { slug: '', target_port: '', auth_required: false, allowed_groups: [], local_only: false }]
                    })}
                  >
                    <Plus className="w-3 h-3" /> Ajouter API
                  </Button>
                </div>
                {editForm.apis.length === 0 ? (
                  <p className="text-xs text-gray-500">Aucune API</p>
                ) : (
                  <div className="space-y-3">
                    {editForm.apis.map((api, i) => (
                      <div key={i} className="flex items-start gap-3 bg-gray-900/30 p-3 border border-gray-700">
                        <div className="flex-1 grid grid-cols-3 gap-2">
                          <div>
                            <label className="block text-xs text-gray-500 mb-1">Slug</label>
                            <input
                              type="text"
                              value={api.slug}
                              onChange={e => {
                                const apis = [...editForm.apis];
                                apis[i] = { ...apis[i], slug: e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '') };
                                setEditForm({ ...editForm, apis });
                              }}
                              className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 text-sm font-mono"
                            />
                            {api.slug && (
                              <p className="text-xs text-gray-500 mt-0.5 font-mono">{editingApp.slug}-{api.slug}.{baseDomain}</p>
                            )}
                          </div>
                          <div>
                            <label className="block text-xs text-gray-500 mb-1">Port</label>
                            <input
                              type="number"
                              value={api.target_port}
                              onChange={e => {
                                const apis = [...editForm.apis];
                                apis[i] = { ...apis[i], target_port: e.target.value };
                                setEditForm({ ...editForm, apis });
                              }}
                              className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 text-sm"
                            />
                          </div>
                          <div className="flex items-end gap-3">
                            <label className="flex items-center gap-1 text-xs cursor-pointer">
                              <input
                                type="checkbox"
                                checked={api.auth_required}
                                onChange={e => {
                                  const apis = [...editForm.apis];
                                  apis[i] = { ...apis[i], auth_required: e.target.checked };
                                  setEditForm({ ...editForm, apis });
                                }}
                                className="rounded"
                              />
                              <Key className="w-3 h-3 text-purple-400" />
                            </label>
                            <label className="flex items-center gap-1 text-xs cursor-pointer">
                              <input
                                type="checkbox"
                                checked={api.local_only || false}
                                onChange={e => {
                                  const apis = [...editForm.apis];
                                  apis[i] = { ...apis[i], local_only: e.target.checked };
                                  setEditForm({ ...editForm, apis });
                                }}
                                className="rounded"
                              />
                              <Shield className="w-3 h-3 text-yellow-400" />
                            </label>
                          </div>
                        </div>
                        <button
                          onClick={() => setEditForm({ ...editForm, apis: editForm.apis.filter((_, j) => j !== i) })}
                          className="text-gray-500 hover:text-red-400 p-1 mt-5"
                        >
                          <Trash2 className="w-4 h-4" />
                        </button>
                      </div>
                    ))}
                  </div>
                )}
              </div>

              {/* Allowed Groups */}
              {userGroups.length > 0 && (
                <div>
                  <label className="block text-sm text-gray-400 mb-2">Acces restreint par groupe (frontend)</label>
                  <div className="flex flex-wrap gap-2">
                    {userGroups.filter(g => g.id !== 'admins').map(group => {
                      const selected = (editForm.frontend.allowed_groups || []).includes(group.id);
                      return (
                        <button
                          key={group.id}
                          type="button"
                          onClick={() => {
                            const groups = selected
                              ? editForm.frontend.allowed_groups.filter(id => id !== group.id)
                              : [...(editForm.frontend.allowed_groups || []), group.id];
                            setEditForm({ ...editForm, frontend: { ...editForm.frontend, allowed_groups: groups } });
                          }}
                          className={`flex items-center gap-1.5 px-3 py-1.5 text-xs border transition-all ${
                            selected
                              ? 'border-white/30 bg-white/10 text-white'
                              : 'border-gray-700 bg-gray-800/50 text-gray-400 hover:border-gray-500'
                          }`}
                        >
                          <span className="w-2.5 h-2.5" style={{ backgroundColor: group.color }} />
                          {group.name}
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}

              {/* Systemd Services (powersave) */}
              <div className="border border-gray-700 p-4">
                <div className="text-sm font-medium mb-3 flex items-center gap-2">
                  <Server className="w-4 h-4 text-orange-400" />
                  Services systemd (powersave)
                </div>
                <p className="text-xs text-gray-500 mb-3">
                  Definir les services systemd a demarrer/arreter. Separez par des virgules.
                </p>
                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Services App</label>
                    <input
                      type="text"
                      placeholder="myapp.service, myapp-worker.service"
                      value={editForm.services?.app || ''}
                      onChange={e => setEditForm({ ...editForm, services: { ...editForm.services, app: e.target.value } })}
                      className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 text-sm font-mono"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Services DB</label>
                    <input
                      type="text"
                      placeholder="postgresql.service"
                      value={editForm.services?.db || ''}
                      onChange={e => setEditForm({ ...editForm, services: { ...editForm.services, db: e.target.value } })}
                      className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 text-sm font-mono"
                    />
                  </div>
                </div>
              </div>

              {/* Power Policy - Idle Timeouts */}
              <div className="border border-gray-700 p-4">
                <div className="text-sm font-medium mb-3 flex items-center gap-2">
                  <Clock className="w-4 h-4 text-purple-400" />
                  Powersave - Timeouts d&apos;inactivite
                </div>
                <p className="text-xs text-gray-500 mb-3">
                  Arret automatique apres inactivite. 0 = desactive.
                </p>
                <div className="grid grid-cols-2 gap-4">
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">Code-server (minutes)</label>
                    <input
                      type="number"
                      min="0"
                      value={editForm.powerPolicy?.codeServerTimeoutMins || 0}
                      onChange={e => setEditForm({
                        ...editForm,
                        powerPolicy: { ...editForm.powerPolicy, codeServerTimeoutMins: parseInt(e.target.value) || 0 }
                      })}
                      className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 text-sm"
                    />
                  </div>
                  <div>
                    <label className="block text-xs text-gray-500 mb-1">App/DB (minutes)</label>
                    <input
                      type="number"
                      min="0"
                      value={editForm.powerPolicy?.appTimeoutMins || 0}
                      onChange={e => setEditForm({
                        ...editForm,
                        powerPolicy: { ...editForm.powerPolicy, appTimeoutMins: parseInt(e.target.value) || 0 }
                      })}
                      className="w-full px-2 py-1.5 bg-gray-900 border border-gray-700 text-sm"
                    />
                  </div>
                </div>
              </div>
            </div>
            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => { setShowEditModal(false); setEditingApp(null); }}>Annuler</Button>
              <Button onClick={handleEdit} loading={saving}>Sauvegarder</Button>
            </div>
          </div>
        </div>
      )}

      {/* Token Display Modal */}
      {tokenModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
          <div className="bg-gray-800 p-6 w-full max-w-lg border border-gray-700">
            <h2 className="text-xl font-bold mb-2 flex items-center gap-2">
              <CheckCircle className="w-5 h-5 text-green-400" />
              Application creee
            </h2>
            <p className="text-sm text-gray-400 mb-4">
              Voici le token d&apos;authentification pour <strong>{tokenModal.name}</strong>.
              Copiez-le maintenant, il ne sera plus affiche.
            </p>
            <div className="flex items-center gap-2 bg-gray-900 border border-gray-700 p-3">
              <code className="flex-1 text-sm text-green-400 font-mono break-all">{tokenModal.token}</code>
              <button
                onClick={() => copyToken(tokenModal.token)}
                className="text-gray-400 hover:text-white p-1 flex-shrink-0"
              >
                <Copy className="w-4 h-4" />
              </button>
            </div>
            <div className="flex justify-end mt-4">
              <Button onClick={() => setTokenModal(null)}>Fermer</Button>
            </div>
          </div>
        </div>
      )}

      {/* Terminal Modal */}
      {terminalApp && (
        <TerminalModal app={terminalApp} onClose={() => setTerminalApp(null)} />
      )}
    </div>
  );
}

function TerminalModal({ app, onClose }) {
  const termRef = useRef(null);
  const termInstance = useRef(null);
  const wsRef = useRef(null);
  const fitAddonRef = useRef(null);

  useEffect(() => {
    let cancelled = false;

    async function init() {
      const { Terminal: XTerm } = await import('@xterm/xterm');
      const { FitAddon } = await import('@xterm/addon-fit');
      await import('@xterm/xterm/css/xterm.css');

      if (cancelled || !termRef.current) return;

      const fitAddon = new FitAddon();
      fitAddonRef.current = fitAddon;

      const term = new XTerm({
        cursorBlink: true,
        fontSize: 14,
        fontFamily: 'Menlo, Monaco, "Courier New", monospace',
        theme: {
          background: '#111827',
          foreground: '#e5e7eb',
          cursor: '#10b981',
          selectionBackground: '#374151',
        },
      });

      term.loadAddon(fitAddon);
      term.open(termRef.current);
      fitAddon.fit();
      termInstance.current = term;

      // Connect WebSocket
      const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
      const ws = new WebSocket(`${proto}//${window.location.host}/api/applications/${app.id}/terminal`);
      ws.binaryType = 'arraybuffer';
      wsRef.current = ws;

      ws.onopen = () => {
        term.write('\r\n\x1b[32mConnexion au conteneur ' + app.container_name + '...\x1b[0m\r\n\r\n');
      };

      ws.onmessage = (event) => {
        if (event.data instanceof ArrayBuffer) {
          term.write(new Uint8Array(event.data));
        } else {
          term.write(event.data);
        }
      };

      ws.onclose = () => {
        term.write('\r\n\x1b[31mConnexion fermee.\x1b[0m\r\n');
      };

      ws.onerror = () => {
        term.write('\r\n\x1b[31mErreur de connexion.\x1b[0m\r\n');
      };

      // Send keystrokes to the server
      term.onData((data) => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(data);
        }
      });

      // Handle window resize
      const handleResize = () => {
        fitAddon.fit();
      };
      window.addEventListener('resize', handleResize);

      return () => {
        window.removeEventListener('resize', handleResize);
      };
    }

    init();

    return () => {
      cancelled = true;
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
      if (termInstance.current) {
        termInstance.current.dispose();
        termInstance.current = null;
      }
    };
  }, [app]);

  return (
    <div className="fixed inset-0 bg-black/80 flex flex-col z-50">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 bg-gray-900 border-b border-gray-700">
        <div className="flex items-center gap-2 text-sm">
          <Terminal className="w-4 h-4 text-emerald-400" />
          <span className="font-medium">{app.name}</span>
          <span className="text-gray-500 font-mono">({app.container_name})</span>
        </div>
        <button
          onClick={onClose}
          className="text-gray-400 hover:text-white p-1 transition-colors"
        >
          <X className="w-5 h-5" />
        </button>
      </div>
      {/* Terminal */}
      <div ref={termRef} className="flex-1 p-2" style={{ backgroundColor: '#111827' }} />
    </div>
  );
}

export default Applications;
