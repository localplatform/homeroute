import { useState, useEffect } from 'react';
import { Server, Search, Globe, Network } from 'lucide-react';
import PageHeader from '../components/PageHeader';
import Section from '../components/Section';
import { getDnsConfig, getDhcpLeases } from '../api/client';

function Dns() {
  const [config, setConfig] = useState(null);
  const [leases, setLeases] = useState([]);
  const [search, setSearch] = useState('');
  const [loading, setLoading] = useState(true);
  const [activeTab, setActiveTab] = useState('dhcp');

  useEffect(() => {
    async function fetchData() {
      try {
        const [configRes, leasesRes] = await Promise.all([
          getDnsConfig(),
          getDhcpLeases()
        ]);

        if (configRes.data.success) {
          const raw = configRes.data.config;
          setConfig({
            interface: raw.dhcp?.interface,
            domain: raw.dhcp?.domain,
            dhcpRange: raw.dhcp?.range_start && raw.dhcp?.range_end
              ? `${raw.dhcp.range_start} - ${raw.dhcp.range_end}`
              : null,
            cacheSize: raw.dns?.cache_size,
            dnsServers: raw.dns?.upstream_servers,
            staticRecords: raw.dns?.static_records || [],
            dhcpOptions: raw.dhcp?.static_leases?.map(l => `${l.mac} → ${l.ip} (${l.hostname || ''})`),
            wildcardAddress: raw.dns?.wildcard_ipv4 ? {
              domain: raw.dns.local_domain,
              ip: raw.dns.wildcard_ipv4
            } : null,
            ipv6: {
              raEnabled: raw.ipv6?.ra_enabled,
              dhcpRange: raw.ipv6?.ra_prefix,
              options: raw.ipv6?.dhcpv6_dns_servers?.map(s => `DNS: ${s}`)
            }
          });
        }
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
    lease.mac.toLowerCase().includes(search.toLowerCase()) ||
    lease.ipv6_addresses?.some(addr => addr.includes(search))
  );

  function renderDnsTab() {
    return (
      <div className="space-y-4">
        <div className="grid grid-cols-3 gap-4">
          <Section title="Serveurs DNS upstream" className="!mb-0">
            <div className="space-y-1">
              {config?.dnsServers?.map(server => (
                <div key={server} className="font-mono text-sm bg-gray-900 px-2 py-1">
                  {server}
                </div>
              ))}
            </div>
          </Section>

          <Section title="Cache DNS" className="!mb-0">
            <div className="text-3xl font-bold text-blue-400">
              {config?.cacheSize || '-'}
            </div>
            <p className="text-sm text-gray-400 mt-1">entrées</p>
          </Section>

          <Section title="Wildcard DNS" className="!mb-0">
            {config?.wildcardAddress ? (
              <div className="font-mono text-sm">
                <span className="text-purple-400">*.{config.wildcardAddress.domain}</span>
                <span className="text-gray-400 mx-2">→</span>
                <span className="text-green-400">{config.wildcardAddress.ip}</span>
              </div>
            ) : (
              <span className="text-gray-500 text-sm">Non configuré</span>
            )}
          </Section>
        </div>

        <Section title={`Enregistrements DNS (${config?.staticRecords?.length || 0})`}>
          {config?.staticRecords?.length > 0 ? (
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-left text-gray-400 border-b border-gray-700">
                    <th className="pb-2">Nom</th>
                    <th className="pb-2">Type</th>
                    <th className="pb-2">Valeur</th>
                    <th className="pb-2">TTL</th>
                  </tr>
                </thead>
                <tbody>
                  {config.staticRecords.map((record, i) => (
                    <tr key={i} className="border-b border-gray-700/50 hover:bg-gray-700/30">
                      <td className="py-2 font-mono text-blue-400">{record.name}</td>
                      <td className="py-2">
                        <span className={`px-2 py-0.5 text-xs font-medium ${
                          record.record_type === 'A' ? 'bg-green-900/50 text-green-400' :
                          record.record_type === 'AAAA' ? 'bg-purple-900/50 text-purple-400' :
                          record.record_type === 'CNAME' ? 'bg-yellow-900/50 text-yellow-400' :
                          'bg-gray-700 text-gray-300'
                        }`}>
                          {record.record_type}
                        </span>
                      </td>
                      <td className="py-2 font-mono text-sm">{record.value}</td>
                      <td className="py-2 text-gray-400">{record.ttl}s</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <p className="text-gray-500 text-sm">Aucun enregistrement DNS statique configuré</p>
          )}
        </Section>
      </div>
    );
  }

  function renderDhcpTab() {
    return (
      <div className="space-y-4">
        <div className="grid grid-cols-5 gap-4">
          <Section title="Configuration DHCP" className="!mb-0">
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
            </dl>
          </Section>

          <Section title="Options DHCP" className="!mb-0">
            <div className="space-y-1">
              {config?.dhcpOptions?.length > 0 ? (
                config.dhcpOptions.map((opt, i) => (
                  <div key={i} className="font-mono text-xs bg-gray-900 px-2 py-1">
                    {opt}
                  </div>
                ))
              ) : (
                <span className="text-gray-500 text-sm">-</span>
              )}
            </div>
          </Section>

          <Section title="Configuration IPv6" contrast className="!mb-0 col-span-3">
            <div className="grid grid-cols-3 gap-4">
              <dl className="space-y-2 text-sm">
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
              </dl>
              <div>
                {config?.ipv6?.dhcpRange && (
                  <div className="text-sm">
                    <span className="text-gray-400">Plage DHCPv6</span>
                    <div className="font-mono text-xs text-purple-400 mt-1">{config.ipv6.dhcpRange}</div>
                  </div>
                )}
              </div>
              <div className="space-y-1">
                {config?.ipv6?.options?.map((opt, i) => (
                  <div key={i} className="font-mono text-xs bg-gray-900 px-2 py-1">
                    {opt}
                  </div>
                ))}
              </div>
            </div>
          </Section>
        </div>

        <Section title={`Baux DHCP (${filteredLeases.length})`}>
          <div className="mb-4">
            <div className="relative">
              <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-gray-400" />
              <input
                type="text"
                placeholder="Rechercher..."
                value={search}
                onChange={e => setSearch(e.target.value)}
                className="pl-9 pr-4 py-1.5 bg-gray-900 border border-gray-600 text-sm focus:outline-none focus:border-blue-500"
              />
            </div>
          </div>
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-400 border-b border-gray-700">
                  <th className="pb-2">Hostname</th>
                  <th className="pb-2">Adresse IPv4</th>
                  <th className="pb-2">Adresses IPv6</th>
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
                    <td className="py-2 font-mono text-xs">
                      {lease.ipv6_addresses?.length > 0 ? (
                        <div className="space-y-0.5">
                          {lease.ipv6_addresses.map(addr => (
                            <div key={addr} className="text-purple-400">{addr}</div>
                          ))}
                        </div>
                      ) : (
                        <span className="text-gray-500">-</span>
                      )}
                    </td>
                    <td className="py-2 font-mono text-gray-400 text-xs">{lease.mac}</td>
                    <td className="py-2 text-gray-400 text-xs">
                      {new Date(lease.expiry * 1000).toLocaleString('fr-FR')}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Section>
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

  return (
    <div className="flex flex-col h-full">
      <PageHeader title="DNS / DHCP" icon={Server} />

      <div className="flex flex-1 min-h-0">
        {/* Vertical Tab Sidebar */}
        <div className="w-56 border-r border-gray-700 bg-gray-800/50 flex-shrink-0">
          <button
            onClick={() => setActiveTab('dns')}
            className={`w-full flex items-center gap-2 px-4 py-2.5 text-sm text-left transition-colors ${
              activeTab === 'dns'
                ? 'bg-gray-900 text-blue-400 border-l-2 border-blue-400'
                : 'text-gray-400 hover:bg-gray-800 hover:text-gray-300 border-l-2 border-transparent'
            }`}
          >
            <Globe className="w-4 h-4" />
            DNS
          </button>
          <button
            onClick={() => setActiveTab('dhcp')}
            className={`w-full flex items-center gap-2 px-4 py-2.5 text-sm text-left transition-colors ${
              activeTab === 'dhcp'
                ? 'bg-gray-900 text-blue-400 border-l-2 border-blue-400'
                : 'text-gray-400 hover:bg-gray-800 hover:text-gray-300 border-l-2 border-transparent'
            }`}
          >
            <Network className="w-4 h-4" />
            DHCP
          </button>
        </div>

        {/* Tab Content */}
        <div className="flex-1 overflow-auto p-4">
          {activeTab === 'dns' && renderDnsTab()}
          {activeTab === 'dhcp' && renderDhcpTab()}
        </div>
      </div>
    </div>
  );
}

export default Dns;
