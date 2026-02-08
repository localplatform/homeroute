import { useState, useEffect, useCallback } from 'react';
import {
  Database, ChevronDown, ChevronRight, Table2,
  Link2, HardDrive, Clock, Loader2
} from 'lucide-react';
import Card from '../components/Card';
import PageHeader from '../components/PageHeader';
import useWebSocket from '../hooks/useWebSocket';
import { getDataverseOverview } from '../api/client';

// Field type color mapping
const FIELD_TYPE_COLORS = {
  text: 'bg-blue-900/30 text-blue-400',
  number: 'bg-green-900/30 text-green-400',
  decimal: 'bg-green-900/30 text-green-400',
  boolean: 'bg-yellow-900/30 text-yellow-400',
  date_time: 'bg-purple-900/30 text-purple-400',
  date: 'bg-purple-900/30 text-purple-400',
  time: 'bg-purple-900/30 text-purple-400',
  email: 'bg-cyan-900/30 text-cyan-400',
  url: 'bg-cyan-900/30 text-cyan-400',
  uuid: 'bg-gray-700/50 text-gray-300',
  json: 'bg-orange-900/30 text-orange-400',
  lookup: 'bg-pink-900/30 text-pink-400',
  auto_increment: 'bg-gray-700/50 text-gray-400',
};

function FieldTypeBadge({ type: fieldType }) {
  const color = FIELD_TYPE_COLORS[fieldType] || 'bg-gray-700/50 text-gray-400';
  return (
    <span className={`text-xs px-1.5 py-0.5 rounded ${color}`}>
      {fieldType}
    </span>
  );
}

function Dataverse() {
  const [overview, setOverview] = useState(null);
  const [loading, setLoading] = useState(true);
  const [expandedApps, setExpandedApps] = useState({});
  const [expandedTables, setExpandedTables] = useState({});

  const fetchOverview = useCallback(async () => {
    try {
      const res = await getDataverseOverview();
      setOverview(res.data);
    } catch (err) {
      console.error('Failed to fetch dataverse overview:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchOverview();
  }, [fetchOverview]);

  // Live WebSocket updates
  useWebSocket({
    'dataverse:schema': (data) => {
      setOverview(prev => {
        if (!prev) return prev;
        const apps = [...(prev.apps || [])];
        const idx = apps.findIndex(a => a.appId === data.appId);
        const updated = {
          appId: data.appId,
          slug: data.slug || (idx >= 0 ? apps[idx].slug : ''),
          tables: data.tables || [],
          relationsCount: data.relationsCount || 0,
          version: data.version || 0,
          lastUpdated: new Date().toISOString(),
        };
        if (idx >= 0) {
          apps[idx] = { ...apps[idx], ...updated };
        } else {
          apps.push(updated);
        }
        return { ...prev, apps };
      });
    },
  });

  const toggleApp = (appId) => {
    setExpandedApps(prev => ({ ...prev, [appId]: !prev[appId] }));
  };

  const toggleTable = (key) => {
    setExpandedTables(prev => ({ ...prev, [key]: !prev[key] }));
  };

  if (loading) {
    return (
      <div>
        <PageHeader icon={Database} title="Dataverse" />
        <div className="flex items-center justify-center py-20">
          <Loader2 className="w-8 h-8 text-blue-400 animate-spin" />
        </div>
      </div>
    );
  }

  const apps = overview?.apps || [];

  return (
    <div>
      <PageHeader icon={Database} title="Dataverse" />

      {apps.length === 0 ? (
        <Card>
          <div className="text-center py-12">
            <Database className="w-12 h-12 text-gray-600 mx-auto mb-3" />
            <p className="text-gray-400">Aucune application n'a encore de base de donnees Dataverse.</p>
            <p className="text-gray-500 text-sm mt-1">Les schemas apparaitront ici automatiquement.</p>
          </div>
        </Card>
      ) : (
        <div className="space-y-4">
          {apps.map(app => (
            <Card key={app.appId}>
              <div
                className="flex items-center justify-between cursor-pointer"
                onClick={() => toggleApp(app.appId)}
              >
                <div className="flex items-center gap-3">
                  {expandedApps[app.appId] ? (
                    <ChevronDown className="w-5 h-5 text-gray-400" />
                  ) : (
                    <ChevronRight className="w-5 h-5 text-gray-400" />
                  )}
                  <Database className="w-5 h-5 text-blue-400" />
                  <div>
                    <h3 className="text-white font-medium">{app.slug}</h3>
                    <div className="flex items-center gap-4 text-xs text-gray-400 mt-0.5">
                      <span className="flex items-center gap-1">
                        <Table2 className="w-3 h-3" />
                        {app.tables?.length || 0} tables
                      </span>
                      <span className="flex items-center gap-1">
                        <Link2 className="w-3 h-3" />
                        {app.relationsCount || 0} relations
                      </span>
                      {app.dbSizeBytes && (
                        <span className="flex items-center gap-1">
                          <HardDrive className="w-3 h-3" />
                          {formatBytes(app.dbSizeBytes)}
                        </span>
                      )}
                    </div>
                  </div>
                </div>
                <div className="flex items-center gap-2 text-xs text-gray-500">
                  <Clock className="w-3 h-3" />
                  v{app.version || 0}
                </div>
              </div>

              {expandedApps[app.appId] && (
                <div className="mt-4 border-t border-gray-700 pt-4 space-y-2">
                  {(app.tables || []).length === 0 ? (
                    <p className="text-gray-500 text-sm">Aucune table.</p>
                  ) : (
                    (app.tables || []).map(table => {
                      const tableKey = `${app.appId}:${table.name}`;
                      return (
                        <div key={tableKey} className="bg-gray-800/50 rounded">
                          <div
                            className="flex items-center justify-between p-3 cursor-pointer hover:bg-gray-700/30"
                            onClick={() => toggleTable(tableKey)}
                          >
                            <div className="flex items-center gap-2">
                              {expandedTables[tableKey] ? (
                                <ChevronDown className="w-4 h-4 text-gray-500" />
                              ) : (
                                <ChevronRight className="w-4 h-4 text-gray-500" />
                              )}
                              <Table2 className="w-4 h-4 text-green-400" />
                              <span className="text-gray-200 text-sm font-mono">{table.name}</span>
                            </div>
                            <div className="flex items-center gap-3 text-xs text-gray-500">
                              <span>{table.columnsCount || table.columns?.length || 0} colonnes</span>
                              <span>{table.rowsCount || 0} lignes</span>
                            </div>
                          </div>

                          {expandedTables[tableKey] && table.columns && (
                            <div className="px-3 pb-3">
                              <table className="w-full text-sm">
                                <thead>
                                  <tr className="text-xs text-gray-500 border-b border-gray-700">
                                    <th className="text-left py-1 font-normal">Colonne</th>
                                    <th className="text-left py-1 font-normal">Type</th>
                                    <th className="text-center py-1 font-normal">Requis</th>
                                    <th className="text-center py-1 font-normal">Unique</th>
                                  </tr>
                                </thead>
                                <tbody>
                                  {table.columns.map(col => (
                                    <tr key={col.name} className="border-b border-gray-800/50">
                                      <td className="py-1.5 text-gray-300 font-mono text-xs">{col.name}</td>
                                      <td className="py-1.5"><FieldTypeBadge type={col.fieldType} /></td>
                                      <td className="py-1.5 text-center">
                                        {col.required && <span className="text-yellow-400 text-xs">*</span>}
                                      </td>
                                      <td className="py-1.5 text-center">
                                        {col.unique && <span className="text-blue-400 text-xs">U</span>}
                                      </td>
                                    </tr>
                                  ))}
                                </tbody>
                              </table>
                            </div>
                          )}
                        </div>
                      );
                    })
                  )}
                </div>
              )}
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}

function formatBytes(bytes) {
  if (!bytes) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
}

export default Dataverse;
