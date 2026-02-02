import React, { useState, useEffect } from 'react';
import Card from '../components/Card';
import PageHeader from '../components/PageHeader';
import Section from '../components/Section';
import { BarChart3, Activity, HardDrive, Users } from 'lucide-react';
import {
  getTrafficOverview,
  getTrafficTimeseries,
  getTopDevices,
  getTopEndpoints,
  getApplicationBreakdown,
  getTopDomains,
  getDnsByCategory
} from '../api/client';
import TrafficTimeseriesChart from '../components/charts/TrafficTimeseriesChart';
import TopDevicesChart from '../components/charts/TopDevicesChart';
import TopEndpointsChart from '../components/charts/TopEndpointsChart';
import ApplicationPieChart from '../components/charts/ApplicationPieChart';

export default function Traffic() {
  const [timeRange, setTimeRange] = useState('24h');
  const [overview, setOverview] = useState(null);
  const [timeseriesData, setTimeseriesData] = useState([]);
  const [topDevices, setTopDevices] = useState([]);
  const [topEndpoints, setTopEndpoints] = useState([]);
  const [appBreakdown, setAppBreakdown] = useState([]);
  const [topDomains, setTopDomains] = useState([]);
  const [dnsCategories, setDnsCategories] = useState([]);
  const [liveStats, setLiveStats] = useState({ rps: 0, bandwidthMbps: 0 });
  const [loading, setLoading] = useState(true);

  // Fetch data
  useEffect(() => {
    fetchData();
    const interval = setInterval(fetchData, 30000); // Refresh every 30s
    return () => clearInterval(interval);
  }, [timeRange]);

  // SSE for real-time updates
  useEffect(() => {
    const eventSource = new EventSource('/api/traffic/events');

    eventSource.addEventListener('trafficUpdate', (e) => {
      const data = JSON.parse(e.data);
      setLiveStats(prev => ({
        ...prev,
        rps: data.rps || prev.rps,
        bandwidthMbps: data.bandwidthMbps || prev.bandwidthMbps
      }));
    });

    eventSource.addEventListener('networkUpdate', (e) => {
      const data = JSON.parse(e.data);
      setLiveStats(prev => ({
        ...prev,
        networkBandwidthMbps: data.bandwidthMbps || 0
      }));
    });

    return () => eventSource.close();
  }, []);

  async function fetchData() {
    try {
      const [overviewRes, timeseriesRes, devicesRes, endpointsRes, appsRes, domainsRes, categoriesRes] = await Promise.all([
        getTrafficOverview(timeRange),
        getTrafficTimeseries({ metric: 'requests', granularity: 'hour', timeRange }),
        getTopDevices(timeRange),
        getTopEndpoints(timeRange),
        getApplicationBreakdown(timeRange),
        getTopDomains(timeRange, 10),
        getDnsByCategory(timeRange)
      ]);

      setOverview(overviewRes.data || null);
      setTimeseriesData(timeseriesRes.data || []);
      setTopDevices(devicesRes.data || []);
      setTopEndpoints(endpointsRes.data || []);
      setAppBreakdown(appsRes.data || []);
      setTopDomains(domainsRes.data || []);
      setDnsCategories(categoriesRes.data || []);
      setLoading(false);
    } catch (error) {
      console.error('Error fetching traffic data:', error);
      setLoading(false);
    }
  }

  return (
    <div>
      <PageHeader title="Analyse du Trafic" icon={BarChart3}>
        <select
          value={timeRange}
          onChange={(e) => setTimeRange(e.target.value)}
          className="bg-gray-800 border border-gray-700 text-gray-100 px-4 py-2"
        >
          <option value="1h">Dernière heure</option>
          <option value="24h">24 heures</option>
          <option value="7d">7 jours</option>
          <option value="30d">30 jours</option>
        </select>
      </PageHeader>

      {loading ? (
        <div className="text-gray-400">Chargement...</div>
      ) : (
        <>
          {/* Overview Cards */}
          <Section title="Vue d'ensemble">
          <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-4 gap-4">
            <Card title="Requêtes" icon={Activity}>
              <div className="text-3xl font-bold text-gray-100">
                {overview?.totalRequests?.toLocaleString() || '0'}
              </div>
              <div className="text-sm text-gray-400 mt-1">
                {liveStats.rps} RPS actuellement
              </div>
            </Card>

            <Card title="Bande passante" icon={BarChart3}>
              <div className="text-3xl font-bold text-gray-100">
                {formatBytes(overview?.totalBytes)}
              </div>
              <div className="text-sm text-gray-400 mt-1">
                {liveStats.bandwidthMbps?.toFixed(2) || '0.00'} Mbps actuellement
              </div>
            </Card>

            <Card title="Périphériques" icon={Users}>
              <div className="text-3xl font-bold text-gray-100">
                {overview?.uniqueDevices || '0'}
              </div>
              <div className="text-sm text-gray-400 mt-1">actifs</div>
            </Card>

            <Card title="Endpoints" icon={HardDrive}>
              <div className="text-3xl font-bold text-gray-100">
                {overview?.uniqueEndpoints || '0'}
              </div>
              <div className="text-sm text-gray-400 mt-1">différents</div>
            </Card>
          </div>
          </Section>

          {/* Timeseries Chart */}
          <Section title="Requêtes au fil du temps" contrast>
          <Card title="Requêtes au fil du temps">
            {timeseriesData.length > 0 ? (
              <TrafficTimeseriesChart data={timeseriesData} metric="requests" />
            ) : (
              <div className="text-gray-400 text-center py-12">
                Aucune donnée disponible pour cette période
              </div>
            )}
          </Card>
          </Section>

          {/* Top Devices & Endpoints */}
          <Section title="Top Périphériques / Endpoints">
          <div className="grid grid-cols-1 xl:grid-cols-2 gap-4">
            <Card title="Top Périphériques">
              {topDevices.length > 0 ? (
                <TopDevicesChart data={topDevices} />
              ) : (
                <div className="text-gray-400 text-center py-12">
                  Aucune donnée disponible
                </div>
              )}
            </Card>

            <Card title="Top Endpoints">
              {topEndpoints.length > 0 ? (
                <TopEndpointsChart data={topEndpoints} />
              ) : (
                <div className="text-gray-400 text-center py-12">
                  Aucune donnée disponible
                </div>
              )}
            </Card>
          </div>
          </Section>

          <Section title="Trafic par Application" contrast>
          <Card title="Trafic par Application">
            {appBreakdown.length > 0 ? (
              <ApplicationPieChart data={appBreakdown} />
            ) : (
              <div className="text-gray-400 text-center py-12">
                Aucune donnée disponible
              </div>
            )}
          </Card>
          </Section>

          <Section title="DNS Analytics">
          <div className="grid grid-cols-1 xl:grid-cols-2 gap-4">
            <Card title="Top Domaines LAN/WAN">
              {topDomains.length > 0 ? (
                <div className="space-y-2">
                  {topDomains.map((domain, idx) => (
                    <div key={idx} className="flex justify-between items-center py-2 border-b border-gray-700 last:border-0">
                      <div className="flex-1">
                        <div className="text-gray-100 font-medium">{domain.domain}</div>
                        <div className="text-xs text-gray-400">{domain.category || 'Other'}</div>
                      </div>
                      <div className="text-right">
                        <div className="text-gray-100">{domain.totalQueries?.toLocaleString() || 0} requêtes</div>
                        <div className="text-xs text-gray-400">{domain.uniqueDevices || 0} devices</div>
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="text-gray-400 text-center py-12">
                  Aucune donnée DNS disponible
                </div>
              )}
            </Card>

            <Card title="Trafic DNS par Catégorie">
              {dnsCategories.length > 0 ? (
                <div className="space-y-2">
                  {dnsCategories.map((cat, idx) => (
                    <div key={idx} className="flex justify-between items-center py-2 border-b border-gray-700 last:border-0">
                      <div className="text-gray-100 font-medium">{cat.category || 'Other'}</div>
                      <div className="text-gray-100">{cat.totalQueries?.toLocaleString() || 0} requêtes</div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="text-gray-400 text-center py-12">
                  Aucune donnée disponible
                </div>
              )}
            </Card>
          </div>
          </Section>
        </>
      )}
    </div>
  );
}

function formatBytes(bytes) {
  if (!bytes) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(2)} ${sizes[i]}`;
}
