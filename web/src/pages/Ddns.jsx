import { useState, useEffect } from 'react';
import { Globe, RefreshCw, Clock, Wifi } from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import StatusBadge from '../components/StatusBadge';
import { getDdnsStatus, forceDdnsUpdate } from '../api/client';

function Ddns() {
  const [status, setStatus] = useState(null);
  const [loading, setLoading] = useState(true);
  const [updating, setUpdating] = useState(false);

  useEffect(() => {
    fetchStatus();
    const interval = setInterval(fetchStatus, 60000);
    return () => clearInterval(interval);
  }, []);

  async function fetchStatus() {
    try {
      const res = await getDdnsStatus();
      if (res.data.success) {
        setStatus(res.data.status);
      }
    } catch (error) {
      console.error('Error:', error);
    } finally {
      setLoading(false);
    }
  }

  async function handleUpdate() {
    setUpdating(true);
    try {
      await forceDdnsUpdate();
      await fetchStatus();
    } catch (error) {
      console.error('Error updating:', error);
    } finally {
      setUpdating(false);
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-bold">Dynamic DNS (Cloudflare)</h1>
        <Button onClick={handleUpdate} loading={updating}>
          <RefreshCw className="w-4 h-4" />
          Forcer la mise à jour
        </Button>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        <Card title="Enregistrement" icon={Globe}>
          <div className="text-lg font-mono text-blue-400 break-all">
            {status?.config?.recordName || '-'}
          </div>
          <p className="text-xs text-gray-500 mt-2">AAAA Record</p>
        </Card>

        <Card title="IPv6 Actuelle" icon={Wifi}>
          <div className="text-sm font-mono text-green-400 break-all">
            {status?.currentIpv6 || 'Non disponible'}
          </div>
          <p className="text-xs text-gray-500 mt-2">Interface enp5s0</p>
        </Card>

        <Card title="Zone ID" icon={Globe}>
          <div className="text-xs font-mono text-gray-400 break-all">
            {status?.config?.zoneId || '-'}
          </div>
        </Card>

        <Card title="API Token" icon={Globe}>
          <div className="text-sm font-mono text-gray-500">
            {status?.config?.apiToken || '-'}
          </div>
        </Card>
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Status */}
        <Card title="État" icon={Clock}>
          <dl className="space-y-4">
            <div className="flex justify-between items-center">
              <dt className="text-gray-400">Dernière mise à jour</dt>
              <dd className="font-mono text-sm">
                {status?.lastUpdate || 'Jamais'}
              </dd>
            </div>
            <div className="flex justify-between items-center">
              <dt className="text-gray-400">Dernière IP enregistrée</dt>
              <dd className="font-mono text-sm text-purple-400 break-all">
                {status?.lastIp || '-'}
              </dd>
            </div>
            <div className="flex justify-between items-center">
              <dt className="text-gray-400">État</dt>
              <dd>
                {status?.currentIpv6 ? (
                  <StatusBadge status="up">Connecté</StatusBadge>
                ) : (
                  <StatusBadge status="down">Pas d&apos;IPv6</StatusBadge>
                )}
              </dd>
            </div>
          </dl>
        </Card>

        {/* Cron Info */}
        <Card title="Automatisation" icon={RefreshCw}>
          <div className="space-y-3">
            <div className="bg-gray-900 rounded p-3">
              <div className="text-sm font-semibold mb-1">Cron Job</div>
              <code className="text-xs text-green-400">*/2 * * * * /usr/local/bin/cloudflare-ddns-v6.sh</code>
              <p className="text-xs text-gray-500 mt-2">Exécuté toutes les 2 minutes</p>
            </div>
            <div className="bg-gray-900 rounded p-3">
              <div className="text-sm font-semibold mb-1">Script</div>
              <code className="text-xs text-gray-400">/usr/local/bin/cloudflare-ddns-v6.sh</code>
            </div>
            <div className="bg-gray-900 rounded p-3">
              <div className="text-sm font-semibold mb-1">Configuration</div>
              <code className="text-xs text-gray-400">/etc/cloudflare-ddns.conf</code>
            </div>
          </div>
        </Card>
      </div>

      {/* Logs */}
      <Card title="Logs récents" icon={Clock}>
        <div className="bg-gray-900 rounded p-3 max-h-96 overflow-y-auto font-mono text-xs">
          {status?.logs?.length > 0 ? (
            status.logs.map((log, i) => (
              <div
                key={i}
                className={`py-1 ${
                  log.includes('ERREUR') ? 'text-red-400' :
                  log.includes('MAJ') || log.includes('CREE') ? 'text-green-400' :
                  'text-gray-400'
                }`}
              >
                {log}
              </div>
            ))
          ) : (
            <p className="text-gray-500 text-center py-4">Aucun log</p>
          )}
        </div>
      </Card>
    </div>
  );
}

export default Ddns;
