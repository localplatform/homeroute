import { useState, useEffect } from 'react';
import {
  HardDrive, Plus, Trash2, RefreshCw, X, Check,
  Play, Square, RotateCw, Moon, CheckCircle, XCircle, Settings
} from 'lucide-react';
import Button from '../components/Button';
import StatusBadge from '../components/StatusBadge';
import PageHeader from '../components/PageHeader';
import Section from '../components/Section';
import {
  getHosts,
  addHost,
  updateHost,
  deleteHost,
  wakeHost,
  shutdownHost,
  rebootHost,
  sleepHost,
  setWolMac,
  setAutoOff,
  updateLocalHostConfig,
  getLocalInterfaces
} from '../api/client';
import useWebSocket from '../hooks/useWebSocket';

export default function Hosts() {
  const [hosts, setHosts] = useState([]);
  const [loading, setLoading] = useState(true);
  const [showAddModal, setShowAddModal] = useState(false);
  const [settingsHost, setSettingsHost] = useState(null);
  // Add host form
  const [formData, setFormData] = useState({
    name: '',
    host: '',
    port: 22,
    username: 'root',
    password: '',
    container_storage_path: '/var/lib/machines',
  });
  const [addingHost, setAddingHost] = useState(false);
  const [addError, setAddError] = useState('');
  const [message, setMessage] = useState(null);

  // Settings modal state
  const [settingsForm, setSettingsForm] = useState({
    wol_mac: '',
    auto_off_mode: 'off',
    auto_off_minutes: 5,
    lan_interface: '',
    container_storage_path: '/var/lib/machines',
  });
  const [localInterfaces, setLocalInterfaces] = useState([]);
  const [savingSettings, setSavingSettings] = useState(false);

  useWebSocket({
    'hosts:status': (data) => {
      setHosts(prev =>
        prev.map(h =>
          h.id === data.hostId
            ? { ...h, status: data.status, latency: data.latency, lastSeen: data.lastSeen }
            : h
        )
      );
    },
    'hosts:metrics': (data) => {
      setHosts(prev =>
        prev.map(h =>
          h.id === data.hostId
            ? { ...h, metrics: data }
            : h
        )
      );
    },
    'hosts:power': (data) => {
      setHosts(prev =>
        prev.map(h =>
          h.id === data.hostId
            ? { ...h, power_state: data.state }
            : h
        )
      );
    },
  });

  useEffect(() => {
    if (message) {
      const timer = setTimeout(() => setMessage(null), 4000);
      return () => clearTimeout(timer);
    }
  }, [message]);

  useEffect(() => {
    loadHosts();
  }, []);

  const loadHosts = async () => {
    try {
      setLoading(true);
      const res = await getHosts();
      setHosts(res.data.hosts || []);
    } catch (error) {
      console.error('Failed to load hosts:', error);
    } finally {
      setLoading(false);
    }
  };

  // ── Host CRUD ───────────────────────────────

  const handleAddHost = async (e) => {
    e.preventDefault();
    setAddingHost(true);
    setAddError('');

    try {
      const res = await addHost({
        ...formData,
        port: parseInt(formData.port)
      });

      if (res.data.success) {
        setHosts([...hosts, res.data.host]);
        setShowAddModal(false);
        resetForm();
      } else {
        setAddError(res.data.error || 'Failed to add host');
      }
    } catch (error) {
      setAddError(error.response?.data?.error || error.message || 'Failed to add host');
    } finally {
      setAddingHost(false);
    }
  };

  const handleDeleteHost = async (id) => {
    if (!confirm('Supprimer cet hote ?')) return;

    try {
      await deleteHost(id);
      setHosts(hosts.filter(h => h.id !== id));
      setMessage({ type: 'success', text: 'Hote supprime' });
    } catch (error) {
      setMessage({ type: 'error', text: 'Echec suppression : ' + error.message });
    }
  };

  // ── Power actions ───────────────────────────

  const handleWake = async (id) => {
    try {
      const res = await wakeHost(id);
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Paquet WOL envoye !' });
      } else {
        setMessage({ type: 'error', text: 'Echec : ' + res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Echec WOL : ' + error.message });
    }
  };

  const handleShutdown = async (id) => {
    if (!confirm('Eteindre cet hote ?')) return;
    try {
      const res = await shutdownHost(id);
      if (res.data.success) setMessage({ type: 'success', text: 'Commande d\'extinction envoyee !' });
    } catch (error) {
      setMessage({ type: 'error', text: 'Echec : ' + error.message });
    }
  };

  const handleReboot = async (id) => {
    if (!confirm('Redemarrer cet hote ?')) return;
    try {
      const res = await rebootHost(id);
      if (res.data.success) setMessage({ type: 'success', text: 'Commande de redemarrage envoyee !' });
    } catch (error) {
      setMessage({ type: 'error', text: 'Echec : ' + error.message });
    }
  };

  const handleSleep = async (id) => {
    if (!confirm('Mettre en veille cet hote ?')) return;
    try {
      const res = await sleepHost(id);
      if (res.data.success) setMessage({ type: 'success', text: 'Commande de mise en veille envoyee !' });
    } catch (error) {
      setMessage({ type: 'error', text: 'Echec : ' + error.message });
    }
  };

  // ── Settings modal ─────────────────────────

  const filterPhysicalInterfaces = (interfaces) => {
    if (!interfaces) return [];
    return interfaces.filter(i =>
      i.ifname &&
      i.ifname !== 'lo' &&
      !i.ifname.startsWith('br-') &&
      !i.ifname.startsWith('veth') &&
      !i.ifname.startsWith('docker') &&
      !i.ifname.startsWith('virbr')
    );
  };

  const openSettings = async (host) => {
    setSettingsForm({
      wol_mac: host.wol_mac || host.mac || '',
      auto_off_mode: host.auto_off_mode || 'off',
      auto_off_minutes: host.auto_off_minutes || 5,
      lan_interface: host.lan_interface || '',
      container_storage_path: host.container_storage_path || '/var/lib/machines',
    });

    if (host.is_local) {
      try {
        const res = await getLocalInterfaces();
        setLocalInterfaces(res.data.interfaces || []);
      } catch (error) {
        console.error('Failed to load local interfaces:', error);
        setLocalInterfaces([]);
      }
    } else {
      setLocalInterfaces([]);
    }

    setSettingsHost(host);
  };

  const handleSaveSettings = async () => {
    if (!settingsHost) return;
    setSavingSettings(true);

    try {
      if (settingsHost.is_local) {
        await updateLocalHostConfig({
          lan_interface: settingsForm.lan_interface || null,
          container_storage_path: settingsForm.container_storage_path,
        });
        setHosts(prev => prev.map(h =>
          h.id === settingsHost.id
            ? { ...h, lan_interface: settingsForm.lan_interface, container_storage_path: settingsForm.container_storage_path }
            : h
        ));
      } else {
        // Save WOL MAC
        if (settingsForm.wol_mac !== (settingsHost.wol_mac || settingsHost.mac || '')) {
          await setWolMac(settingsHost.id, settingsForm.wol_mac);
        }
        // Save auto-off
        if (settingsForm.auto_off_mode !== (settingsHost.auto_off_mode || 'off') ||
            settingsForm.auto_off_minutes !== (settingsHost.auto_off_minutes || 5)) {
          const mins = settingsForm.auto_off_mode === 'off' ? 0 : settingsForm.auto_off_minutes;
          await setAutoOff(settingsHost.id, settingsForm.auto_off_mode, mins);
        }
        // Save macvlan + storage path
        await updateHost(settingsHost.id, {
          lan_interface: settingsForm.lan_interface || null,
          container_storage_path: settingsForm.container_storage_path,
        });
        setHosts(prev => prev.map(h =>
          h.id === settingsHost.id
            ? {
                ...h,
                wol_mac: settingsForm.wol_mac,
                auto_off_mode: settingsForm.auto_off_mode,
                auto_off_minutes: settingsForm.auto_off_minutes,
                lan_interface: settingsForm.lan_interface,
                container_storage_path: settingsForm.container_storage_path,
              }
            : h
        ));
      }
      setMessage({ type: 'success', text: 'Configuration sauvegardee' });
      setSettingsHost(null);
    } catch (error) {
      setMessage({ type: 'error', text: 'Echec sauvegarde : ' + (error.response?.data?.error || error.message) });
    } finally {
      setSavingSettings(false);
    }
  };

  // ── Form helpers ────────────────────────────

  const resetForm = () => {
    setFormData({ name: '', host: '', port: 22, username: 'root', password: '', container_storage_path: '/var/lib/machines' });
    setAddError('');
  };

  const getEffectiveStatus = (host) => {
    if (host.is_local) return 'online';
    if (host.power_state && host.power_state !== 'online' && host.power_state !== 'offline') {
      return host.power_state;
    }
    return host.status || 'offline';
  };

  const getStatusColor = (status) => {
    switch (status) {
      case 'online': return 'up';
      case 'offline': return 'down';
      case 'suspended': return 'unknown';
      case 'suspending':
      case 'shutting_down':
      case 'rebooting':
      case 'waking_up': return 'active';
      default: return 'unknown';
    }
  };

  const getStatusLabel = (status) => {
    switch (status) {
      case 'online': return 'En ligne';
      case 'offline': return 'Hors ligne';
      case 'suspended': return 'En veille';
      case 'suspending': return 'Mise en veille...';
      case 'shutting_down': return 'Extinction...';
      case 'rebooting': return 'Redemarrage...';
      case 'waking_up': return 'Reveil...';
      default: return status || 'Inconnu';
    }
  };

  const formatBytes = (bytes) => {
    if (!bytes) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  };

  return (
    <div>
      <PageHeader title="Hotes" icon={HardDrive}>
        <Button onClick={() => setShowAddModal(true)}>
          <Plus className="w-4 h-4 mr-2" />
          Ajouter
        </Button>
      </PageHeader>

      {message && (
        <div className={`p-4 flex items-center gap-2 ${
          message.type === 'success' ? 'bg-green-900/50 text-green-400' : 'bg-red-900/50 text-red-400'
        }`}>
          {message.type === 'success' ? <CheckCircle className="w-5 h-5" /> : <XCircle className="w-5 h-5" />}
          {message.text}
        </div>
      )}

      <Section title="Liste des hotes">
        {loading ? (
          <div className="text-center py-12 text-gray-400">Chargement...</div>
        ) : hosts.length === 0 ? (
          <div className="bg-gray-800 border border-gray-700 p-8 text-center text-gray-400">
            Aucun hote configure. Cliquez sur "Ajouter" pour commencer.
          </div>
        ) : (
          <div className="border border-gray-700 overflow-x-auto">
            <table className="w-full text-left">
              <thead className="bg-gray-800/60 border-b border-gray-700">
                <tr>
                  <th className="px-3 py-2 text-xs font-medium text-gray-400 uppercase">Nom</th>
                  <th className="px-3 py-2 text-xs font-medium text-gray-400 uppercase">Statut</th>
                  <th className="px-3 py-2 text-xs font-medium text-gray-400 uppercase">Adresse</th>
                  <th className="px-3 py-2 text-xs font-medium text-gray-400 uppercase">CPU</th>
                  <th className="px-3 py-2 text-xs font-medium text-gray-400 uppercase">RAM</th>
                  <th className="px-3 py-2 text-xs font-medium text-gray-400 uppercase">Macvlan</th>
                  <th className="px-3 py-2 text-xs font-medium text-gray-400 uppercase">Vu</th>
                  <th className="px-3 py-2 text-xs font-medium text-gray-400 uppercase">Actions</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-700/50">
                {hosts.map((host) => (
                  <tr key={host.id} className="bg-gray-800 hover:bg-gray-700/50">
                    {/* Name */}
                    <td className="px-3 py-2 text-sm font-medium text-white">
                      <div className="flex items-center gap-2">
                        <HardDrive className={`w-4 h-4 flex-shrink-0 ${host.is_local ? 'text-green-400' : 'text-blue-400'}`} />
                        {host.is_local ? 'HomeRoute' : host.name}
                      </div>
                    </td>
                    {/* Status */}
                    <td className="px-3 py-2 text-sm">
                      {(() => {
                        const st = getEffectiveStatus(host);
                        return (
                          <StatusBadge status={getStatusColor(st)}>
                            {getStatusLabel(st)}
                          </StatusBadge>
                        );
                      })()}
                    </td>
                    {/* Address */}
                    <td className="px-3 py-2 text-sm font-mono text-gray-300">
                      {host.is_local ? '127.0.0.1' : host.host}
                    </td>
                    {/* CPU */}
                    <td className="px-3 py-2 text-sm">
                      {host.metrics ? (
                        <div className="flex items-center gap-1">
                          <div className="w-12 bg-gray-700 h-1.5 rounded-sm overflow-hidden">
                            <div className="h-1.5 bg-blue-500" style={{ width: `${Math.min(host.metrics.cpuPercent, 100)}%` }} />
                          </div>
                          <span className="text-gray-400 text-xs">{host.metrics.cpuPercent.toFixed(0)}%</span>
                        </div>
                      ) : <span className="text-gray-600 text-xs">--</span>}
                    </td>
                    {/* RAM */}
                    <td className="px-3 py-2 text-sm">
                      {host.metrics ? (
                        <div className="flex items-center gap-1">
                          <div className="w-12 bg-gray-700 h-1.5 rounded-sm overflow-hidden">
                            <div className="h-1.5 bg-green-500" style={{ width: `${(host.metrics.memoryUsedBytes / host.metrics.memoryTotalBytes * 100).toFixed(0)}%` }} />
                          </div>
                          <span className="text-gray-400 text-xs">{formatBytes(host.metrics.memoryUsedBytes)}</span>
                        </div>
                      ) : <span className="text-gray-600 text-xs">--</span>}
                    </td>
                    {/* Macvlan */}
                    <td className="px-3 py-2 text-sm text-gray-300">
                      {host.lan_interface || '--'}
                    </td>
                    {/* Last seen */}
                    <td className="px-3 py-2 text-xs text-gray-500">
                      {host.is_local ? '--' : (host.lastSeen ? new Date(host.lastSeen).toLocaleTimeString() : '--')}
                    </td>
                    {/* Actions */}
                    <td className="px-3 py-2">
                      <div className="flex gap-1">
                        <button
                          onClick={() => openSettings(host)}
                          className="p-1.5 text-gray-400 hover:bg-gray-600/20 hover:text-white"
                          title="Parametres"
                        >
                          <Settings className="w-3.5 h-3.5" />
                        </button>
                        {!host.is_local && (() => {
                          const st = getEffectiveStatus(host);
                          const isOnline = st === 'online';
                          const canWake = st === 'offline' || st === 'suspended';
                          return (
                            <>
                              <button
                                onClick={() => handleWake(host.id)}
                                disabled={!canWake}
                                className="p-1.5 text-green-400 hover:bg-green-600/20 disabled:opacity-30 disabled:cursor-not-allowed"
                                title="Wake"
                              >
                                <Play className="w-3.5 h-3.5" />
                              </button>
                              <button
                                onClick={() => handleReboot(host.id)}
                                disabled={!isOnline}
                                className="p-1.5 text-yellow-400 hover:bg-yellow-600/20 disabled:opacity-30 disabled:cursor-not-allowed"
                                title="Reboot"
                              >
                                <RotateCw className="w-3.5 h-3.5" />
                              </button>
                              <button
                                onClick={() => handleSleep(host.id)}
                                disabled={!isOnline}
                                className="p-1.5 text-blue-400 hover:bg-blue-600/20 disabled:opacity-30 disabled:cursor-not-allowed"
                                title="Sleep"
                              >
                                <Moon className="w-3.5 h-3.5" />
                              </button>
                              <button
                                onClick={() => handleShutdown(host.id)}
                                disabled={!isOnline}
                                className="p-1.5 text-red-400 hover:bg-red-600/20 disabled:opacity-30 disabled:cursor-not-allowed"
                                title="Shutdown"
                              >
                                <Square className="w-3.5 h-3.5" />
                              </button>
                              <button
                                onClick={() => handleDeleteHost(host.id)}
                                className="p-1.5 text-red-400 hover:bg-red-600/20"
                                title="Supprimer"
                              >
                                <Trash2 className="w-3.5 h-3.5" />
                              </button>
                            </>
                          );
                        })()}
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>

      {/* Add Host Modal */}
      {showAddModal && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-gray-800 p-6 w-full max-w-md">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-xl font-bold text-white">Ajouter un hote</h2>
              <button onClick={() => { setShowAddModal(false); resetForm(); }} className="text-gray-400 hover:text-white">
                <X className="w-5 h-5" />
              </button>
            </div>

            <form onSubmit={handleAddHost} className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">Nom *</label>
                <input
                  type="text"
                  value={formData.name}
                  onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white focus:ring-2 focus:ring-blue-500"
                  placeholder="Mon serveur"
                  required
                />
              </div>

              <div className="grid grid-cols-3 gap-2">
                <div className="col-span-2">
                  <label className="block text-sm font-medium text-gray-300 mb-1">Adresse IP *</label>
                  <input
                    type="text"
                    value={formData.host}
                    onChange={(e) => setFormData({ ...formData, host: e.target.value })}
                    className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white focus:ring-2 focus:ring-blue-500"
                    placeholder="10.0.0.10"
                    required
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-1">Port</label>
                  <input
                    type="number"
                    value={formData.port}
                    onChange={(e) => setFormData({ ...formData, port: e.target.value })}
                    className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white focus:ring-2 focus:ring-blue-500"
                    placeholder="22"
                  />
                </div>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">Utilisateur SSH *</label>
                <input
                  type="text"
                  value={formData.username}
                  onChange={(e) => setFormData({ ...formData, username: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white focus:ring-2 focus:ring-blue-500"
                  placeholder="root"
                  required
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">Mot de passe SSH *</label>
                <input
                  type="password"
                  value={formData.password}
                  onChange={(e) => setFormData({ ...formData, password: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white focus:ring-2 focus:ring-blue-500"
                  placeholder="••••••••"
                  required
                />
                <p className="text-xs text-gray-400 mt-1">Utilise une seule fois pour configurer l'authentification par cle SSH</p>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">Chemin de stockage conteneurs</label>
                <input
                  type="text"
                  value={formData.container_storage_path}
                  onChange={(e) => setFormData({ ...formData, container_storage_path: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white focus:ring-2 focus:ring-blue-500"
                  placeholder="/var/lib/machines"
                />
                <p className="text-xs text-gray-400 mt-1">Repertoire de stockage des conteneurs nspawn sur cet hote</p>
              </div>

              {addError && (
                <div className="p-3 bg-red-900/20 border border-red-600 text-red-400 text-sm">{addError}</div>
              )}

              <div className="flex gap-2 pt-2">
                <Button type="button" variant="secondary" onClick={() => { setShowAddModal(false); resetForm(); }} className="flex-1">
                  Annuler
                </Button>
                <Button type="submit" disabled={addingHost} className="flex-1">
                  {addingHost ? (
                    <><RefreshCw className="w-4 h-4 mr-2 animate-spin" />Ajout...</>
                  ) : (
                    <><Check className="w-4 h-4 mr-2" />Ajouter</>
                  )}
                </Button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* Host Settings Modal */}
      {settingsHost && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-gray-800 p-6 w-full max-w-md">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-xl font-bold text-white">
                {settingsHost.is_local ? 'HomeRoute' : settingsHost.name} — Parametres
              </h2>
              <button onClick={() => setSettingsHost(null)} className="text-gray-400 hover:text-white">
                <X className="w-5 h-5" />
              </button>
            </div>

            <div className="space-y-4">
              {/* Remote-only fields */}
              {!settingsHost.is_local && (
                <>
                  {/* MAC WOL */}
                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-1">MAC WOL</label>
                    {settingsHost.interfaces && settingsHost.interfaces.length > 0 ? (
                      <select
                        className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white"
                        value={settingsForm.wol_mac}
                        onChange={(e) => setSettingsForm({ ...settingsForm, wol_mac: e.target.value })}
                      >
                        <option value="">-- Aucun --</option>
                        {settingsHost.interfaces.filter(i => i.address && i.address !== '00:00:00:00:00:00').map((iface, idx) => (
                          <option key={idx} value={iface.address}>
                            {iface.ifname} ({iface.address})
                          </option>
                        ))}
                      </select>
                    ) : (
                      <input
                        type="text"
                        className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white"
                        value={settingsForm.wol_mac}
                        onChange={(e) => setSettingsForm({ ...settingsForm, wol_mac: e.target.value })}
                        placeholder="AA:BB:CC:DD:EE:FF"
                      />
                    )}
                  </div>

                  {/* Auto-off */}
                  <div>
                    <label className="block text-sm font-medium text-gray-300 mb-1">Auto-arret</label>
                    <div className="flex gap-2">
                      <select
                        className="flex-1 px-3 py-2 bg-gray-700 border border-gray-600 text-white"
                        value={settingsForm.auto_off_mode}
                        onChange={(e) => setSettingsForm({ ...settingsForm, auto_off_mode: e.target.value })}
                      >
                        <option value="off">Off</option>
                        <option value="sleep">Veille</option>
                        <option value="shutdown">Extinction</option>
                      </select>
                      {settingsForm.auto_off_mode !== 'off' && (
                        <select
                          className="px-3 py-2 bg-gray-700 border border-gray-600 text-white"
                          value={settingsForm.auto_off_minutes}
                          onChange={(e) => setSettingsForm({ ...settingsForm, auto_off_minutes: parseInt(e.target.value) })}
                        >
                          <option value={2}>2m</option>
                          <option value={5}>5m</option>
                          <option value={10}>10m</option>
                          <option value={15}>15m</option>
                          <option value={30}>30m</option>
                        </select>
                      )}
                    </div>
                  </div>
                </>
              )}

              {/* Macvlan interface — both local and remote */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">Interface Macvlan</label>
                {settingsHost.is_local ? (
                  <select
                    className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white"
                    value={settingsForm.lan_interface}
                    onChange={(e) => setSettingsForm({ ...settingsForm, lan_interface: e.target.value })}
                  >
                    <option value="">-- Aucune --</option>
                    {filterPhysicalInterfaces(localInterfaces).map((iface, idx) => (
                      <option key={idx} value={iface.ifname}>
                        {iface.ifname}{iface.ipv4 ? ` (${iface.ipv4})` : ''}
                      </option>
                    ))}
                  </select>
                ) : (
                  <select
                    className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white"
                    value={settingsForm.lan_interface}
                    onChange={(e) => setSettingsForm({ ...settingsForm, lan_interface: e.target.value })}
                  >
                    <option value="">-- Aucune --</option>
                    {filterPhysicalInterfaces(settingsHost.interfaces).map((iface, idx) => (
                      <option key={idx} value={iface.ifname}>
                        {iface.ifname}{iface.ipv4 ? ` (${iface.ipv4})` : ''}
                      </option>
                    ))}
                  </select>
                )}
              </div>

              {/* Container storage path — both local and remote */}
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">Chemin de stockage</label>
                <input
                  type="text"
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 text-white"
                  value={settingsForm.container_storage_path}
                  onChange={(e) => setSettingsForm({ ...settingsForm, container_storage_path: e.target.value })}
                  placeholder="/var/lib/machines"
                />
              </div>

              <div className="flex gap-2 pt-2">
                <Button type="button" variant="secondary" onClick={() => setSettingsHost(null)} className="flex-1">
                  Fermer
                </Button>
                <Button onClick={handleSaveSettings} disabled={savingSettings} className="flex-1">
                  {savingSettings ? (
                    <><RefreshCw className="w-4 h-4 mr-2 animate-spin" />Sauvegarde...</>
                  ) : (
                    <><Check className="w-4 h-4 mr-2" />Sauvegarder</>
                  )}
                </Button>
              </div>
            </div>
          </div>
        </div>
      )}

    </div>
  );
}
