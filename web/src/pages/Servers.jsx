import { useState, useEffect } from 'react';
import { Server, Plus, Trash2, Edit, RefreshCw, Activity, X, Check } from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import StatusBadge from '../components/StatusBadge';
import {
  getServers,
  addServer,
  updateServer,
  deleteServer,
  testServerConnection,
  refreshServerInterfaces
} from '../api/client';
import useWebSocket from '../hooks/useWebSocket';

export default function Servers() {
  const [servers, setServers] = useState([]);
  const [loading, setLoading] = useState(true);
  const [showAddModal, setShowAddModal] = useState(false);
  const [editingServer, setEditingServer] = useState(null);

  // Form state
  const [formData, setFormData] = useState({
    name: '',
    host: '',
    port: 22,
    username: 'root',
    password: '',
    groups: '',
    wolInterface: ''
  });
  const [interfaces, setInterfaces] = useState([]);
  const [addingServer, setAddingServer] = useState(false);
  const [addError, setAddError] = useState('');

  useWebSocket({
    'servers:status': (data) => {
      setServers(prevServers =>
        prevServers.map(server =>
          server.id === data.serverId
            ? { ...server, status: data.online ? 'online' : 'offline', latency: data.latency, lastSeen: data.lastSeen }
            : server
        )
      );
    }
  });

  useEffect(() => {
    loadServers();
  }, []);

  const loadServers = async () => {
    try {
      setLoading(true);
      const response = await getServers();
      setServers(response.data.data || []);
    } catch (error) {
      console.error('Failed to load servers:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleAddServer = async (e) => {
    e.preventDefault();
    setAddingServer(true);
    setAddError('');

    try {
      const groups = formData.groups
        .split(',')
        .map(g => g.trim())
        .filter(g => g);

      const response = await addServer({
        ...formData,
        groups,
        port: parseInt(formData.port)
      });

      if (response.data.success) {
        setServers([...servers, response.data.data]);
        setShowAddModal(false);
        resetForm();
      } else {
        setAddError(response.data.error || 'Failed to add server');
      }
    } catch (error) {
      console.error('Failed to add server:', error);
      setAddError(error.response?.data?.error || error.message || 'Failed to add server');
    } finally {
      setAddingServer(false);
    }
  };

  const handleDeleteServer = async (id) => {
    if (!confirm('Are you sure you want to delete this server?')) {
      return;
    }

    try {
      await deleteServer(id);
      setServers(servers.filter(s => s.id !== id));
    } catch (error) {
      console.error('Failed to delete server:', error);
      alert('Failed to delete server: ' + error.message);
    }
  };

  const handleTestConnection = async (id) => {
    try {
      const response = await testServerConnection(id);
      if (response.data.success) {
        alert('Connection successful!');
      } else {
        alert('Connection failed: ' + response.data.message);
      }
    } catch (error) {
      console.error('Failed to test connection:', error);
      alert('Connection failed: ' + error.message);
    }
  };

  const handleRefreshInterfaces = async (id) => {
    try {
      const response = await refreshServerInterfaces(id);
      if (response.data.success) {
        alert(`Interfaces refreshed! Found ${response.data.data.length} interface(s)`);
        loadServers();
      }
    } catch (error) {
      console.error('Failed to refresh interfaces:', error);
      alert('Failed to refresh interfaces: ' + error.message);
    }
  };

  const resetForm = () => {
    setFormData({
      name: '',
      host: '',
      port: 22,
      username: 'root',
      password: '',
      groups: '',
      wolInterface: ''
    });
    setInterfaces([]);
    setAddError('');
  };

  const handleCloseModal = () => {
    setShowAddModal(false);
    setEditingServer(null);
    resetForm();
  };

  const getStatusColor = (status) => {
    switch (status) {
      case 'online':
        return 'success';
      case 'offline':
        return 'danger';
      default:
        return 'secondary';
    }
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-white">Servers</h1>
          <p className="text-gray-400 mt-1">
            Manage remote servers for Wake-on-LAN and monitoring
          </p>
        </div>
        <Button onClick={() => setShowAddModal(true)}>
          <Plus className="w-4 h-4 mr-2" />
          Add Server
        </Button>
      </div>

      {loading ? (
        <div className="text-center py-12 text-gray-400">Loading servers...</div>
      ) : servers.length === 0 ? (
        <Card title="No servers" icon={Server}>
          <p className="text-gray-400">
            No servers configured yet. Click "Add Server" to get started.
          </p>
        </Card>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-6">
          {servers.map((server) => (
            <Card
              key={server.id}
              title={server.name}
              icon={Server}
            >
              <div className="space-y-3">
                <div className="flex items-center justify-between">
                  <StatusBadge status={getStatusColor(server.status)}>
                    {server.status || 'unknown'}
                  </StatusBadge>
                  {server.latency && (
                    <span className="text-sm text-gray-400">{server.latency}ms</span>
                  )}
                </div>

                <div className="space-y-2 text-sm">
                  <div className="flex justify-between">
                    <span className="text-gray-400">Host:</span>
                    <span className="text-white font-mono">{server.host}:{server.port}</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-400">User:</span>
                    <span className="text-white">{server.username}</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-400">Interface:</span>
                    <span className="text-white font-mono">{server.interface}</span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-400">MAC:</span>
                    <span className="text-white font-mono text-xs">{server.mac}</span>
                  </div>
                  {server.ipv4 && (
                    <div className="flex justify-between">
                      <span className="text-gray-400">IPv4:</span>
                      <span className="text-white font-mono text-xs">{server.ipv4}</span>
                    </div>
                  )}
                </div>

                {server.groups && server.groups.length > 0 && (
                  <div className="flex flex-wrap gap-1">
                    {server.groups.map((group, idx) => (
                      <span
                        key={idx}
                        className="px-2 py-1 text-xs bg-blue-600/20 text-blue-400 rounded"
                      >
                        {group}
                      </span>
                    ))}
                  </div>
                )}

                {server.lastSeen && (
                  <div className="text-xs text-gray-500">
                    Last seen: {new Date(server.lastSeen).toLocaleString()}
                  </div>
                )}

                <div className="flex gap-2 pt-2 border-t border-gray-700">
                  <Button
                    variant="secondary"
                    onClick={() => handleTestConnection(server.id)}
                    className="flex-1 text-xs"
                  >
                    <Activity className="w-3 h-3 mr-1" />
                    Test
                  </Button>
                  <Button
                    variant="secondary"
                    onClick={() => handleRefreshInterfaces(server.id)}
                    className="flex-1 text-xs"
                  >
                    <RefreshCw className="w-3 h-3 mr-1" />
                    Refresh
                  </Button>
                  <Button
                    variant="danger"
                    onClick={() => handleDeleteServer(server.id)}
                    className="text-xs"
                  >
                    <Trash2 className="w-3 h-3" />
                  </Button>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}

      {/* Add Server Modal */}
      {showAddModal && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-xl font-bold text-white">Add Server</h2>
              <button
                onClick={handleCloseModal}
                className="text-gray-400 hover:text-white"
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            <form onSubmit={handleAddServer} className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Server Name *
                </label>
                <input
                  type="text"
                  value={formData.name}
                  onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded text-white focus:ring-2 focus:ring-blue-500"
                  placeholder="My Server"
                  required
                />
              </div>

              <div className="grid grid-cols-3 gap-2">
                <div className="col-span-2">
                  <label className="block text-sm font-medium text-gray-300 mb-1">
                    Host/IP *
                  </label>
                  <input
                    type="text"
                    value={formData.host}
                    onChange={(e) => setFormData({ ...formData, host: e.target.value })}
                    className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded text-white focus:ring-2 focus:ring-blue-500"
                    placeholder="192.168.1.100"
                    required
                  />
                </div>
                <div>
                  <label className="block text-sm font-medium text-gray-300 mb-1">
                    Port
                  </label>
                  <input
                    type="number"
                    value={formData.port}
                    onChange={(e) => setFormData({ ...formData, port: e.target.value })}
                    className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded text-white focus:ring-2 focus:ring-blue-500"
                    placeholder="22"
                  />
                </div>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  SSH Username *
                </label>
                <input
                  type="text"
                  value={formData.username}
                  onChange={(e) => setFormData({ ...formData, username: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded text-white focus:ring-2 focus:ring-blue-500"
                  placeholder="root"
                  required
                />
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  SSH Password *
                </label>
                <input
                  type="password"
                  value={formData.password}
                  onChange={(e) => setFormData({ ...formData, password: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded text-white focus:ring-2 focus:ring-blue-500"
                  placeholder="••••••••"
                  required
                />
                <p className="text-xs text-gray-400 mt-1">
                  Used once to setup SSH key authentication
                </p>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Groups (comma-separated)
                </label>
                <input
                  type="text"
                  value={formData.groups}
                  onChange={(e) => setFormData({ ...formData, groups: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded text-white focus:ring-2 focus:ring-blue-500"
                  placeholder="storage, critical"
                />
              </div>

              {addError && (
                <div className="p-3 bg-red-900/20 border border-red-600 rounded text-red-400 text-sm">
                  {addError}
                </div>
              )}

              <div className="flex gap-2 pt-2">
                <Button
                  type="button"
                  variant="secondary"
                  onClick={handleCloseModal}
                  className="flex-1"
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  disabled={addingServer}
                  className="flex-1"
                >
                  {addingServer ? (
                    <>
                      <RefreshCw className="w-4 h-4 mr-2 animate-spin" />
                      Adding...
                    </>
                  ) : (
                    <>
                      <Check className="w-4 h-4 mr-2" />
                      Add Server
                    </>
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
