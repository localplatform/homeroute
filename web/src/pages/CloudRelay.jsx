import { useState, useEffect, useCallback } from 'react';
import { Cloud, Power, PowerOff, RefreshCw, Server, Activity, Wifi, Settings, Upload } from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import StatusBadge from '../components/StatusBadge';
import PageHeader from '../components/PageHeader';
import Section from '../components/Section';
import ConfirmModal from '../components/ConfirmModal';
import useWebSocket from '../hooks/useWebSocket';
import {
  getCloudRelayStatus, enableCloudRelay, disableCloudRelay,
  bootstrapCloudRelay, updateCloudRelayConfig,
} from '../api/client';

function CloudRelay() {
  const [status, setStatus] = useState(null);
  const [loading, setLoading] = useState(true);
  const [enabling, setEnabling] = useState(false);
  const [disabling, setDisabling] = useState(false);
  const [bootstrapping, setBootstrapping] = useState(false);
  const [bootstrapLog, setBootstrapLog] = useState(null);
  const [showDisableConfirm, setShowDisableConfirm] = useState(false);
  const [showBootstrapForm, setShowBootstrapForm] = useState(false);
  const [bootstrapForm, setBootstrapForm] = useState({ host: '', ssh_user: 'root', ssh_port: '22', ssh_password: '' });
  const [configEditing, setConfigEditing] = useState(false);
  const [configForm, setConfigForm] = useState({ host: '', ssh_user: '', ssh_port: '' });
  const [savingConfig, setSavingConfig] = useState(false);

  const fetchStatus = useCallback(async () => {
    try {
      const res = await getCloudRelayStatus();
      setStatus(res.data);
    } catch (error) {
      console.error('Error fetching relay status:', error);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchStatus();
    const interval = setInterval(fetchStatus, 10000);
    return () => clearInterval(interval);
  }, [fetchStatus]);

  // Real-time WebSocket updates
  useWebSocket({
    'cloud_relay:status': (data) => {
      setStatus(prev => ({
        ...prev,
        status: data.status?.toLowerCase() || prev?.status,
        latency_ms: data.latency_ms ?? prev?.latency_ms,
        active_streams: data.active_streams ?? prev?.active_streams,
      }));
    },
  });

  async function handleEnable() {
    setEnabling(true);
    try {
      const res = await enableCloudRelay();
      if (res.data.success) {
        await fetchStatus();
      }
    } catch (error) {
      console.error('Enable error:', error);
    } finally {
      setEnabling(false);
    }
  }

  async function handleDisable() {
    setDisabling(true);
    try {
      const res = await disableCloudRelay();
      if (res.data.success) {
        await fetchStatus();
      }
    } catch (error) {
      console.error('Disable error:', error);
    } finally {
      setDisabling(false);
      setShowDisableConfirm(false);
    }
  }

  async function handleBootstrap() {
    if (!bootstrapForm.host.trim() || !bootstrapForm.ssh_user.trim()) return;
    setBootstrapping(true);
    setBootstrapLog(null);
    try {
      const payload = {
        host: bootstrapForm.host.trim(),
        ssh_user: bootstrapForm.ssh_user.trim(),
        ssh_port: parseInt(bootstrapForm.ssh_port) || 22,
      };
      if (bootstrapForm.ssh_password.trim()) {
        payload.ssh_password = bootstrapForm.ssh_password.trim();
      }
      const res = await bootstrapCloudRelay(payload);
      if (res.data.success) {
        setBootstrapLog({ success: true, message: res.data.message, vps_ipv4: res.data.vps_ipv4 });
        setShowBootstrapForm(false);
        await fetchStatus();
      } else {
        setBootstrapLog({ success: false, message: res.data.error || 'Erreur inconnue' });
      }
    } catch (error) {
      const msg = error.response?.data || error.message;
      setBootstrapLog({ success: false, message: typeof msg === 'string' ? msg : JSON.stringify(msg) });
    } finally {
      setBootstrapping(false);
    }
  }

  async function handleSaveConfig() {
    setSavingConfig(true);
    try {
      const payload = {};
      if (configForm.host.trim()) payload.host = configForm.host.trim();
      if (configForm.ssh_user.trim()) payload.ssh_user = configForm.ssh_user.trim();
      if (configForm.ssh_port.trim()) payload.ssh_port = parseInt(configForm.ssh_port);
      await updateCloudRelayConfig(payload);
      setConfigEditing(false);
      await fetchStatus();
    } catch (error) {
      console.error('Config save error:', error);
    } finally {
      setSavingConfig(false);
    }
  }

  const isConnected = status?.status === 'connected';
  const isEnabled = status?.enabled;

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  return (
    <div>
      <PageHeader title="Cloud Relay" icon={Cloud}>
        <div className="flex items-center gap-2">
          {isEnabled ? (
            <StatusBadge status={isConnected ? 'up' : 'unknown'}>
              {status?.status || 'disconnected'}
            </StatusBadge>
          ) : (
            <StatusBadge status="down">Desactive</StatusBadge>
          )}
          <Button onClick={fetchStatus} variant="secondary">
            <RefreshCw className="w-4 h-4" />
          </Button>
        </div>
      </PageHeader>

      {/* Status Overview */}
      <Section title="Tunnel QUIC">
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-px">
          <Card title="Statut" icon={Activity}>
            <div className="flex items-center gap-3">
              <div className={`w-3 h-3 rounded-full ${isConnected ? 'bg-green-500 animate-pulse' : isEnabled ? 'bg-yellow-500' : 'bg-gray-600'}`} />
              <span className={`text-lg font-semibold ${isConnected ? 'text-green-400' : isEnabled ? 'text-yellow-400' : 'text-gray-500'}`}>
                {isConnected ? 'Connecte' : isEnabled ? (status?.status || 'Deconnecte') : 'Desactive'}
              </span>
            </div>
            <p className="text-xs text-gray-500 mt-2">
              {isEnabled ? 'Mode relay via VPS' : 'Mode direct (IPv6)'}
            </p>
          </Card>

          <Card title="VPS" icon={Server}>
            <div className="text-sm font-mono text-blue-400">
              {status?.vps_host || '-'}
            </div>
            {status?.vps_ipv4 && (
              <div className="text-xs font-mono text-gray-400 mt-1">
                IPv4: {status.vps_ipv4}
              </div>
            )}
          </Card>

          <Card title="Latence" icon={Wifi}>
            <div className={`text-2xl font-bold ${status?.latency_ms != null ? (status.latency_ms < 50 ? 'text-green-400' : status.latency_ms < 100 ? 'text-yellow-400' : 'text-red-400') : 'text-gray-600'}`}>
              {status?.latency_ms != null ? `${status.latency_ms} ms` : '-'}
            </div>
            <p className="text-xs text-gray-500 mt-1">Tunnel QUIC</p>
          </Card>

          <Card title="Connexions actives" icon={Activity}>
            <div className="text-2xl font-bold text-blue-400">
              {status?.active_streams ?? '-'}
            </div>
            <p className="text-xs text-gray-500 mt-1">Streams bidirectionnels</p>
          </Card>
        </div>
      </Section>

      {/* Actions */}
      <Section title="Actions" contrast>
        <div className="flex flex-wrap gap-3">
          {!isEnabled ? (
            <Button onClick={handleEnable} loading={enabling} variant="success">
              <Power className="w-4 h-4" />
              Activer le relay
            </Button>
          ) : (
            <Button onClick={() => setShowDisableConfirm(true)} variant="danger">
              <PowerOff className="w-4 h-4" />
              Desactiver le relay
            </Button>
          )}
          <Button onClick={() => setShowBootstrapForm(!showBootstrapForm)} variant="primary">
            <Upload className="w-4 h-4" />
            Bootstrap VPS
          </Button>
          <Button onClick={() => {
            setConfigEditing(!configEditing);
            if (!configEditing) {
              setConfigForm({
                host: status?.vps_host || '',
                ssh_user: '',
                ssh_port: '',
              });
            }
          }} variant="secondary">
            <Settings className="w-4 h-4" />
            Configuration
          </Button>
        </div>

        {/* Bootstrap Form */}
        {showBootstrapForm && (
          <div className="mt-4 bg-gray-800 border border-gray-700 p-4 max-w-lg">
            <h3 className="text-sm font-semibold mb-3">Deployer hr-cloud-relay sur la VPS</h3>
            <p className="text-xs text-gray-400 mb-4">
              Le binaire sera compile, copie via SCP, et installe comme service systemd sur la VPS.
            </p>
            <div className="space-y-3">
              <div>
                <label className="block text-xs text-gray-400 mb-1">Hote VPS (IP ou hostname)</label>
                <input
                  type="text"
                  value={bootstrapForm.host}
                  onChange={(e) => setBootstrapForm(f => ({ ...f, host: e.target.value }))}
                  placeholder="vps.example.com"
                  className="w-full bg-gray-900 border border-gray-600 px-3 py-2 text-sm font-mono text-white focus:outline-none focus:border-blue-500"
                />
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-xs text-gray-400 mb-1">Utilisateur SSH</label>
                  <input
                    type="text"
                    value={bootstrapForm.ssh_user}
                    onChange={(e) => setBootstrapForm(f => ({ ...f, ssh_user: e.target.value }))}
                    className="w-full bg-gray-900 border border-gray-600 px-3 py-2 text-sm font-mono text-white focus:outline-none focus:border-blue-500"
                  />
                </div>
                <div>
                  <label className="block text-xs text-gray-400 mb-1">Port SSH</label>
                  <input
                    type="number"
                    value={bootstrapForm.ssh_port}
                    onChange={(e) => setBootstrapForm(f => ({ ...f, ssh_port: e.target.value }))}
                    className="w-full bg-gray-900 border border-gray-600 px-3 py-2 text-sm font-mono text-white focus:outline-none focus:border-blue-500"
                  />
                </div>
              </div>
              <div>
                <label className="block text-xs text-gray-400 mb-1">Mot de passe SSH (optionnel, pour sudo)</label>
                <input
                  type="password"
                  value={bootstrapForm.ssh_password}
                  onChange={(e) => setBootstrapForm(f => ({ ...f, ssh_password: e.target.value }))}
                  placeholder="Laisser vide si cle SSH"
                  className="w-full bg-gray-900 border border-gray-600 px-3 py-2 text-sm font-mono text-white focus:outline-none focus:border-blue-500"
                />
              </div>
              <div className="flex gap-2 pt-2">
                <Button onClick={handleBootstrap} loading={bootstrapping} variant="success" disabled={!bootstrapForm.host.trim()}>
                  <Upload className="w-4 h-4" />
                  Deployer
                </Button>
                <Button onClick={() => setShowBootstrapForm(false)} variant="secondary">
                  Annuler
                </Button>
              </div>
            </div>
          </div>
        )}

        {/* Bootstrap Result */}
        {bootstrapLog && (
          <div className={`mt-4 p-3 border ${bootstrapLog.success ? 'border-green-700 bg-green-900/20' : 'border-red-700 bg-red-900/20'}`}>
            <p className={`text-sm ${bootstrapLog.success ? 'text-green-400' : 'text-red-400'}`}>
              {bootstrapLog.message}
            </p>
            {bootstrapLog.vps_ipv4 && (
              <p className="text-xs text-gray-400 mt-1">IPv4 VPS: <span className="font-mono text-blue-400">{bootstrapLog.vps_ipv4}</span></p>
            )}
          </div>
        )}

        {/* Config Editor */}
        {configEditing && (
          <div className="mt-4 bg-gray-800 border border-gray-700 p-4 max-w-lg">
            <h3 className="text-sm font-semibold mb-3">Modifier la configuration</h3>
            <div className="space-y-3">
              <div>
                <label className="block text-xs text-gray-400 mb-1">Hote VPS</label>
                <input
                  type="text"
                  value={configForm.host}
                  onChange={(e) => setConfigForm(f => ({ ...f, host: e.target.value }))}
                  placeholder="vps.example.com"
                  className="w-full bg-gray-900 border border-gray-600 px-3 py-2 text-sm font-mono text-white focus:outline-none focus:border-blue-500"
                />
              </div>
              <div className="grid grid-cols-2 gap-3">
                <div>
                  <label className="block text-xs text-gray-400 mb-1">Utilisateur SSH</label>
                  <input
                    type="text"
                    value={configForm.ssh_user}
                    onChange={(e) => setConfigForm(f => ({ ...f, ssh_user: e.target.value }))}
                    className="w-full bg-gray-900 border border-gray-600 px-3 py-2 text-sm font-mono text-white focus:outline-none focus:border-blue-500"
                  />
                </div>
                <div>
                  <label className="block text-xs text-gray-400 mb-1">Port SSH</label>
                  <input
                    type="number"
                    value={configForm.ssh_port}
                    onChange={(e) => setConfigForm(f => ({ ...f, ssh_port: e.target.value }))}
                    className="w-full bg-gray-900 border border-gray-600 px-3 py-2 text-sm font-mono text-white focus:outline-none focus:border-blue-500"
                  />
                </div>
              </div>
              <div className="flex gap-2 pt-2">
                <Button onClick={handleSaveConfig} loading={savingConfig} variant="success">
                  Enregistrer
                </Button>
                <Button onClick={() => setConfigEditing(false)} variant="secondary">
                  Annuler
                </Button>
              </div>
            </div>
          </div>
        )}
      </Section>

      {/* Architecture Info */}
      <Section title="Architecture">
        <Card title="Flux du trafic" icon={Cloud}>
          <div className="font-mono text-xs text-gray-400 space-y-2">
            <div className="flex items-center gap-2 flex-wrap">
              <span className="text-blue-400">Client</span>
              <span className="text-gray-600">&rarr;</span>
              <span className="text-orange-400">Cloudflare</span>
              <span className="text-gray-600">&rarr;</span>
              <span className="text-purple-400">VPS :443</span>
              <span className="text-green-400">=== QUIC ===&gt;</span>
              <span className="text-cyan-400">On-Prem</span>
              <span className="text-gray-600">&rarr;</span>
              <span className="text-green-400">Proxy</span>
            </div>
            <div className="border-t border-gray-700 pt-2 mt-2">
              <p className="text-gray-500">Le VPS relaie les octets TCP bruts via un tunnel QUIC multiplex.</p>
              <p className="text-gray-500">Le TLS est termine sur le on-prem (certificat Let&apos;s Encrypt).</p>
              <p className="text-gray-500">Authentification mTLS entre VPS et on-prem (CA auto-signe).</p>
            </div>
          </div>
        </Card>
      </Section>

      {/* Disable Confirmation Modal */}
      <ConfirmModal
        isOpen={showDisableConfirm}
        onClose={() => setShowDisableConfirm(false)}
        onConfirm={handleDisable}
        title="Desactiver le Cloud Relay"
        message="Le DNS Cloudflare sera bascule vers le mode direct (AAAA IPv6). Le trafic externe ne passera plus par la VPS. Confirmer ?"
        confirmText="Desactiver"
        variant="danger"
        loading={disabling}
      />
    </div>
  );
}

export default CloudRelay;
