import { useState, useEffect } from 'react';
import { Link } from 'react-router-dom';
import { Network, Server, Shield, Globe, Wifi, ArrowRight } from 'lucide-react';
import Card from '../components/Card';
import StatusBadge from '../components/StatusBadge';
import { getInterfaces, getDhcpLeases, getAdblockStats, getDdnsStatus } from '../api/client';

function Dashboard() {
  const [data, setData] = useState({
    interfaces: null,
    leases: null,
    adblock: null,
    ddns: null,
    loading: true
  });

  useEffect(() => {
    async function fetchData() {
      try {
        const [ifRes, leaseRes, adblockRes, ddnsRes] = await Promise.all([
          getInterfaces(),
          getDhcpLeases(),
          getAdblockStats(),
          getDdnsStatus()
        ]);

        setData({
          interfaces: ifRes.data.success ? ifRes.data.interfaces : [],
          leases: leaseRes.data.success ? leaseRes.data.leases : [],
          adblock: adblockRes.data.success ? adblockRes.data.stats : null,
          ddns: ddnsRes.data.success ? ddnsRes.data.status : null,
          loading: false
        });
      } catch (error) {
        console.error('Error fetching data:', error);
        setData(prev => ({ ...prev, loading: false }));
      }
    }

    fetchData();
    const interval = setInterval(fetchData, 30000);
    return () => clearInterval(interval);
  }, []);

  if (data.loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  const physicalInterfaces = data.interfaces?.filter(i =>
    ['eno1', 'enp5s0', 'enp7s0f0', 'enp7s0f1'].includes(i.name)
  ) || [];

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold">Dashboard</h1>

      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
        {/* Interfaces Card */}
        <Card
          title="Interfaces Réseau"
          icon={Network}
          actions={
            <Link to="/network" className="text-blue-400 hover:text-blue-300">
              <ArrowRight className="w-4 h-4" />
            </Link>
          }
        >
          <div className="space-y-2">
            {physicalInterfaces.map(iface => (
              <div key={iface.name} className="flex items-center justify-between">
                <span className="text-sm font-mono">{iface.name}</span>
                <StatusBadge status={iface.state === 'UP' ? 'up' : 'down'}>
                  {iface.state}
                </StatusBadge>
              </div>
            ))}
          </div>
          <p className="text-xs text-gray-500 mt-3">
            {data.interfaces?.length || 0} interfaces totales
          </p>
        </Card>

        {/* DHCP Leases Card */}
        <Card
          title="Baux DHCP"
          icon={Wifi}
          actions={
            <Link to="/dns" className="text-blue-400 hover:text-blue-300">
              <ArrowRight className="w-4 h-4" />
            </Link>
          }
        >
          <div className="text-3xl font-bold text-blue-400">
            {data.leases?.length || 0}
          </div>
          <p className="text-sm text-gray-400">appareils connectés</p>
          <div className="mt-3 text-xs text-gray-500">
            {data.leases?.slice(0, 3).map(lease => (
              <div key={lease.mac} className="truncate">
                {lease.hostname || lease.ip}
              </div>
            ))}
          </div>
        </Card>

        {/* AdBlock Card */}
        <Card
          title="AdBlock"
          icon={Shield}
          actions={
            <Link to="/adblock" className="text-blue-400 hover:text-blue-300">
              <ArrowRight className="w-4 h-4" />
            </Link>
          }
        >
          <div className="text-3xl font-bold text-green-400">
            {data.adblock?.domainCount?.toLocaleString() || 0}
          </div>
          <p className="text-sm text-gray-400">domaines bloqués</p>
          <p className="text-xs text-gray-500 mt-3">
            {data.adblock?.sources?.length || 0} sources actives
          </p>
        </Card>

        {/* DDNS Card */}
        <Card
          title="Dynamic DNS"
          icon={Globe}
          actions={
            <Link to="/ddns" className="text-blue-400 hover:text-blue-300">
              <ArrowRight className="w-4 h-4" />
            </Link>
          }
        >
          <div className="text-sm font-mono text-blue-400 break-all">
            {data.ddns?.config?.recordName || '-'}
          </div>
          <p className="text-xs text-gray-500 mt-2 font-mono break-all">
            {data.ddns?.currentIpv6 || 'Pas d\'IPv6'}
          </p>
          <p className="text-xs text-gray-500 mt-2">
            {data.ddns?.lastUpdate ? `MAJ: ${data.ddns.lastUpdate}` : '-'}
          </p>
        </Card>
      </div>

      {/* Recent DHCP Leases Table */}
      <Card title="Baux DHCP Récents" icon={Server}>
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-gray-400 border-b border-gray-700">
                <th className="pb-2">Hostname</th>
                <th className="pb-2">IP</th>
                <th className="pb-2">MAC</th>
                <th className="pb-2">Expiration</th>
              </tr>
            </thead>
            <tbody>
              {data.leases?.slice(0, 10).map(lease => (
                <tr key={lease.mac} className="border-b border-gray-700/50">
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

export default Dashboard;
