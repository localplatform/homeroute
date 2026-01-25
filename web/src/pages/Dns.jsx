import { useState, useEffect } from 'react';
import { Server, Search } from 'lucide-react';
import Card from '../components/Card';
import { getDnsConfig, getDhcpLeases } from '../api/client';

function Dns() {
  const [config, setConfig] = useState(null);
  const [leases, setLeases] = useState([]);
  const [search, setSearch] = useState('');
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    async function fetchData() {
      try {
        const [configRes, leasesRes] = await Promise.all([
          getDnsConfig(),
          getDhcpLeases()
        ]);

        if (configRes.data.success) setConfig(configRes.data.config);
        if (leasesRes.data.success) setLeases(leasesRes.data.leases);
      } catch (error) {
        console.error('Error:', error);
      } finally {
        setLoading(false);
      }
    }

    fetchData();
  }, []);

  const filteredLeases = leases.filter(lease =>
    lease.hostname?.toLowerCase().includes(search.toLowerCase()) ||
    lease.ip.includes(search) ||
    lease.mac.toLowerCase().includes(search.toLowerCase())
  );

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold">DNS / DHCP</h1>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Configuration */}
        <Card title="Configuration dnsmasq" icon={Server}>
          <dl className="space-y-3 text-sm">
            <div className="flex justify-between">
              <dt className="text-gray-400">Interface</dt>
              <dd className="font-mono text-blue-400">{config?.interface || '-'}</dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-gray-400">Domaine</dt>
              <dd className="font-mono">{config?.domain || '-'}</dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-gray-400">Plage DHCP</dt>
              <dd className="font-mono text-green-400">{config?.dhcpRange || '-'}</dd>
            </div>
            <div className="flex justify-between">
              <dt className="text-gray-400">Cache DNS</dt>
              <dd className="font-mono">{config?.cacheSize || '-'} entrées</dd>
            </div>
            <div className="border-t border-gray-700 pt-3">
              <dt className="text-gray-400 mb-2">Serveurs DNS upstream</dt>
              <dd className="space-y-1">
                {config?.dnsServers?.map(server => (
                  <div key={server} className="font-mono text-sm bg-gray-900 px-2 py-1 rounded">
                    {server}
                  </div>
                ))}
              </dd>
            </div>
            <div className="border-t border-gray-700 pt-3">
              <dt className="text-gray-400 mb-2">Options DHCP</dt>
              <dd className="space-y-1">
                {config?.dhcpOptions?.map((opt, i) => (
                  <div key={i} className="font-mono text-xs bg-gray-900 px-2 py-1 rounded">
                    {opt}
                  </div>
                ))}
              </dd>
            </div>
            {config?.wildcardAddress && (
              <div className="border-t border-gray-700 pt-3">
                <dt className="text-gray-400 mb-2">Wildcard DNS</dt>
                <dd className="font-mono text-sm">
                  *.{config.wildcardAddress.domain} → {config.wildcardAddress.ip}
                </dd>
              </div>
            )}
          </dl>
        </Card>

        {/* IPv6 Configuration */}
        <Card title="Configuration IPv6" icon={Server}>
          <dl className="space-y-3 text-sm">
            <div className="flex justify-between">
              <dt className="text-gray-400">Router Advertisement</dt>
              <dd className="font-mono">
                {config?.ipv6?.raEnabled ? (
                  <span className="text-green-400">Activé</span>
                ) : (
                  <span className="text-gray-500">Désactivé</span>
                )}
              </dd>
            </div>
            {config?.ipv6?.dhcpRange && (
              <div className="flex justify-between">
                <dt className="text-gray-400">Plage DHCPv6</dt>
                <dd className="font-mono text-xs">{config.ipv6.dhcpRange}</dd>
              </div>
            )}
            {config?.ipv6?.options?.map((opt, i) => (
              <div key={i} className="font-mono text-xs bg-gray-900 px-2 py-1 rounded">
                {opt}
              </div>
            ))}
          </dl>
        </Card>
      </div>

      {/* DHCP Leases */}
      <Card
        title={`Baux DHCP (${filteredLeases.length})`}
        icon={Server}
        actions={
          <div className="relative">
            <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-gray-400" />
            <input
              type="text"
              placeholder="Rechercher..."
              value={search}
              onChange={e => setSearch(e.target.value)}
              className="pl-9 pr-4 py-1.5 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
            />
          </div>
        }
      >
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-gray-400 border-b border-gray-700">
                <th className="pb-2">Hostname</th>
                <th className="pb-2">Adresse IP</th>
                <th className="pb-2">Adresse MAC</th>
                <th className="pb-2">Expiration</th>
              </tr>
            </thead>
            <tbody>
              {filteredLeases.map(lease => (
                <tr key={lease.mac} className="border-b border-gray-700/50 hover:bg-gray-700/30">
                  <td className="py-2 font-mono">
                    {lease.hostname || <span className="text-gray-500">-</span>}
                  </td>
                  <td className="py-2 font-mono text-blue-400">{lease.ip}</td>
                  <td className="py-2 font-mono text-gray-400 text-xs">{lease.mac}</td>
                  <td className="py-2 text-gray-400 text-xs">
                    {new Date(lease.expiration).toLocaleString('fr-FR')}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Card>
    </div>
  );
}

export default Dns;
