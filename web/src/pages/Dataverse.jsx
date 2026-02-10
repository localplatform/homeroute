import { useState, useEffect, useCallback } from 'react';
import {
  Database, ChevronRight, Table2, Loader2
} from 'lucide-react';
import PageHeader from '../components/PageHeader';
import { getDataverseOverview, getDataverseTables, getDataverseTable } from '../api/client';

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

function ColumnHeader({ children }) {
  return (
    <div className="px-3 py-2 border-b border-gray-700 bg-gray-800/60 flex-shrink-0">
      <h3 className="text-xs font-semibold text-gray-400 uppercase tracking-wider">
        {children}
      </h3>
    </div>
  );
}

function EmptyState({ icon: Icon, text }) {
  return (
    <div className="flex-1 flex items-center justify-center text-center px-4">
      <div>
        {Icon && <Icon className="w-8 h-8 text-gray-600 mx-auto mb-2" />}
        <p className="text-sm text-gray-500">{text}</p>
      </div>
    </div>
  );
}

function Spinner() {
  return (
    <div className="flex-1 flex items-center justify-center">
      <Loader2 className="w-5 h-5 text-gray-500 animate-spin" />
    </div>
  );
}

function Dataverse() {
  const [apps, setApps] = useState([]);
  const [loading, setLoading] = useState(true);

  // Column 2: tables fetched per click
  const [selectedApp, setSelectedApp] = useState(null);
  const [tables, setTables] = useState([]);
  const [loadingTables, setLoadingTables] = useState(false);

  // Column 3: columns fetched per click
  const [selectedTableName, setSelectedTableName] = useState(null);
  const [tableDetail, setTableDetail] = useState(null);
  const [loadingColumns, setLoadingColumns] = useState(false);

  const fetchOverview = useCallback(async () => {
    try {
      const res = await getDataverseOverview();
      setApps(res.data?.apps || []);
    } catch (err) {
      console.error('Failed to fetch dataverse overview:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchOverview(); }, [fetchOverview]);

  // Auto-select first app
  useEffect(() => {
    if (apps.length > 0 && !selectedApp) {
      selectApp(apps[0]);
    }
  }, [apps]);

  async function selectApp(app) {
    setSelectedApp(app);
    setSelectedTableName(null);
    setTableDetail(null);
    setLoadingTables(true);
    try {
      const res = await getDataverseTables(app.appId);
      setTables(res.data?.tables || []);
    } catch {
      setTables([]);
    } finally {
      setLoadingTables(false);
    }
  }

  async function selectTable(name) {
    setSelectedTableName(name);
    setLoadingColumns(true);
    try {
      const res = await getDataverseTable(selectedApp.appId, name);
      setTableDetail(res.data?.table || null);
    } catch {
      setTableDetail(null);
    } finally {
      setLoadingColumns(false);
    }
  }

  const columns = tableDetail?.columns || [];

  if (loading) {
    return (
      <div className="h-full flex flex-col">
        <PageHeader icon={Database} title="Dataverse" />
        <div className="flex-1 flex items-center justify-center">
          <Loader2 className="w-8 h-8 text-blue-400 animate-spin" />
        </div>
      </div>
    );
  }

  if (apps.length === 0) {
    return (
      <div className="h-full flex flex-col">
        <PageHeader icon={Database} title="Dataverse" />
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center">
            <Database className="w-12 h-12 text-gray-600 mx-auto mb-3" />
            <p className="text-gray-400">Aucune application n'a encore de base de donnees Dataverse.</p>
            <p className="text-gray-500 text-sm mt-1">Les schemas apparaitront ici automatiquement.</p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      <PageHeader icon={Database} title="Dataverse" />

      <div className="flex-1 min-h-0 flex">

        {/* Column 1: Apps */}
        <div className="w-64 flex-shrink-0 border-r border-gray-700 flex flex-col bg-gray-800">
          <ColumnHeader>Applications ({apps.length})</ColumnHeader>
          <div className="flex-1 overflow-y-auto">
            {apps.map(app => {
              const isSelected = selectedApp?.appId === app.appId;
              return (
                <div
                  key={app.appId}
                  onClick={() => selectApp(app)}
                  className={`flex items-center justify-between px-3 py-2 cursor-pointer border-b border-gray-700/50 transition-colors ${
                    isSelected
                      ? 'bg-blue-600/90 text-white'
                      : 'text-gray-300 hover:bg-gray-700/50'
                  }`}
                >
                  <div className="flex items-center gap-2 min-w-0">
                    <Database className={`w-4 h-4 flex-shrink-0 ${isSelected ? 'text-white' : 'text-blue-400'}`} />
                    <div className="min-w-0">
                      <div className="text-sm font-medium truncate">{app.slug}</div>
                      <div className={`text-xs ${isSelected ? 'text-blue-100' : 'text-gray-500'}`}>
                        {app.tables?.length || 0} tables
                        {app.dbSizeBytes ? ` · ${formatBytes(app.dbSizeBytes)}` : ''}
                      </div>
                    </div>
                  </div>
                  <ChevronRight className={`w-4 h-4 flex-shrink-0 ${isSelected ? 'text-white' : 'text-gray-600'}`} />
                </div>
              );
            })}
          </div>
        </div>

        {/* Column 2: Tables */}
        <div className="w-72 flex-shrink-0 border-r border-gray-700 flex flex-col bg-gray-800/80">
          <ColumnHeader>
            {selectedApp ? `Tables — ${selectedApp.slug} (${tables.length})` : 'Tables'}
          </ColumnHeader>
          {!selectedApp ? (
            <EmptyState text="Sélectionnez une application" />
          ) : loadingTables ? (
            <Spinner />
          ) : tables.length === 0 ? (
            <EmptyState icon={Table2} text="Aucune table" />
          ) : (
            <div className="flex-1 overflow-y-auto">
              {tables.map(table => {
                const isSelected = selectedTableName === table.name;
                return (
                  <div
                    key={table.name}
                    onClick={() => selectTable(table.name)}
                    className={`flex items-center justify-between px-3 py-2 cursor-pointer border-b border-gray-700/50 transition-colors ${
                      isSelected
                        ? 'bg-blue-600/90 text-white'
                        : 'text-gray-300 hover:bg-gray-700/50'
                    }`}
                  >
                    <div className="flex items-center gap-2 min-w-0">
                      <Table2 className={`w-4 h-4 flex-shrink-0 ${isSelected ? 'text-white' : 'text-green-400'}`} />
                      <div className="min-w-0">
                        <div className="text-sm font-mono truncate">{table.name}</div>
                        <div className={`text-xs ${isSelected ? 'text-blue-100' : 'text-gray-500'}`}>
                          {table.columns?.length || 0} col · {table.row_count || 0} lignes
                        </div>
                      </div>
                    </div>
                    <ChevronRight className={`w-4 h-4 flex-shrink-0 ${isSelected ? 'text-white' : 'text-gray-600'}`} />
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Column 3: Columns */}
        <div className="flex-1 flex flex-col bg-gray-800/60">
          <ColumnHeader>
            {tableDetail ? `Colonnes — ${tableDetail.name} (${columns.length})` : 'Colonnes'}
          </ColumnHeader>
          {!selectedTableName ? (
            <EmptyState text={selectedApp ? 'Sélectionnez une table' : 'Sélectionnez une application'} />
          ) : loadingColumns ? (
            <Spinner />
          ) : columns.length === 0 ? (
            <EmptyState text="Aucune colonne" />
          ) : (
            <div className="flex-1 overflow-y-auto">
              {columns.map(col => (
                <div
                  key={col.name}
                  className="flex items-center justify-between px-4 py-2 border-b border-gray-700/50 text-gray-300"
                >
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-mono">{col.name}</span>
                    {col.required && (
                      <span className="text-xs text-yellow-400" title="Requis">*</span>
                    )}
                    {col.unique && (
                      <span className="text-xs px-1 py-0.5 rounded bg-blue-900/30 text-blue-400" title="Unique">U</span>
                    )}
                  </div>
                  <FieldTypeBadge type={col.field_type} />
                </div>
              ))}
              <div className="px-4 py-3 border-t border-gray-700 bg-gray-800/40 flex-shrink-0">
                <div className="flex items-center gap-4 text-xs text-gray-500">
                  <span>{tableDetail.row_count || 0} lignes</span>
                  <span>{columns.length} colonnes</span>
                  <span>v{selectedApp?.version || 0}</span>
                </div>
              </div>
            </div>
          )}
        </div>

      </div>
    </div>
  );
}

function formatBytes(bytes) {
  if (!bytes) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(0)) + ' ' + sizes[i];
}

export default Dataverse;
