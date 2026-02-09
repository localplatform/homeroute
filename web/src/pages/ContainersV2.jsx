import { useState, useEffect, useRef, useCallback } from 'react';
import {
  Container,
  Plus,
  Trash2,
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
  HardDrive,
  RefreshCw,
  AlertTriangle,
  X,
  Terminal,
  Code2,
  Loader2,
  Play,
  Square,
  ArrowRightLeft,
} from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import PageHeader from '../components/PageHeader';
import useWebSocket from '../hooks/useWebSocket';
import {
  getContainersV2,
  createContainerV2,
  updateContainerV2,
  deleteContainerV2,
  startContainerV2,
  stopContainerV2,
  migrateContainerV2,
  cancelMigrationV2,
  getReverseProxyConfig,
  getHosts,
} from '../api/client';

const STATUS_BADGES = {
  connected: { color: 'text-green-400 bg-green-900/30', icon: Wifi, label: 'Connecte' },
  deploying: { color: 'text-blue-400 bg-blue-900/30', icon: Loader2, label: 'Deploiement', spin: true },
  pending: { color: 'text-yellow-400 bg-yellow-900/30', icon: Clock, label: 'En attente' },
  running: { color: 'text-yellow-400 bg-yellow-900/30', icon: Clock, label: 'En attente' },
  stopped: { color: 'text-gray-400 bg-gray-900/30', icon: Square, label: 'Arrete' },
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

function ContainersV2() {
  const [containers, setContainers] = useState([]);
  const [baseDomain, setBaseDomain] = useState('');
  const [hosts, setHosts] = useState([]);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState(null);
  const [saving, setSaving] = useState(false);

  // Modal states
  const [showCreateModal, setShowCreateModal] = useState(false);
  const [terminalContainer, setTerminalContainer] = useState(null);
  const [migrateModal, setMigrateModal] = useState(null);
  const [selectedHostId, setSelectedHostId] = useState('');
  const [migrating, setMigrating] = useState(false);
  const [migrations, setMigrations] = useState({});

  // Agent metrics state
  const [appMetrics, setAppMetrics] = useState({});

  // Create form
  const [createForm, setCreateForm] = useState({
    name: '',
    slug: '',
    host_id: 'local',
    frontend: { target_port: '', auth_required: false, allowed_groups: [], local_only: false },
    apis: [],
    code_server_enabled: true,
  });

  const fetchData = useCallback(async () => {
    try {
      const [containersRes, configRes, hostsRes] = await Promise.all([
        getContainersV2(),
        getReverseProxyConfig(),
        getHosts().catch(() => ({ data: { hosts: [] } })),
      ]);
      if (containersRes.data.success !== false) {
        const list = containersRes.data.containers || containersRes.data || [];
        setContainers(list);
        // Pre-populate metrics from initial REST response
        const initialMetrics = {};
        for (const c of list) {
          if (c.metrics) {
            initialMetrics[c.id] = {
              codeServerStatus: c.metrics.code_server_status,
              appStatus: c.metrics.app_status,
              dbStatus: c.metrics.db_status,
              memoryBytes: c.metrics.memory_bytes,
              cpuPercent: c.metrics.cpu_percent,
            };
          }
        }
        if (Object.keys(initialMetrics).length > 0) {
          setAppMetrics(prev => ({ ...initialMetrics, ...prev }));
        }
      }
      if (configRes.data.success) setBaseDomain(configRes.data.config?.baseDomain || '');
      const hostList = hostsRes.data?.hosts || [];
      setHosts(hostList);
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

  const fetchDataRef = useRef(fetchData);
  fetchDataRef.current = fetchData;

  useWebSocket({
    'agent:status': (data) => {
      const { appId, status, message: stepMsg } = data;
      setContainers(prev => {
        const old = prev.find(c => c.id === appId);
        if (!old) return prev;
        const wasDeploying = old.status === 'deploying' || old._deployMessage;
        const nowReady = status === 'connected' || (status === 'pending' && wasDeploying);
        if (wasDeploying && nowReady) {
          setTimeout(() => fetchDataRef.current(), 500);
        }
        return prev.map(c =>
          c.id === appId
            ? { ...c, status, _deployMessage: status === 'deploying' ? (stepMsg || null) : null }
            : c
        );
      });
    },
    'agent:metrics': (data) => {
      const { appId, codeServerStatus, appStatus, dbStatus, memoryBytes, cpuPercent } = data;
      setAppMetrics(prev => ({
        ...prev,
        [appId]: { codeServerStatus, appStatus, dbStatus, memoryBytes, cpuPercent }
      }));
    },
    'hosts:status': (data) => {
      const { hostId, status } = data;
      setHosts(prev => prev.map(h =>
        h.id === hostId ? { ...h, status } : h
      ));
    },
    'migration:progress': (data) => {
      setMigrations(prev => ({
        ...prev,
        [data.appId]: {
          phase: data.phase,
          progressPct: data.progressPct,
          bytesTransferred: data.bytesTransferred,
          totalBytes: data.totalBytes,
          error: data.error,
        }
      }));
      if (data.phase === 'complete') {
        setTimeout(() => fetchDataRef.current(), 1000);
      }
    },
  });

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
        host_id: createForm.host_id,
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

      const res = await createContainerV2(payload);
      if (res.data.success) {
        setShowCreateModal(false);
        setCreateForm({
          name: '', slug: '', host_id: 'local',
          frontend: { target_port: '', auth_required: false, allowed_groups: [], local_only: false },
          apis: [],
          code_server_enabled: true,
        });
        setMessage({ type: 'success', text: 'Conteneur cree' });
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

  async function handleDelete(id, name) {
    if (!confirm(`Supprimer "${name}" ?\nCeci detruira le conteneur nspawn, les enregistrements DNS et les certificats.`)) return;
    try {
      const res = await deleteContainerV2(id);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Conteneur supprime' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  async function handleStart(id) {
    try {
      const res = await startContainerV2(id);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Conteneur demarre' });
        fetchData();
        // Agent takes time to boot and reconnect
        setTimeout(() => fetchData(), 5000);
        setTimeout(() => fetchData(), 15000);
      } else {
        setMessage({ type: 'error', text: res.data.error || 'Erreur' });
      }
    } catch {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  async function handleStop(id) {
    try {
      const res = await stopContainerV2(id);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Conteneur arrete' });
        fetchData();
        // Agent takes time to disconnect
        setTimeout(() => fetchData(), 3000);
      } else {
        setMessage({ type: 'error', text: res.data.error || 'Erreur' });
      }
    } catch {
      setMessage({ type: 'error', text: 'Erreur' });
    }
  }

  const openMigrateModal = async (container) => {
    try {
      const res = await getHosts();
      const hostList = res.data.hosts || res.data || [];
      setHosts(hostList);
      setMigrateModal(container);
      setSelectedHostId('');
    } catch (err) {
      console.error('Failed to fetch hosts:', err);
    }
  };

  const handleMigrate = async () => {
    if (!migrateModal || !selectedHostId) return;
    const targetHost = hosts.find(h => h.id === selectedHostId);
    const targetName = selectedHostId === 'local' ? 'HomeRoute (local)' : (targetHost?.name || selectedHostId);
    if (!confirm(`Migrer ${migrateModal.name} vers ${targetName} ?\n\nLe conteneur sera arrete pendant la migration.`)) return;
    setMigrating(true);
    try {
      await migrateContainerV2(migrateModal.id, selectedHostId);
      setMigrateModal(null);
    } catch (err) {
      console.error('Migration failed:', err);
      alert(err.response?.data?.error || 'Migration failed');
    } finally {
      setMigrating(false);
    }
  };

  function formatBytes(bytes) {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  }

  // MigrationProgress inline component
  function MigrationProgress({ appId, migration, onDismiss }) {
    if (!migration) return null;

    const prevRef = useRef({ bytes: 0, time: Date.now() });
    const speedRef = useRef(0);

    useEffect(() => {
      const now = Date.now();
      const elapsed = (now - prevRef.current.time) / 1000;
      const deltaBytes = migration.bytesTransferred - prevRef.current.bytes;
      if (elapsed > 0.5 && deltaBytes > 0) {
        const instantSpeed = deltaBytes / elapsed;
        speedRef.current = speedRef.current > 0
          ? speedRef.current * 0.6 + instantSpeed * 0.4
          : instantSpeed;
        prevRef.current = { bytes: migration.bytesTransferred, time: now };
      }
    }, [migration.bytesTransferred]);

    const phaseLabels = {
      stopping: 'Arret...',
      exporting: 'Export...',
      transferring: 'Transfert conteneur...',
      transferring_workspace: 'Transfert workspace...',
      importing: 'Import...',
      importing_workspace: 'Import workspace...',
      starting: 'Demarrage...',
      verifying: 'Verification...',
      complete: 'Termine',
      failed: 'Echoue',
    };

    const isActive = migration.phase !== 'complete' && migration.phase !== 'failed';
    const isTransfer = migration.phase === 'transferring' || migration.phase === 'transferring_workspace';
    const speed = speedRef.current;
    const remaining = migration.totalBytes - migration.bytesTransferred;
    const eta = speed > 0 && remaining > 0 ? Math.ceil(remaining / speed) : 0;

    const formatEta = (secs) => {
      if (secs <= 0) return '';
      if (secs < 60) return `${secs}s`;
      const m = Math.floor(secs / 60);
      const s = secs % 60;
      return s > 0 ? `${m}m${s}s` : `${m}m`;
    };

    const handleCancel = async () => {
      try {
        await cancelMigrationV2(appId);
      } catch (err) {
        console.error('Cancel migration failed:', err);
      }
    };

    return (
      <div className="p-2 bg-gray-700/50">
        <div className="flex items-center justify-between mb-1">
          <span className={`text-xs ${migration.phase === 'failed' ? 'text-red-400' : migration.phase === 'complete' ? 'text-green-400' : 'text-gray-300'}`}>
            {phaseLabels[migration.phase] || migration.phase}
          </span>
          <div className="flex items-center gap-2">
            <span className="text-xs text-gray-400">{migration.progressPct}%</span>
            {isActive && (
              <button
                onClick={handleCancel}
                className="text-red-400 hover:text-red-300 transition-colors"
                title="Annuler la migration"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            )}
            {!isActive && (
              <button
                onClick={onDismiss}
                className="text-gray-500 hover:text-gray-300 transition-colors"
                title="Fermer"
              >
                <X className="w-3.5 h-3.5" />
              </button>
            )}
          </div>
        </div>
        <div className="w-full bg-gray-600 h-1.5">
          <div
            className={`h-1.5 transition-all duration-500 ${
              migration.phase === 'failed' ? 'bg-red-500' :
              migration.phase === 'complete' ? 'bg-green-500' : 'bg-blue-500'
            }`}
            style={{ width: `${migration.progressPct}%` }}
          />
        </div>
        {migration.totalBytes > 0 && (
          <div className="flex items-center justify-between text-xs text-gray-500 mt-1">
            <span>{formatBytes(migration.bytesTransferred)} / {formatBytes(migration.totalBytes)}</span>
            {isTransfer && speed > 0 && (
              <span>
                {formatBytes(speed)}/s
                {eta > 0 && ` - ${formatEta(eta)}`}
              </span>
            )}
          </div>
        )}
        {migration.error && (
          <div className="text-xs text-red-400 mt-1 select-all cursor-text">{migration.error}</div>
        )}
      </div>
    );
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  const runningCount = containers.filter(c => (c.agent_status || c.status) === 'connected').length;
  const stoppedCount = containers.filter(c => {
    const s = c.agent_status || c.status;
    return s === 'disconnected' || s === 'pending' || s === 'stopped';
  }).length;
  const deployingCount = containers.filter(c => (c.agent_status || c.status) === 'deploying' || c.status === 'deploying').length;

  return (
    <div>
      <PageHeader title="Containers V2" icon={Container}>
        <Button onClick={fetchData} variant="secondary">
          <RefreshCw className="w-4 h-4" />
          Rafraichir
        </Button>
        <Button onClick={() => setShowCreateModal(true)}>
          <Plus className="w-4 h-4" />
          Nouveau conteneur
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
      <div className="grid grid-cols-1 md:grid-cols-4 gap-px">
        <Card title="Total" icon={Container}>
          <div className="text-2xl font-bold">{containers.length}</div>
        </Card>
        <Card title="Running" icon={Wifi}>
          <div className="text-2xl font-bold text-green-400">{runningCount}</div>
        </Card>
        <Card title="Stopped" icon={WifiOff}>
          <div className="text-2xl font-bold text-gray-400">{stoppedCount}</div>
        </Card>
        <Card title="Deploying" icon={Loader2}>
          <div className="text-2xl font-bold text-blue-400">{deployingCount}</div>
        </Card>
      </div>

      {/* Containers Table */}
      <Card>
        {containers.length === 0 ? (
          <div className="text-center py-8 text-gray-500">
            <Container className="w-12 h-12 mx-auto mb-2 opacity-50" />
            <p>Aucun conteneur V2</p>
            <p className="text-xs mt-2">Creez un conteneur nspawn pour deployer une application</p>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full">
              <thead>
                <tr className="text-xs text-gray-500 border-b border-gray-700">
                  <th className="text-left py-2 px-3 font-medium">Nom</th>
                  <th className="text-left py-2 px-3 font-medium">Slug</th>
                  <th className="text-left py-2 px-3 font-medium">Conteneur</th>
                  <th className="text-left py-2 px-3 font-medium">Hote</th>
                  <th className="text-left py-2 px-3 font-medium">Status</th>
                  <th className="text-left py-2 px-3 font-medium">IPv4</th>
                  <th className="text-right py-2 px-3 font-medium">CPU</th>
                  <th className="text-right py-2 px-3 font-medium">RAM</th>
                  <th className="text-right py-2 px-3 font-medium">Actions</th>
                </tr>
              </thead>
              <tbody>
                {containers.map(container => {
                  const displayStatus = container.agent_status || container.status;
                  const isDeploying = displayStatus === 'deploying' || container.status === 'deploying';
                  const metrics = appMetrics[container.id];
                  const isMigrating = !!migrations[container.id];
                  const hostName = container.host_id === 'local' || !container.host_id
                    ? 'HomeRoute'
                    : (hosts.find(h => h.id === container.host_id)?.name || container.host_id);

                  return (
                    <tr key={container.id} className="border-b border-gray-800 hover:bg-gray-800/30">
                      {/* Name */}
                      <td className="py-3 px-3">
                        <div className="flex items-center gap-2">
                          {isDeploying ? (
                            <Loader2 className="w-4 h-4 text-blue-400 animate-spin flex-shrink-0" />
                          ) : (
                            <Container className="w-4 h-4 text-blue-400 flex-shrink-0" />
                          )}
                          <span className="font-medium truncate">{container.name}</span>
                        </div>
                      </td>

                      {/* Slug */}
                      <td className="py-3 px-3">
                        <span className="text-xs font-mono text-gray-400">{container.slug}</span>
                      </td>

                      {/* Container Name */}
                      <td className="py-3 px-3">
                        <span className="text-xs font-mono text-gray-500">{container.container_name}</span>
                      </td>

                      {/* Host */}
                      <td className="py-3 px-3">
                        <div className="flex items-center gap-1 text-xs text-gray-400">
                          <HardDrive className="w-3 h-3" />
                          {hostName}
                        </div>
                      </td>

                      {/* Status */}
                      <td className="py-3 px-3">
                        {isMigrating ? (
                          <MigrationProgress
                            appId={container.id}
                            migration={migrations[container.id]}
                            onDismiss={() => setMigrations(prev => {
                              const next = { ...prev };
                              delete next[container.id];
                              return next;
                            })}
                          />
                        ) : isDeploying ? (
                          <div>
                            <span className="text-xs px-2 py-0.5 text-blue-400 bg-blue-900/30">Deploiement</span>
                            <p className="text-xs text-gray-500 mt-1 truncate max-w-32">{container._deployMessage}</p>
                          </div>
                        ) : (
                          <StatusBadge status={displayStatus} />
                        )}
                      </td>

                      {/* IPv4 */}
                      <td className="py-3 px-3">
                        <span className="text-xs font-mono text-gray-400">
                          {container.ipv4_address || container.ipv4 || '-'}
                        </span>
                      </td>

                      {/* CPU */}
                      <td className="py-3 px-3 text-right">
                        <span className={`font-mono text-sm ${
                          !isMigrating && displayStatus === 'connected' && metrics?.cpuPercent > 80 ? 'text-red-400' :
                          !isMigrating && displayStatus === 'connected' && metrics?.cpuPercent > 50 ? 'text-yellow-400' :
                          !isMigrating && displayStatus === 'connected' && metrics?.cpuPercent > 0 ? 'text-green-400' : 'text-gray-600'
                        }`}>
                          {!isMigrating && displayStatus === 'connected' && metrics?.cpuPercent !== undefined ? `${metrics.cpuPercent.toFixed(1)}%` : '-'}
                        </span>
                      </td>

                      {/* RAM */}
                      <td className="py-3 px-3 text-right">
                        <span className="font-mono text-sm text-gray-400">
                          {!isMigrating && displayStatus === 'connected' && metrics?.memoryBytes ? formatBytes(metrics.memoryBytes) : '-'}
                        </span>
                      </td>

                      {/* Actions */}
                      <td className="py-3 px-3">
                        <div className={`flex items-center justify-end gap-1 ${isMigrating ? 'opacity-50 pointer-events-none' : ''}`}>
                          {displayStatus === 'connected' ? (
                            <button
                              onClick={() => handleStop(container.id)}
                              className="p-1.5 text-yellow-400 hover:text-yellow-300 hover:bg-yellow-900/30 transition-colors"
                              title="Arreter"
                            >
                              <Square className="w-4 h-4" />
                            </button>
                          ) : displayStatus !== 'deploying' ? (
                            <button
                              onClick={() => handleStart(container.id)}
                              className="p-1.5 text-green-400 hover:text-green-300 hover:bg-green-900/30 transition-colors"
                              title="Demarrer"
                            >
                              <Play className="w-4 h-4" />
                            </button>
                          ) : null}
                          <button
                            onClick={() => setTerminalContainer(container)}
                            disabled={isMigrating}
                            className="p-1.5 text-emerald-400 hover:text-emerald-300 hover:bg-emerald-900/30 transition-colors"
                            title="Terminal"
                          >
                            <Terminal className="w-4 h-4" />
                          </button>
                          {container.code_server_enabled && baseDomain && (
                            <a
                              href={`https://code.${container.slug}.${baseDomain}`}
                              target="_blank"
                              rel="noopener noreferrer"
                              className="p-1.5 text-purple-400 hover:text-purple-300 hover:bg-purple-900/30 transition-colors"
                              title="code-server"
                            >
                              <Code2 className="w-4 h-4" />
                            </a>
                          )}
                          <button
                            onClick={() => openMigrateModal(container)}
                            disabled={isMigrating}
                            className="p-1.5 text-gray-400 hover:text-blue-400 hover:bg-gray-700 rounded transition-colors"
                            title="Migrer vers un autre hote"
                          >
                            <ArrowRightLeft className="w-4 h-4" />
                          </button>
                          <button
                            onClick={() => handleDelete(container.id, container.name)}
                            disabled={isMigrating}
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
            <h2 className="text-xl font-bold mb-4">Nouveau conteneur V2</h2>
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
                    <p className="text-xs text-gray-500 mt-1 font-mono">app.{createForm.slug}.{baseDomain}</p>
                  )}
                </div>
              </div>

              {/* Host selector */}
              <div>
                <label className="block text-sm text-gray-400 mb-1">Hote</label>
                <select
                  value={createForm.host_id}
                  onChange={e => setCreateForm({ ...createForm, host_id: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 text-sm"
                >
                  <option value="local">HomeRoute (local)</option>
                  {hosts.filter(h => h.status === 'online').map(h => (
                    <option key={h.id} value={h.id}>{h.name} ({h.host})</option>
                  ))}
                </select>
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
                  <span className="text-xs text-gray-500 font-mono ml-2">code.{createForm.slug}.{baseDomain}</span>
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
                              <p className="text-xs text-gray-500 mt-0.5 font-mono">{api.slug}.{createForm.slug}.{baseDomain}</p>
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
            </div>
            <div className="flex justify-end gap-2 mt-6">
              <Button variant="secondary" onClick={() => setShowCreateModal(false)}>Annuler</Button>
              <Button onClick={handleCreate} loading={saving}>Creer</Button>
            </div>
          </div>
        </div>
      )}

      {/* Migrate Modal */}
      {migrateModal && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-gray-800 p-6 w-full max-w-md border border-gray-700">
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-semibold">Migrer {migrateModal.name}</h3>
              <button onClick={() => setMigrateModal(null)} className="p-1 text-gray-400 hover:text-white">
                <X className="w-5 h-5" />
              </button>
            </div>
            <p className="text-sm text-gray-400 mb-4">
              Selectionnez l&apos;hote de destination pour migrer ce conteneur.
            </p>
            <select
              value={selectedHostId}
              onChange={(e) => setSelectedHostId(e.target.value)}
              className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white mb-4"
            >
              <option value="">Choisir un hote...</option>
              {hosts
                .filter(h => h.id !== migrateModal.host_id && h.name !== 'HomeRoute')
                .map(h => (
                  <option key={h.id} value={h.id}>
                    {h.name} ({h.host}) — {h.status}
                  </option>
                ))
              }
              {migrateModal.host_id !== 'local' && (
                <option value="local">HomeRoute (local)</option>
              )}
            </select>
            <div className="flex justify-end gap-2">
              <button
                onClick={() => setMigrateModal(null)}
                className="px-4 py-2 text-gray-300 hover:text-white transition-colors"
              >
                Annuler
              </button>
              <button
                onClick={handleMigrate}
                disabled={!selectedHostId || migrating}
                className="px-4 py-2 bg-blue-600 hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed text-white transition-colors flex items-center gap-2"
              >
                {migrating && <Loader2 className="w-4 h-4 animate-spin" />}
                Migrer
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Terminal Modal */}
      {terminalContainer && (
        <TerminalModal container={terminalContainer} onClose={() => setTerminalContainer(null)} />
      )}
    </div>
  );
}

function TerminalModal({ container, onClose }) {
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
      const ws = new WebSocket(`${proto}//${window.location.host}/api/containers/${container.id}/terminal`);
      ws.binaryType = 'arraybuffer';
      wsRef.current = ws;

      ws.onopen = () => {
        term.write('\r\n\x1b[32mConnexion au conteneur ' + container.container_name + '...\x1b[0m\r\n\r\n');
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

      term.onData((data) => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(data);
        }
      });

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
  }, [container]);

  return (
    <div className="fixed inset-0 bg-black/80 flex flex-col z-50">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-2 bg-gray-900 border-b border-gray-700">
        <div className="flex items-center gap-2 text-sm">
          <Terminal className="w-4 h-4 text-emerald-400" />
          <span className="font-medium">{container.name}</span>
          <span className="text-gray-500 font-mono">({container.container_name})</span>
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

export default ContainersV2;
