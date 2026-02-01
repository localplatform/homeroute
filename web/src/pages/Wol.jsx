import { useState, useEffect } from 'react';
import { Power, Plus, Trash2, Play, Square, RotateCw, Clock, X, Check } from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import StatusBadge from '../components/StatusBadge';
import {
  getServers,
  sendWakeOnLan,
  shutdownServer,
  rebootServer,
  getWolSchedules,
  addWolSchedule,
  deleteWolSchedule,
  updateWolSchedule
} from '../api/client';
import useWebSocket from '../hooks/useWebSocket';

export default function Wol() {
  const [servers, setServers] = useState([]);
  const [schedules, setSchedules] = useState([]);
  const [loading, setLoading] = useState(true);
  const [selectedServer, setSelectedServer] = useState(null);
  const [showScheduleModal, setShowScheduleModal] = useState(false);

  // Schedule form
  const [scheduleForm, setScheduleForm] = useState({
    serverId: '',
    action: 'wake',
    cron: '0 7 * * *',
    description: '',
    enabled: true
  });
  const [addingSchedule, setAddingSchedule] = useState(false);
  const [scheduleError, setScheduleError] = useState('');

  useWebSocket({
    'servers:status': (data) => {
      setServers(prevServers =>
        prevServers.map(server =>
          server.id === data.serverId
            ? { ...server, status: data.online ? 'online' : 'offline', latency: data.latency }
            : server
        )
      );
    }
  });

  useEffect(() => {
    loadData();
  }, []);

  const loadData = async () => {
    try {
      setLoading(true);
      const [serversRes, schedulesRes] = await Promise.all([
        getServers(),
        getWolSchedules()
      ]);
      setServers(serversRes.data.data || []);
      setSchedules(schedulesRes.data.data || []);
    } catch (error) {
      console.error('Failed to load data:', error);
    } finally {
      setLoading(false);
    }
  };

  const handleWake = async (serverId) => {
    try {
      const response = await sendWakeOnLan(serverId);
      if (response.data.success) {
        alert('WOL magic packet sent!');
      }
    } catch (error) {
      console.error('Failed to send WOL:', error);
      alert('Failed to send WOL: ' + error.message);
    }
  };

  const handleShutdown = async (serverId) => {
    if (!confirm('Are you sure you want to shutdown this server?')) {
      return;
    }

    try {
      const response = await shutdownServer(serverId);
      if (response.data.success) {
        alert('Shutdown command sent!');
      }
    } catch (error) {
      console.error('Failed to shutdown server:', error);
      alert('Failed to shutdown: ' + error.message);
    }
  };

  const handleReboot = async (serverId) => {
    if (!confirm('Are you sure you want to reboot this server?')) {
      return;
    }

    try {
      const response = await rebootServer(serverId);
      if (response.data.success) {
        alert('Reboot command sent!');
      }
    } catch (error) {
      console.error('Failed to reboot server:', error);
      alert('Failed to reboot: ' + error.message);
    }
  };

  const handleAddSchedule = async (e) => {
    e.preventDefault();
    setAddingSchedule(true);
    setScheduleError('');

    try {
      const response = await addWolSchedule(scheduleForm);
      if (response.data.success) {
        setSchedules([...schedules, response.data.data]);
        setShowScheduleModal(false);
        resetScheduleForm();
      }
    } catch (error) {
      console.error('Failed to add schedule:', error);
      setScheduleError(error.response?.data?.error || error.message);
    } finally {
      setAddingSchedule(false);
    }
  };

  const handleDeleteSchedule = async (id) => {
    if (!confirm('Are you sure you want to delete this schedule?')) {
      return;
    }

    try {
      await deleteWolSchedule(id);
      setSchedules(schedules.filter(s => s.id !== id));
    } catch (error) {
      console.error('Failed to delete schedule:', error);
      alert('Failed to delete schedule: ' + error.message);
    }
  };

  const handleToggleSchedule = async (id, enabled) => {
    try {
      const response = await updateWolSchedule(id, { enabled });
      if (response.data.success) {
        setSchedules(schedules.map(s => s.id === id ? response.data.data : s));
      }
    } catch (error) {
      console.error('Failed to toggle schedule:', error);
      alert('Failed to toggle schedule: ' + error.message);
    }
  };

  const resetScheduleForm = () => {
    setScheduleForm({
      serverId: '',
      action: 'wake',
      cron: '0 7 * * *',
      description: '',
      enabled: true
    });
    setScheduleError('');
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

  const getActionIcon = (action) => {
    switch (action) {
      case 'wake':
        return Play;
      case 'shutdown':
        return Square;
      case 'reboot':
        return RotateCw;
      default:
        return Power;
    }
  };

  const getActionColor = (action) => {
    switch (action) {
      case 'wake':
        return 'bg-green-600/20 text-green-400';
      case 'shutdown':
        return 'bg-red-600/20 text-red-400';
      case 'reboot':
        return 'bg-yellow-600/20 text-yellow-400';
      default:
        return 'bg-blue-600/20 text-blue-400';
    }
  };

  if (loading) {
    return <div className="text-center py-12 text-gray-400">Loading...</div>;
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold text-white">Wake-on-LAN</h1>
        <p className="text-gray-400 mt-1">
          Control servers remotely: wake, shutdown, reboot
        </p>
      </div>

      {/* Server Controls */}
      <div>
        <h2 className="text-lg font-semibold text-white mb-4">Server Controls</h2>
        {servers.length === 0 ? (
          <Card title="No servers" icon={Power}>
            <p className="text-gray-400">
              No servers configured. Go to the Servers page to add servers.
            </p>
          </Card>
        ) : (
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
            {servers.map((server) => (
              <Card key={server.id} title={server.name} icon={Power}>
                <div className="space-y-3">
                  <div className="flex items-center justify-between">
                    <StatusBadge status={getStatusColor(server.status)}>
                      {server.status || 'unknown'}
                    </StatusBadge>
                    {server.latency && (
                      <span className="text-sm text-gray-400">{server.latency}ms</span>
                    )}
                  </div>

                  <div className="text-sm space-y-1">
                    <div className="flex justify-between">
                      <span className="text-gray-400">Host:</span>
                      <span className="text-white font-mono text-xs">{server.host}</span>
                    </div>
                    <div className="flex justify-between">
                      <span className="text-gray-400">MAC:</span>
                      <span className="text-white font-mono text-xs">{server.mac}</span>
                    </div>
                  </div>

                  <div className="flex gap-2 pt-2 border-t border-gray-700">
                    <Button
                      variant="success"
                      onClick={() => handleWake(server.id)}
                      disabled={server.status === 'online'}
                      className="flex-1 text-xs"
                    >
                      <Play className="w-3 h-3 mr-1" />
                      Wake
                    </Button>
                    <Button
                      variant="warning"
                      onClick={() => handleReboot(server.id)}
                      disabled={server.status !== 'online'}
                      className="flex-1 text-xs"
                    >
                      <RotateCw className="w-3 h-3 mr-1" />
                      Reboot
                    </Button>
                    <Button
                      variant="danger"
                      onClick={() => handleShutdown(server.id)}
                      disabled={server.status !== 'online'}
                      className="flex-1 text-xs"
                    >
                      <Square className="w-3 h-3 mr-1" />
                      Shutdown
                    </Button>
                  </div>
                </div>
              </Card>
            ))}
          </div>
        )}
      </div>

      {/* Schedules */}
      <div>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold text-white">Schedules</h2>
          <Button onClick={() => setShowScheduleModal(true)} disabled={servers.length === 0}>
            <Plus className="w-4 h-4 mr-2" />
            Add Schedule
          </Button>
        </div>

        {schedules.length === 0 ? (
          <Card title="No schedules" icon={Clock}>
            <p className="text-gray-400">
              No schedules configured. Click "Add Schedule" to create one.
            </p>
          </Card>
        ) : (
          <div className="space-y-3">
            {schedules.map((schedule) => {
              const ActionIcon = getActionIcon(schedule.action);
              return (
                <Card key={schedule.id}>
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-3 flex-1">
                      <div className={`p-2 rounded ${getActionColor(schedule.action)}`}>
                        <ActionIcon className="w-4 h-4" />
                      </div>
                      <div className="flex-1">
                        <div className="flex items-center gap-2">
                          <span className="text-white font-medium">{schedule.serverName}</span>
                          <span className="text-gray-400">•</span>
                          <span className="text-gray-300">{schedule.action}</span>
                        </div>
                        <div className="text-sm text-gray-400 mt-1">
                          <Clock className="w-3 h-3 inline mr-1" />
                          {schedule.cron}
                          {schedule.description && ` - ${schedule.description}`}
                        </div>
                        {schedule.lastRun && (
                          <div className="text-xs text-gray-500 mt-1">
                            Last run: {new Date(schedule.lastRun).toLocaleString()}
                          </div>
                        )}
                      </div>
                    </div>
                    <div className="flex items-center gap-2">
                      <label className="flex items-center gap-2 cursor-pointer">
                        <input
                          type="checkbox"
                          checked={schedule.enabled}
                          onChange={(e) => handleToggleSchedule(schedule.id, e.target.checked)}
                          className="w-4 h-4 rounded"
                        />
                        <span className="text-sm text-gray-400">Enabled</span>
                      </label>
                      <Button
                        variant="danger"
                        onClick={() => handleDeleteSchedule(schedule.id)}
                        className="text-xs"
                      >
                        <Trash2 className="w-3 h-3" />
                      </Button>
                    </div>
                  </div>
                </Card>
              );
            })}
          </div>
        )}
      </div>

      {/* Add Schedule Modal */}
      {showScheduleModal && (
        <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
          <div className="bg-gray-800 rounded-lg p-6 w-full max-w-md">
            <div className="flex items-center justify-between mb-4">
              <h2 className="text-xl font-bold text-white">Add Schedule</h2>
              <button
                onClick={() => {
                  setShowScheduleModal(false);
                  resetScheduleForm();
                }}
                className="text-gray-400 hover:text-white"
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            <form onSubmit={handleAddSchedule} className="space-y-4">
              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Server *
                </label>
                <select
                  value={scheduleForm.serverId}
                  onChange={(e) => setScheduleForm({ ...scheduleForm, serverId: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded text-white focus:ring-2 focus:ring-blue-500"
                  required
                >
                  <option value="">Select server...</option>
                  {servers.map((server) => (
                    <option key={server.id} value={server.id}>
                      {server.name}
                    </option>
                  ))}
                </select>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Action *
                </label>
                <select
                  value={scheduleForm.action}
                  onChange={(e) => setScheduleForm({ ...scheduleForm, action: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded text-white focus:ring-2 focus:ring-blue-500"
                  required
                >
                  <option value="wake">Wake</option>
                  <option value="shutdown">Shutdown</option>
                  <option value="reboot">Reboot</option>
                </select>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Cron Expression *
                </label>
                <input
                  type="text"
                  value={scheduleForm.cron}
                  onChange={(e) => setScheduleForm({ ...scheduleForm, cron: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded text-white font-mono focus:ring-2 focus:ring-blue-500"
                  placeholder="0 7 * * *"
                  required
                />
                <p className="text-xs text-gray-400 mt-1">
                  Format: minute hour day month weekday (e.g., "0 7 * * *" = every day at 7:00 AM)
                </p>
              </div>

              <div>
                <label className="block text-sm font-medium text-gray-300 mb-1">
                  Description
                </label>
                <input
                  type="text"
                  value={scheduleForm.description}
                  onChange={(e) => setScheduleForm({ ...scheduleForm, description: e.target.value })}
                  className="w-full px-3 py-2 bg-gray-700 border border-gray-600 rounded text-white focus:ring-2 focus:ring-blue-500"
                  placeholder="Daily morning wake"
                />
              </div>

              <div className="flex items-center gap-2">
                <input
                  type="checkbox"
                  id="enabled"
                  checked={scheduleForm.enabled}
                  onChange={(e) => setScheduleForm({ ...scheduleForm, enabled: e.target.checked })}
                  className="w-4 h-4 rounded"
                />
                <label htmlFor="enabled" className="text-sm text-gray-300 cursor-pointer">
                  Enable schedule immediately
                </label>
              </div>

              {scheduleError && (
                <div className="p-3 bg-red-900/20 border border-red-600 rounded text-red-400 text-sm">
                  {scheduleError}
                </div>
              )}

              <div className="flex gap-2 pt-2">
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => {
                    setShowScheduleModal(false);
                    resetScheduleForm();
                  }}
                  className="flex-1"
                >
                  Cancel
                </Button>
                <Button
                  type="submit"
                  disabled={addingSchedule}
                  className="flex-1"
                >
                  {addingSchedule ? (
                    <>
                      <RotateCw className="w-4 h-4 mr-2 animate-spin" />
                      Adding...
                    </>
                  ) : (
                    <>
                      <Check className="w-4 h-4 mr-2" />
                      Add Schedule
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
