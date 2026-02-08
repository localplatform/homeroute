import { useState, useEffect } from 'react';
import {
  HardDrive, Plus, Trash2, RefreshCw, X, Check,
  Play, Square, RotateCw, Moon
} from 'lucide-react';
import Button from '../components/Button';
import StatusBadge from '../components/StatusBadge';
import PageHeader from '../components/PageHeader';
import Section from '../components/Section';
import {
  getHosts,
  addHost,
  deleteHost,
  wakeHost,
  shutdownHost,
  rebootHost,
  sleepHost,
  setWolMac
} from '../api/client';
import useWebSocket from '../hooks/useWebSocket';

export default function Hosts() {
  const [hosts, setHosts] = useState([]);
  const [loading, setLoading] = useState(true);
  const [showAddModal, setShowAddModal] = useState(false);
  // Add host form
  const [formData, setFormData] = useState({
    name: '',
    host: '',
    port: 22,
    username: 'root',
    password: ''
  });
  const [addingHost, setAddingHost] = useState(false);
  const [addError, setAddError] = useState('');

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
  });

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
    } catch (error) {
      alert('Failed to delete host: ' + error.message);
    }
  };

  // ── Power actions ───────────────────────────

  const handleWake = async (id) => {
    try {
      const res = await wakeHost(id);
      if (res.data.success) {
        alert('Paquet WOL envoye !');
      } else {
        alert('Echec : ' + res.data.error);
      }
    } catch (error) {
      alert('Echec WOL : ' + error.message);
    }
  };

  const handleShutdown = async (id) => {
    if (!confirm('Eteindre cet hote ?')) return;
    try {
      const res = await shutdownHost(id);
      if (res.data.success) alert('Commande envoyee !');
    } catch (error) {
      alert('Echec : ' + error.message);
    }
  };

  const handleReboot = async (id) => {
    if (!confirm('Redemarrer cet hote ?')) return;
    try {
      const res = await rebootHost(id);
      if (res.data.success) alert('Commande envoyee !');
    } catch (error) {
      alert('Echec : ' + error.message);
    }
  };

  const handleSleep = async (id) => {
    if (!confirm('Mettre en veille cet hote ?')) return;
    try {
      const res = await sleepHost(id);
      if (res.data.success) alert('Commande envoyee !');
    } catch (error) {
      alert('Echec : ' + error.message);
    }
  };

  const handleSetWolMac = async (id, mac) => {
    try {
      await setWolMac(id, mac);
      setHosts(prev => prev.map(h => h.id === id ? { ...h, wol_mac: mac } : h));
    } catch (error) {
      console.error('Failed to set WOL MAC:', error);
    }
  };

  // ── Form helpers ────────────────────────────

  const resetForm = () => {
    setFormData({ name: '', host: '', port: 22, username: 'root', password: '' });
    setAddError('');
  };

  const getStatusColor = (status) => {
    switch (status) {
      case 'online': return 'success';
      case 'offline': return 'danger';
      default: return 'secondary';
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
                  <th className="px-3 py-2 text-xs font-medium text-gray-400 uppercase">MAC (WOL)</th>
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
                        <HardDrive className="w-4 h-4 text-blue-400 flex-shrink-0" />
                        {host.name}
                      </div>
                    </td>
                    {/* Status */}
                    <td className="px-3 py-2 text-sm">
                      <StatusBadge status={getStatusColor(host.status)}>
                        {host.status || 'unknown'}
                      </StatusBadge>
                    </td>
                    {/* Address */}
                    <td className="px-3 py-2 text-sm font-mono text-gray-300">
                      {host.host}:{host.port}
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
                    {/* WOL MAC */}
                    <td className="px-3 py-2 text-sm">
                      {host.interfaces && host.interfaces.length > 0 ? (
                        <select
                          className="bg-gray-700 text-xs text-gray-300 border border-gray-600 px-1 py-0.5 rounded-sm"
                          value={host.wol_mac || host.mac || ''}
                          onChange={(e) => handleSetWolMac(host.id, e.target.value)}
                        >
                          {host.interfaces.filter(i => i.address && i.address !== '00:00:00:00:00:00').map((iface, idx) => (
                            <option key={idx} value={iface.address}>
                              {iface.ifname} ({iface.address})
                            </option>
                          ))}
                        </select>
                      ) : (
                        <span className="text-gray-500 text-xs font-mono">{host.mac || '--'}</span>
                      )}
                    </td>
                    {/* Last seen */}
                    <td className="px-3 py-2 text-xs text-gray-500">
                      {host.lastSeen ? new Date(host.lastSeen).toLocaleTimeString() : '--'}
                    </td>
                    {/* Actions */}
                    <td className="px-3 py-2">
                      <div className="flex gap-1">
                        <button
                          onClick={() => handleWake(host.id)}
                          disabled={host.status === 'online'}
                          className="p-1.5 text-green-400 hover:bg-green-600/20 disabled:opacity-30 disabled:cursor-not-allowed"
                          title="Wake"
                        >
                          <Play className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={() => handleReboot(host.id)}
                          disabled={host.status !== 'online'}
                          className="p-1.5 text-yellow-400 hover:bg-yellow-600/20 disabled:opacity-30 disabled:cursor-not-allowed"
                          title="Reboot"
                        >
                          <RotateCw className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={() => handleSleep(host.id)}
                          disabled={host.status !== 'online'}
                          className="p-1.5 text-blue-400 hover:bg-blue-600/20 disabled:opacity-30 disabled:cursor-not-allowed"
                          title="Sleep"
                        >
                          <Moon className="w-3.5 h-3.5" />
                        </button>
                        <button
                          onClick={() => handleShutdown(host.id)}
                          disabled={host.status !== 'online'}
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

    </div>
  );
}
