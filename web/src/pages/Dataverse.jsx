import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import {
  Database, ChevronRight, Table2, Loader2,
  Download, History, Network
} from 'lucide-react';
import PageHeader from '../components/PageHeader';
import {
  getDataverseOverview, getDataverseTables, getDataverseTable,
  getDataverseSchema, getDataverseMigrations, downloadDataverseBackup
} from '../api/client';

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

function ColumnHeader({ children, actions }) {
  return (
    <div className="px-3 py-2 border-b border-gray-700 bg-gray-800/60 flex-shrink-0">
      <div className="flex items-center justify-between">
        <h3 className="text-xs font-semibold text-gray-400 uppercase tracking-wider">
          {children}
        </h3>
        {actions && <div className="flex items-center gap-1">{actions}</div>}
      </div>
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

/* ------------------------------------------------------------------ */
/*  ER Diagram (inline SVG)                                           */
/* ------------------------------------------------------------------ */

const TABLE_BOX_W = 220;
const TABLE_HEADER_H = 28;
const TABLE_ROW_H = 22;
const TABLE_PAD_X = 40;
const TABLE_PAD_Y = 30;
const GRID_COLS = 3;

function computeTableHeight(table) {
  const cols = table.columns?.length || 0;
  return TABLE_HEADER_H + cols * TABLE_ROW_H + 8;
}

function ERDiagram({ schema }) {
  const containerRef = useRef(null);
  const tables = schema?.tables || [];
  const relations = schema?.relations || [];

  const layout = useMemo(() => {
    const positions = {};
    let maxX = 0;
    let maxY = 0;
    // Track max height per row so rows don't overlap
    const rowMaxH = {};

    tables.forEach((t, idx) => {
      const col = idx % GRID_COLS;
      const row = Math.floor(idx / GRID_COLS);

      // Compute Y from accumulated row heights
      let y = TABLE_PAD_Y;
      for (let r = 0; r < row; r++) {
        y += (rowMaxH[r] || 0) + TABLE_PAD_Y;
      }

      const x = TABLE_PAD_X + col * (TABLE_BOX_W + TABLE_PAD_X);
      const h = computeTableHeight(t);

      if (!rowMaxH[row] || h > rowMaxH[row]) {
        rowMaxH[row] = h;
      }

      positions[t.name] = { x, y, w: TABLE_BOX_W, h };
      if (x + TABLE_BOX_W > maxX) maxX = x + TABLE_BOX_W;
      if (y + h > maxY) maxY = y + h;
    });

    // Recalculate Y with final row heights
    tables.forEach((t, idx) => {
      const col = idx % GRID_COLS;
      const row = Math.floor(idx / GRID_COLS);
      let y = TABLE_PAD_Y;
      for (let r = 0; r < row; r++) {
        y += (rowMaxH[r] || 0) + TABLE_PAD_Y;
      }
      const x = TABLE_PAD_X + col * (TABLE_BOX_W + TABLE_PAD_X);
      const h = computeTableHeight(t);
      positions[t.name] = { x, y, w: TABLE_BOX_W, h };
      if (y + h > maxY) maxY = y + h;
    });

    return { positions, width: maxX + TABLE_PAD_X, height: maxY + TABLE_PAD_Y };
  }, [tables]);

  // Build column index for each table for connector positioning
  const tableColIndex = useMemo(() => {
    const idx = {};
    tables.forEach(t => {
      const m = {};
      (t.columns || []).forEach((c, i) => { m[c.name] = i; });
      idx[t.name] = m;
    });
    return idx;
  }, [tables]);

  if (tables.length === 0) {
    return <EmptyState icon={Network} text="Aucune table dans le schema" />;
  }

  return (
    <div ref={containerRef} className="flex-1 overflow-auto bg-gray-900/40">
      <svg
        width={layout.width}
        height={layout.height}
        className="min-w-full"
        style={{ minWidth: layout.width, minHeight: layout.height }}
      >
        {/* Relations lines */}
        {relations.map((rel, i) => {
          const fromPos = layout.positions[rel.from_table];
          const toPos = layout.positions[rel.to_table];
          if (!fromPos || !toPos) return null;

          const fromColIdx = tableColIndex[rel.from_table]?.[rel.from_column] ?? 0;
          const toColIdx = tableColIndex[rel.to_table]?.[rel.to_column] ?? 0;

          const fromY = fromPos.y + TABLE_HEADER_H + fromColIdx * TABLE_ROW_H + TABLE_ROW_H / 2;
          const toY = toPos.y + TABLE_HEADER_H + toColIdx * TABLE_ROW_H + TABLE_ROW_H / 2;

          // Decide side: connect from right if target is to the right, otherwise from left
          let x1, x2;
          if (fromPos.x < toPos.x) {
            x1 = fromPos.x + fromPos.w;
            x2 = toPos.x;
          } else if (fromPos.x > toPos.x) {
            x1 = fromPos.x;
            x2 = toPos.x + toPos.w;
          } else {
            // Same column: connect left-to-left with an offset
            x1 = fromPos.x;
            x2 = toPos.x;
          }

          const midX = (x1 + x2) / 2;

          return (
            <g key={i}>
              <path
                d={`M ${x1} ${fromY} C ${midX} ${fromY}, ${midX} ${toY}, ${x2} ${toY}`}
                fill="none"
                stroke="#6366f1"
                strokeWidth="1.5"
                strokeOpacity="0.6"
              />
              {/* Arrow at target end */}
              <circle cx={x2} cy={toY} r="3" fill="#6366f1" fillOpacity="0.8" />
              {/* Label */}
              <text
                x={midX}
                y={Math.min(fromY, toY) - 6}
                textAnchor="middle"
                className="text-[9px]"
                fill="#818cf8"
                fillOpacity="0.7"
              >
                {rel.relation_type || ''}
              </text>
            </g>
          );
        })}

        {/* Table boxes */}
        {tables.map(table => {
          const pos = layout.positions[table.name];
          if (!pos) return null;
          const cols = table.columns || [];

          return (
            <g key={table.name}>
              {/* Box background */}
              <rect
                x={pos.x}
                y={pos.y}
                width={pos.w}
                height={pos.h}
                rx="6"
                fill="#1f2937"
                stroke="#374151"
                strokeWidth="1"
              />
              {/* Header */}
              <rect
                x={pos.x}
                y={pos.y}
                width={pos.w}
                height={TABLE_HEADER_H}
                rx="6"
                fill="#1e3a5f"
              />
              {/* Cover bottom corners of header rect */}
              <rect
                x={pos.x}
                y={pos.y + TABLE_HEADER_H - 6}
                width={pos.w}
                height="6"
                fill="#1e3a5f"
              />
              <text
                x={pos.x + 10}
                y={pos.y + TABLE_HEADER_H / 2 + 1}
                dominantBaseline="middle"
                className="text-xs font-semibold"
                fill="#93c5fd"
              >
                {table.name}
              </text>
              {table.row_count != null && (
                <text
                  x={pos.x + pos.w - 8}
                  y={pos.y + TABLE_HEADER_H / 2 + 1}
                  dominantBaseline="middle"
                  textAnchor="end"
                  className="text-[9px]"
                  fill="#6b7280"
                >
                  {table.row_count} rows
                </text>
              )}

              {/* Columns */}
              {cols.map((col, ci) => {
                const cy = pos.y + TABLE_HEADER_H + ci * TABLE_ROW_H + TABLE_ROW_H / 2;
                const isLookup = col.field_type === 'lookup';
                return (
                  <g key={col.name}>
                    <text
                      x={pos.x + 10}
                      y={cy + 1}
                      dominantBaseline="middle"
                      className="text-[11px]"
                      fill={isLookup ? '#f472b6' : '#d1d5db'}
                      fontFamily="monospace"
                    >
                      {col.name}
                      {col.required ? ' *' : ''}
                    </text>
                    <text
                      x={pos.x + pos.w - 8}
                      y={cy + 1}
                      dominantBaseline="middle"
                      textAnchor="end"
                      className="text-[9px]"
                      fill="#6b7280"
                    >
                      {col.field_type}
                    </text>
                  </g>
                );
              })}
            </g>
          );
        })}
      </svg>
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Migrations panel                                                  */
/* ------------------------------------------------------------------ */

function MigrationsPanel({ migrations, loading }) {
  if (loading) return <Spinner />;
  if (!migrations || migrations.length === 0) {
    return <EmptyState icon={History} text="Aucune migration" />;
  }

  return (
    <div className="flex-1 overflow-y-auto">
      {migrations.map((m) => (
        <div
          key={m.id}
          className="px-4 py-3 border-b border-gray-700/50 text-gray-300"
        >
          <div className="flex items-center justify-between mb-1">
            <span className="text-sm font-mono text-blue-400">#{m.id}</span>
            <span className="text-xs text-gray-500">
              {m.applied_at ? new Date(m.applied_at).toLocaleString('fr-FR') : '--'}
            </span>
          </div>
          <p className="text-sm text-gray-200 mb-1">{m.description || 'Sans description'}</p>
          {m.operations && (
            <details className="mt-1">
              <summary className="text-xs text-gray-500 cursor-pointer hover:text-gray-400">
                Operations
              </summary>
              <pre className="mt-1 text-xs text-gray-500 bg-gray-900/50 rounded p-2 overflow-x-auto max-h-40">
                {(() => {
                  try {
                    return JSON.stringify(JSON.parse(m.operations), null, 2);
                  } catch {
                    return m.operations;
                  }
                })()}
              </pre>
            </details>
          )}
        </div>
      ))}
    </div>
  );
}

/* ------------------------------------------------------------------ */
/*  Main Component                                                    */
/* ------------------------------------------------------------------ */

function Dataverse() {
  const [apps, setApps] = useState([]);
  const [loading, setLoading] = useState(true);

  // View mode: 'schema' (3-column) or 'diagram' (ER)
  const [viewMode, setViewMode] = useState('schema');

  // Column 2: tables fetched per click
  const [selectedApp, setSelectedApp] = useState(null);
  const [tables, setTables] = useState([]);
  const [loadingTables, setLoadingTables] = useState(false);

  // Column 3: columns fetched per click
  const [selectedTableName, setSelectedTableName] = useState(null);
  const [tableDetail, setTableDetail] = useState(null);
  const [loadingColumns, setLoadingColumns] = useState(false);

  // Column 3 tab: 'columns' or 'migrations'
  const [col3Tab, setCol3Tab] = useState('columns');

  // Migrations data
  const [migrations, setMigrations] = useState([]);
  const [loadingMigrations, setLoadingMigrations] = useState(false);

  // ER diagram schema
  const [erSchema, setErSchema] = useState(null);
  const [loadingEr, setLoadingEr] = useState(false);

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
    setCol3Tab('columns');
    setMigrations([]);
    setErSchema(null);
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
    setCol3Tab('columns');
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

  // Fetch migrations when tab switches
  async function fetchMigrations() {
    if (!selectedApp) return;
    setLoadingMigrations(true);
    try {
      const res = await getDataverseMigrations(selectedApp.appId);
      setMigrations(res.data?.migrations || []);
    } catch {
      setMigrations([]);
    } finally {
      setLoadingMigrations(false);
    }
  }

  function handleCol3TabChange(tab) {
    setCol3Tab(tab);
    if (tab === 'migrations') {
      setSelectedTableName(null);
      setTableDetail(null);
      fetchMigrations();
    }
  }

  // Fetch ER schema when switching to diagram view
  async function fetchErSchema(appId) {
    setLoadingEr(true);
    try {
      const res = await getDataverseSchema(appId);
      setErSchema(res.data || null);
    } catch {
      setErSchema(null);
    } finally {
      setLoadingEr(false);
    }
  }

  function handleViewToggle(mode) {
    setViewMode(mode);
    if (mode === 'diagram' && selectedApp) {
      fetchErSchema(selectedApp.appId);
    }
  }

  const filteredApps = apps;

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
      <PageHeader icon={Database} title="Dataverse">
        <div className="flex items-center gap-3">
          {/* View mode toggle */}
          <div className="flex items-center gap-1 bg-gray-700/50 rounded-lg p-0.5">
            <button
              onClick={() => handleViewToggle('schema')}
              className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-colors ${
                viewMode === 'schema'
                  ? 'bg-blue-600 text-white'
                  : 'text-gray-400 hover:text-gray-200'
              }`}
            >
              <Table2 className="w-3.5 h-3.5" />
              Schema
            </button>
            <button
              onClick={() => handleViewToggle('diagram')}
              className={`flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium transition-colors ${
                viewMode === 'diagram'
                  ? 'bg-blue-600 text-white'
                  : 'text-gray-400 hover:text-gray-200'
              }`}
            >
              <Network className="w-3.5 h-3.5" />
              Diagramme ER
            </button>
          </div>
        </div>
      </PageHeader>

      {viewMode === 'diagram' ? (
        /* ER Diagram view */
        <div className="flex-1 min-h-0 flex">
          {/* App sidebar for diagram view */}
          <div className="w-64 flex-shrink-0 border-r border-gray-700 flex flex-col bg-gray-800">
            <ColumnHeader>Applications ({filteredApps.length})</ColumnHeader>
            <div className="flex-1 overflow-y-auto">
              {filteredApps.map(app => {
                const isSelected = selectedApp?.appId === app.appId;
                return (
                  <div
                    key={app.appId}
                    onClick={() => {
                      selectApp(app);
                      fetchErSchema(app.appId);
                    }}
                    className={`flex items-center justify-between px-3 py-2 cursor-pointer border-b border-gray-700/50 transition-colors ${
                      isSelected
                        ? 'bg-blue-600/90 text-white'
                        : 'text-gray-300 hover:bg-gray-700/50'
                    }`}
                  >
                    <div className="flex items-center gap-2 min-w-0">
                      <Database className={`w-4 h-4 flex-shrink-0 ${isSelected ? 'text-white' : 'text-blue-400'}`} />
                      <div className="min-w-0">
                        <span className="text-sm font-medium truncate">{app.slug}</span>
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

          {/* Diagram area */}
          <div className="flex-1 flex flex-col bg-gray-800/60">
            <ColumnHeader>
              {selectedApp ? `Diagramme ER — ${selectedApp.slug}` : 'Diagramme ER'}
            </ColumnHeader>
            {!selectedApp ? (
              <EmptyState text="Selectionnez une application" />
            ) : loadingEr ? (
              <Spinner />
            ) : (
              <ERDiagram schema={erSchema} />
            )}
          </div>
        </div>
      ) : (
        /* Schema view (3-column layout) */
        <div className="flex-1 min-h-0 flex">

          {/* Column 1: Apps */}
          <div className="w-64 flex-shrink-0 border-r border-gray-700 flex flex-col bg-gray-800">
            <ColumnHeader>Applications ({filteredApps.length})</ColumnHeader>
            <div className="flex-1 overflow-y-auto">
              {filteredApps.map(app => {
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
                        <span className="text-sm font-medium truncate">{app.slug}</span>
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
            <ColumnHeader
              actions={
                selectedApp ? (
                  <button
                    onClick={() => downloadDataverseBackup(selectedApp.appId)}
                    className="flex items-center gap-1 px-2 py-1 rounded text-xs text-gray-400 hover:text-blue-400 hover:bg-gray-700/50 transition-colors"
                    title="Telecharger un backup"
                  >
                    <Download className="w-3.5 h-3.5" />
                    Backup
                  </button>
                ) : null
              }
            >
              {selectedApp ? `Tables — ${selectedApp.slug} (${tables.length})` : 'Tables'}
            </ColumnHeader>
            {!selectedApp ? (
              <EmptyState text="Selectionnez une application" />
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

          {/* Column 3: Columns / Migrations */}
          <div className="flex-1 flex flex-col bg-gray-800/60">
            {/* Tab header */}
            <div className="px-3 py-2 border-b border-gray-700 bg-gray-800/60 flex-shrink-0">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-1">
                  <button
                    onClick={() => handleCol3TabChange('columns')}
                    className={`flex items-center gap-1 px-2 py-1 rounded text-xs font-semibold uppercase tracking-wider transition-colors ${
                      col3Tab === 'columns'
                        ? 'text-blue-400 bg-blue-900/20'
                        : 'text-gray-500 hover:text-gray-300'
                    }`}
                  >
                    <Table2 className="w-3 h-3" />
                    Colonnes
                  </button>
                  {selectedApp && (
                    <button
                      onClick={() => handleCol3TabChange('migrations')}
                      className={`flex items-center gap-1 px-2 py-1 rounded text-xs font-semibold uppercase tracking-wider transition-colors ${
                        col3Tab === 'migrations'
                          ? 'text-blue-400 bg-blue-900/20'
                          : 'text-gray-500 hover:text-gray-300'
                      }`}
                    >
                      <History className="w-3 h-3" />
                      Migrations
                    </button>
                  )}
                </div>
                {col3Tab === 'columns' && tableDetail && (
                  <span className="text-xs text-gray-500">
                    {tableDetail.name} ({columns.length})
                  </span>
                )}
                {col3Tab === 'migrations' && (
                  <span className="text-xs text-gray-500">
                    {migrations.length} migration{migrations.length !== 1 ? 's' : ''}
                  </span>
                )}
              </div>
            </div>

            {col3Tab === 'columns' ? (
              /* Columns view */
              <>
                {!selectedTableName ? (
                  <EmptyState text={selectedApp ? 'Selectionnez une table' : 'Selectionnez une application'} />
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
              </>
            ) : (
              /* Migrations view */
              <MigrationsPanel migrations={migrations} loading={loadingMigrations} />
            )}
          </div>

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
  return parseFloat((bytes / Math.pow(k, i)).toFixed(0)) + ' ' + sizes[i];
}

export default Dataverse;
