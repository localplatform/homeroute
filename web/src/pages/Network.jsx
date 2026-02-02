import { useState, useEffect } from 'react';
import {
  Network as NetworkIcon,
  Route,
  ArrowRight,
  Shield,
  RefreshCw,
  ArrowLeftRight,
  Activity
} from 'lucide-react';
import Card from '../components/Card';
import StatusBadge from '../components/StatusBadge';
import Button from '../components/Button';
import PageHeader from '../components/PageHeader';
import {
  getInterfaces,
  getRoutes,
  getMasqueradeRules,
  getPortForwards,
  getFirewallStatus,
  getFilterRules,
  getRoutingRules,
  getChainStats
} from '../api/client';

function formatBytes(bytes) {
  if (!bytes || bytes === '0') return '0 B';
  const num = parseInt(bytes);
  if (num < 1024) return num + ' B';
  if (num < 1024 * 1024) return (num / 1024).toFixed(1) + ' KB';
  if (num < 1024 * 1024 * 1024) return (num / (1024 * 1024)).toFixed(1) + ' MB';
  return (num / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
}

function formatPackets(pkts) {
  if (!pkts || pkts === '0') return '0';
  const num = parseInt(pkts);
  if (num < 1000) return num.toString();
  if (num < 1000000) return (num / 1000).toFixed(1) + 'K';
  return (num / 1000000).toFixed(2) + 'M';
}

function Network() {
  const [interfaces, setInterfaces] = useState([]);
  const [routes, setRoutes] = useState([]);
  const [masquerade, setMasquerade] = useState([]);
  const [forwards, setForwards] = useState([]);
  const [firewallStatus, setFirewallStatus] = useState(null);
  const [filterRules, setFilterRules] = useState({});
  const [routingRules, setRoutingRules] = useState([]);
  const [chainStats, setChainStats] = useState(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [activeTab, setActiveTab] = useState('interfaces');

  async function fetchData() {
    try {
      const [ifRes, routeRes, masqRes, fwdRes, statusRes, filterRes, routingRes, statsRes] = await Promise.all([
        getInterfaces(),
        getRoutes(),
        getMasqueradeRules(),
        getPortForwards(),
        getFirewallStatus(),
        getFilterRules(),
        getRoutingRules(),
        getChainStats()
      ]);

      if (ifRes.data.success) setInterfaces(ifRes.data.interfaces);
      if (routeRes.data.success) setRoutes(routeRes.data.routes);
      if (masqRes.data.success) setMasquerade(masqRes.data.rules);
      if (fwdRes.data.success) setForwards(fwdRes.data.rules);
      if (statusRes.data.success) setFirewallStatus(statusRes.data.status);
      if (filterRes.data.success) setFilterRules(filterRes.data.rules);
      if (routingRes.data.success) setRoutingRules(routingRes.data.rules);
      if (statsRes.data.success) setChainStats(statsRes.data.stats);
    } catch (error) {
      console.error('Error:', error);
    }
  }

  useEffect(() => {
    async function init() {
      await fetchData();
      setLoading(false);
    }
    init();
  }, []);

  async function handleRefresh() {
    setRefreshing(true);
    await fetchData();
    setRefreshing(false);
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  // Separate physical from virtual interfaces
  const physicalIfaces = interfaces.filter(i =>
    i.name?.startsWith('en') || i.name?.startsWith('eth')
  );
  const bridgeIfaces = interfaces.filter(i =>
    i.name?.startsWith('br-') || i.name?.startsWith('virbr') || i.name?.startsWith('lxc') || i.name === 'docker0'
  );
  const vpnIfaces = interfaces.filter(i =>
    i.name?.startsWith('tailscale') || i.name?.startsWith('wg') || i.name?.startsWith('tun')
  );

  const tabs = [
    { id: 'interfaces', label: 'Interfaces', icon: NetworkIcon },
    { id: 'routing', label: 'Routage', icon: Route },
    { id: 'firewall', label: 'Firewall', icon: Shield },
    { id: 'nat', label: 'NAT', icon: ArrowLeftRight }
  ];

  // Main firewall chains to display
  const mainChains = ['INPUT', 'FORWARD', 'OUTPUT'];

  return (
    <div>
      <PageHeader title="Réseau / Firewall" icon={NetworkIcon}>
        {firewallStatus && (
          <div className="flex items-center gap-2 text-sm">
            <StatusBadge status={firewallStatus.active ? 'up' : 'down'}>
              {firewallStatus.active ? 'Actif' : 'Inactif'}
            </StatusBadge>
            <span className="text-gray-400">{firewallStatus.framework}</span>
          </div>
        )}
        <Button onClick={handleRefresh} disabled={refreshing}>
          <RefreshCw className={`w-4 h-4 mr-2 ${refreshing ? 'animate-spin' : ''}`} />
          Actualiser
        </Button>
      </PageHeader>

      {/* Tabs */}
      <div className="flex border-b border-gray-700">
        {tabs.map(tab => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            className={`flex items-center gap-2 px-4 py-2 text-sm font-medium border-b-2 -mb-px transition-colors ${
              activeTab === tab.id
                ? 'border-blue-400 text-blue-400'
                : 'border-transparent text-gray-400 hover:text-gray-300'
            }`}
          >
            <tab.icon className="w-4 h-4" />
            {tab.label}
          </button>
        ))}
      </div>

      {/* Tab: Interfaces */}
      {activeTab === 'interfaces' && (
        <div>
          {/* Physical Interfaces */}
          <Card title="Interfaces Physiques" icon={NetworkIcon}>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              {physicalIfaces.map(iface => (
                <div key={iface.name} className="bg-gray-900 p-4">
                  <div className="flex items-center justify-between mb-3">
                    <span className="font-mono font-bold">{iface.name}</span>
                    <StatusBadge status={iface.state === 'UP' ? 'up' : 'down'}>
                      {iface.state}
                    </StatusBadge>
                  </div>
                  <div className="space-y-2 text-sm">
                    <div className="flex justify-between text-gray-400">
                      <span>MAC</span>
                      <span className="font-mono text-xs">{iface.mac}</span>
                    </div>
                    <div className="flex justify-between text-gray-400">
                      <span>MTU</span>
                      <span className="font-mono">{iface.mtu}</span>
                    </div>
                    {iface.addresses?.filter(a => a.family === 'inet').map((addr, i) => (
                      <div key={i} className="flex justify-between">
                        <span className="text-gray-400">IPv4</span>
                        <span className="font-mono text-blue-400">{addr.address}/{addr.prefixlen}</span>
                      </div>
                    ))}
                    {iface.addresses?.filter(a => a.family === 'inet6' && a.scope === 'global').map((addr, i) => (
                      <div key={i} className="flex justify-between">
                        <span className="text-gray-400">IPv6</span>
                        <span className="font-mono text-purple-400 text-xs">{addr.address}</span>
                      </div>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </Card>

          {/* VPN Interfaces */}
          {vpnIfaces.length > 0 && (
            <Card title="Interfaces VPN" icon={Shield}>
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                {vpnIfaces.map(iface => (
                  <div key={iface.name} className="bg-gray-900 p-4">
                    <div className="flex items-center justify-between mb-3">
                      <span className="font-mono font-bold text-green-400">{iface.name}</span>
                      <StatusBadge status={iface.state === 'UP' ? 'up' : 'down'}>
                        {iface.state}
                      </StatusBadge>
                    </div>
                    <div className="space-y-2 text-sm">
                      {iface.addresses?.filter(a => a.family === 'inet').map((addr, i) => (
                        <div key={i} className="flex justify-between">
                          <span className="text-gray-400">IPv4</span>
                          <span className="font-mono text-blue-400">{addr.address}/{addr.prefixlen}</span>
                        </div>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            </Card>
          )}

          {/* Bridges */}
          <Card title="Bridges & Containers" icon={NetworkIcon}>
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-gray-400 border-b border-gray-700">
                    <th className="pb-2">Interface</th>
                    <th className="pb-2">Etat</th>
                    <th className="pb-2">IPv4</th>
                    <th className="pb-2">MTU</th>
                  </tr>
                </thead>
                <tbody>
                  {bridgeIfaces.map(iface => (
                    <tr key={iface.name} className="border-b border-gray-700/50">
                      <td className="py-2 font-mono">{iface.name}</td>
                      <td className="py-2">
                        <StatusBadge status={iface.state === 'UP' ? 'up' : 'down'}>
                          {iface.state}
                        </StatusBadge>
                      </td>
                      <td className="py-2 font-mono text-blue-400">
                        {iface.addresses?.find(a => a.family === 'inet')?.address || '-'}
                      </td>
                      <td className="py-2 text-gray-400">{iface.mtu}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </Card>
        </div>
      )}

      {/* Tab: Routing */}
      {activeTab === 'routing' && (
        <div>
          {/* Routing Table */}
          <Card title="Table de Routage IPv4" icon={Route}>
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-gray-400 border-b border-gray-700">
                    <th className="pb-2">Destination</th>
                    <th className="pb-2">Passerelle</th>
                    <th className="pb-2">Interface</th>
                    <th className="pb-2">Metrique</th>
                  </tr>
                </thead>
                <tbody>
                  {routes.map((route, i) => (
                    <tr key={i} className="border-b border-gray-700/50">
                      <td className="py-2 font-mono text-xs">
                        {route.destination === 'default' ? (
                          <span className="text-yellow-400 font-bold">default</span>
                        ) : route.destination}
                      </td>
                      <td className="py-2 font-mono text-xs text-blue-400">
                        {route.gateway || '-'}
                      </td>
                      <td className="py-2 font-mono text-gray-400">{route.device}</td>
                      <td className="py-2 text-gray-500">{route.metric || '-'}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </Card>

          {/* Policy Routing */}
          {routingRules.length > 0 && (
            <Card title="Policy Routing (ip rule)" icon={Activity}>
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="text-left text-gray-400 border-b border-gray-700">
                      <th className="pb-2">Priorite</th>
                      <th className="pb-2">Source</th>
                      <th className="pb-2">Destination</th>
                      <th className="pb-2">Table</th>
                      <th className="pb-2">Mark</th>
                    </tr>
                  </thead>
                  <tbody>
                    {routingRules.map((rule, i) => (
                      <tr key={i} className="border-b border-gray-700/50">
                        <td className="py-2 font-mono text-yellow-400">{rule.priority}</td>
                        <td className="py-2 font-mono text-xs">{rule.src}</td>
                        <td className="py-2 font-mono text-xs">{rule.dst}</td>
                        <td className="py-2 font-mono text-blue-400">{rule.table}</td>
                        <td className="py-2 font-mono text-xs text-gray-500">
                          {rule.fwmark || '-'}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </Card>
          )}
        </div>
      )}

      {/* Tab: Firewall */}
      {activeTab === 'firewall' && (
        <div>
          {/* Chain Stats Overview */}
          {chainStats && (
            <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
              {mainChains.map(chainName => {
                const chain = filterRules[chainName];
                const stats = chainStats.chains?.[chainName];
                if (!chain) return null;
                return (
                  <div key={chainName} className="bg-gray-800 p-4 border border-gray-700">
                    <div className="flex items-center justify-between mb-2">
                      <span className="font-bold">{chainName}</span>
                      <span className={`text-xs px-2 py-1 ${
                        chain.policy === 'DROP' ? 'bg-red-900/50 text-red-400' :
                        chain.policy === 'ACCEPT' ? 'bg-green-900/50 text-green-400' :
                        'bg-gray-700 text-gray-400'
                      }`}>
                        {chain.policy}
                      </span>
                    </div>
                    <div className="text-sm text-gray-400">
                      <div className="flex justify-between">
                        <span>Regles</span>
                        <span className="text-white">{chain.rules?.length || 0}</span>
                      </div>
                      {stats && (
                        <>
                          <div className="flex justify-between">
                            <span>Paquets</span>
                            <span className="text-white">{formatPackets(stats.packets)}</span>
                          </div>
                          <div className="flex justify-between">
                            <span>Octets</span>
                            <span className="text-white">{formatBytes(stats.bytes)}</span>
                          </div>
                        </>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}

          {/* Filter Rules by Chain */}
          {mainChains.map(chainName => {
            const chain = filterRules[chainName];
            if (!chain || !chain.rules?.length) return null;
            return (
              <Card
                key={chainName}
                title={`Chaine ${chainName}`}
                icon={Shield}
              >
                <div className="overflow-x-auto">
                  <table className="w-full text-xs">
                    <thead>
                      <tr className="text-left text-gray-400 border-b border-gray-700">
                        <th className="pb-2 pr-2">#</th>
                        <th className="pb-2 pr-2">Target</th>
                        <th className="pb-2 pr-2">Proto</th>
                        <th className="pb-2 pr-2">In</th>
                        <th className="pb-2 pr-2">Out</th>
                        <th className="pb-2 pr-2">Source</th>
                        <th className="pb-2 pr-2">Dest</th>
                        <th className="pb-2 pr-2">Extra</th>
                        <th className="pb-2 text-right">Pkts</th>
                      </tr>
                    </thead>
                    <tbody>
                      {chain.rules.map((rule, i) => (
                        <tr key={i} className="border-b border-gray-700/50 hover:bg-gray-700/30">
                          <td className="py-2 pr-2 text-gray-500">{rule.num}</td>
                          <td className={`py-2 pr-2 font-mono ${
                            rule.target === 'ACCEPT' ? 'text-green-400' :
                            rule.target === 'DROP' ? 'text-red-400' :
                            rule.target === 'REJECT' ? 'text-red-400' :
                            'text-yellow-400'
                          }`}>
                            {rule.target}
                          </td>
                          <td className="py-2 pr-2 text-gray-400">{rule.prot}</td>
                          <td className="py-2 pr-2 font-mono text-gray-400">
                            {rule.in !== '*' ? rule.in : '-'}
                          </td>
                          <td className="py-2 pr-2 font-mono text-gray-400">
                            {rule.out !== '*' ? rule.out : '-'}
                          </td>
                          <td className="py-2 pr-2 font-mono">{rule.source}</td>
                          <td className="py-2 pr-2 font-mono">{rule.destination}</td>
                          <td className="py-2 pr-2 text-gray-500 max-w-xs truncate" title={rule.extra}>
                            {rule.extra || '-'}
                          </td>
                          <td className="py-2 text-right text-gray-400">{formatPackets(rule.pkts)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </Card>
            );
          })}

          {/* Other chains (Docker, Tailscale, etc.) */}
          {Object.entries(filterRules)
            .filter(([name]) => !mainChains.includes(name) && filterRules[name]?.rules?.length > 0)
            .slice(0, 5)
            .map(([chainName, chain]) => (
              <Card
                key={chainName}
                title={`Chaine ${chainName}`}
                icon={Shield}
              >
                <div className="text-sm text-gray-400 mb-2">
                  {chain.rules?.length || 0} regles
                </div>
                <div className="overflow-x-auto max-h-48 overflow-y-auto">
                  <table className="w-full text-xs">
                    <thead>
                      <tr className="text-left text-gray-400 border-b border-gray-700">
                        <th className="pb-2 pr-2">#</th>
                        <th className="pb-2 pr-2">Target</th>
                        <th className="pb-2 pr-2">Proto</th>
                        <th className="pb-2 pr-2">Source</th>
                        <th className="pb-2">Extra</th>
                      </tr>
                    </thead>
                    <tbody>
                      {chain.rules.slice(0, 10).map((rule, i) => (
                        <tr key={i} className="border-b border-gray-700/50">
                          <td className="py-1 pr-2 text-gray-500">{rule.num}</td>
                          <td className={`py-1 pr-2 font-mono ${
                            rule.target === 'ACCEPT' ? 'text-green-400' :
                            rule.target === 'DROP' ? 'text-red-400' :
                            'text-yellow-400'
                          }`}>
                            {rule.target}
                          </td>
                          <td className="py-1 pr-2 text-gray-400">{rule.prot}</td>
                          <td className="py-1 pr-2 font-mono">{rule.source}</td>
                          <td className="py-1 text-gray-500 truncate max-w-xs">{rule.extra || '-'}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              </Card>
            ))}
        </div>
      )}

      {/* Tab: NAT */}
      {activeTab === 'nat' && (
        <div>
          {/* Port Forwards */}
          <Card title="Port Forwards (DNAT)" icon={ArrowRight}>
            {forwards.length > 0 ? (
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="text-left text-gray-400 border-b border-gray-700">
                      <th className="pb-2">Proto</th>
                      <th className="pb-2">Port Ext.</th>
                      <th className="pb-2">Destination</th>
                      <th className="pb-2">Interface</th>
                      <th className="pb-2 text-right">Paquets</th>
                    </tr>
                  </thead>
                  <tbody>
                    {forwards.map((rule, i) => (
                      <tr key={i} className="border-b border-gray-700/50">
                        <td className="py-2 font-mono text-gray-400">{rule.protocol}</td>
                        <td className="py-2 font-mono text-yellow-400">{rule.destinationPort}</td>
                        <td className="py-2 font-mono text-blue-400">{rule.forwardTo}</td>
                        <td className="py-2 text-gray-400">{rule.inInterface || 'any'}</td>
                        <td className="py-2 text-right text-gray-500">{formatPackets(rule.pkts)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <p className="text-gray-500">Aucun port forward configure</p>
            )}
          </Card>

          {/* Masquerade */}
          <Card title="Masquerade (SNAT)" icon={ArrowLeftRight}>
            {masquerade.length > 0 ? (
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="text-left text-gray-400 border-b border-gray-700">
                      <th className="pb-2">Source</th>
                      <th className="pb-2">Destination</th>
                      <th className="pb-2">Interface Sortie</th>
                      <th className="pb-2 text-right">Paquets</th>
                      <th className="pb-2 text-right">Octets</th>
                    </tr>
                  </thead>
                  <tbody>
                    {masquerade.map((rule, i) => (
                      <tr key={i} className="border-b border-gray-700/50">
                        <td className="py-2 font-mono">{rule.source}</td>
                        <td className="py-2 font-mono text-gray-400">{rule.destination}</td>
                        <td className="py-2 text-blue-400">{rule.outInterface || 'any'}</td>
                        <td className="py-2 text-right text-gray-500">{formatPackets(rule.pkts)}</td>
                        <td className="py-2 text-right text-gray-500">{formatBytes(rule.bytes)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <p className="text-gray-500">Aucune regle masquerade</p>
            )}
          </Card>
        </div>
      )}
    </div>
  );
}

export default Network;
