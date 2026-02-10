import { useState, useEffect, useCallback, useRef } from 'react';
import {
  Database, Table2, Loader2, Plus, Trash2, X, Download,
  ChevronUp, ChevronDown, ChevronLeft, ChevronRight,
  ChevronsLeft, ChevronsRight, ChevronRight as ChevronRightIcon
} from 'lucide-react';
import PageHeader from '../components/PageHeader';
import {
  getDataverseOverview, getDataverseTable, getDataverseRows,
  insertDataverseRows, updateDataverseRows, deleteDataverseRows,
  downloadDataverseBackup
} from '../api/client';

const SYSTEM_COLUMNS = ['id', 'created_at', 'updated_at'];
const ROWS_PER_PAGE_OPTIONS = [25, 50, 100];

// ── Shared UI components ─────────────────────────────────────

function inputForType(fieldType, value, onChange, autoFocus = false, choices = []) {
  const base = 'w-full bg-gray-700 border border-gray-600 rounded px-2 py-1 text-sm text-gray-200 focus:outline-none focus:border-blue-500';

  if (fieldType === 'boolean') {
    return (
      <input
        type="checkbox"
        checked={!!value}
        onChange={e => onChange(e.target.checked)}
        autoFocus={autoFocus}
        className="w-4 h-4 accent-blue-500"
      />
    );
  }

  if (fieldType === 'choice') {
    return (
      <select value={value || ''} onChange={e => onChange(e.target.value)} autoFocus={autoFocus} className={base}>
        <option value="">--</option>
        {choices.map(c => <option key={c} value={c}>{c}</option>)}
      </select>
    );
  }

  if (fieldType === 'json') {
    return (
      <textarea
        value={value || ''}
        onChange={e => onChange(e.target.value)}
        autoFocus={autoFocus}
        rows={3}
        className={base + ' font-mono text-xs'}
      />
    );
  }

  let type = 'text';
  if (fieldType === 'number' || fieldType === 'decimal' || fieldType === 'currency' || fieldType === 'percent' || fieldType === 'auto_increment') {
    type = 'number';
  } else if (fieldType === 'date') {
    type = 'date';
  } else if (fieldType === 'time') {
    type = 'time';
  } else if (fieldType === 'date_time') {
    type = 'datetime-local';
  } else if (fieldType === 'email') {
    type = 'email';
  } else if (fieldType === 'url') {
    type = 'url';
  }

  return (
    <input
      type={type}
      value={value ?? ''}
      onChange={e => onChange(type === 'number' ? (e.target.value === '' ? '' : Number(e.target.value)) : e.target.value)}
      autoFocus={autoFocus}
      className={base}
    />
  );
}

function AddRowModal({ columns, onClose, onAdd }) {
  const editableColumns = columns.filter(c => !SYSTEM_COLUMNS.includes(c.name));
  const [values, setValues] = useState(() => {
    const init = {};
    editableColumns.forEach(c => {
      init[c.name] = c.field_type === 'boolean' ? false : '';
    });
    return init;
  });
  const [submitting, setSubmitting] = useState(false);

  const handleSubmit = async (e) => {
    e.preventDefault();
    setSubmitting(true);
    try {
      await onAdd(values);
      onClose();
    } catch (err) {
      console.error('Insert failed:', err);
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onClick={onClose}>
      <div className="bg-gray-800 border border-gray-700 rounded-lg shadow-xl w-full max-w-lg max-h-[80vh] flex flex-col" onClick={e => e.stopPropagation()}>
        <div className="flex items-center justify-between px-5 py-3 border-b border-gray-700">
          <h2 className="text-lg font-semibold text-gray-200">Ajouter une ligne</h2>
          <button onClick={onClose} className="text-gray-400 hover:text-gray-200">
            <X className="w-5 h-5" />
          </button>
        </div>
        <form onSubmit={handleSubmit} className="flex-1 overflow-y-auto px-5 py-4 space-y-3">
          {editableColumns.map(col => (
            <div key={col.name}>
              <label className="block text-sm text-gray-400 mb-1">
                {col.name}
                {col.required && <span className="text-yellow-400 ml-1">*</span>}
                <span className="text-xs text-gray-600 ml-2">{col.field_type}</span>
              </label>
              {inputForType(col.field_type, values[col.name], v => setValues(prev => ({ ...prev, [col.name]: v })), false, col.choices || [])}
            </div>
          ))}
        </form>
        <div className="flex justify-end gap-2 px-5 py-3 border-t border-gray-700">
          <button type="button" onClick={onClose} className="px-4 py-1.5 text-sm text-gray-400 hover:text-gray-200 bg-gray-700 rounded">
            Annuler
          </button>
          <button
            onClick={handleSubmit}
            disabled={submitting}
            className="px-4 py-1.5 text-sm bg-blue-600 hover:bg-blue-500 text-white rounded disabled:opacity-50 flex items-center gap-2"
          >
            {submitting && <Loader2 className="w-3 h-3 animate-spin" />}
            Ajouter
          </button>
        </div>
      </div>
    </div>
  );
}

function DeleteConfirm({ onConfirm, onCancel }) {
  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50" onClick={onCancel}>
      <div className="bg-gray-800 border border-gray-700 rounded-lg shadow-xl p-5 w-full max-w-sm" onClick={e => e.stopPropagation()}>
        <h3 className="text-lg font-semibold text-gray-200 mb-2">Confirmer la suppression</h3>
        <p className="text-sm text-gray-400 mb-4">Cette action est irreversible. Voulez-vous supprimer cette ligne ?</p>
        <div className="flex justify-end gap-2">
          <button onClick={onCancel} className="px-4 py-1.5 text-sm text-gray-400 hover:text-gray-200 bg-gray-700 rounded">
            Annuler
          </button>
          <button onClick={onConfirm} className="px-4 py-1.5 text-sm bg-red-600 hover:bg-red-500 text-white rounded">
            Supprimer
          </button>
        </div>
      </div>
    </div>
  );
}

function InlineEdit({ value, fieldType, choices, onSave, onCancel }) {
  const [editValue, setEditValue] = useState(value ?? '');

  useEffect(() => {
    const handleKeyDown = (e) => {
      if (e.key === 'Escape') onCancel();
      if (e.key === 'Enter' && fieldType !== 'json') onSave(editValue);
    };
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [editValue, fieldType, onSave, onCancel]);

  return (
    <div className="min-w-[100px]" onBlur={() => onSave(editValue)}>
      {inputForType(fieldType, editValue, setEditValue, true, choices || [])}
    </div>
  );
}

// ── Main Component ───────────────────────────────────────────

function DataBrowser() {
  const [apps, setApps] = useState([]);
  const [loading, setLoading] = useState(true);

  // Navigation state
  const [selectedAppId, setSelectedAppId] = useState(null);
  const [selectedTableName, setSelectedTableName] = useState(null);

  // Table schema + data
  const [tableInfo, setTableInfo] = useState(null);
  const [rows, setRows] = useState([]);
  const [total, setTotal] = useState(0);
  const [loadingRows, setLoadingRows] = useState(false);

  // Pagination & sorting
  const [page, setPage] = useState(1);
  const [rowsPerPage, setRowsPerPage] = useState(50);
  const [orderBy, setOrderBy] = useState('id');
  const [orderDesc, setOrderDesc] = useState(true);

  // Modals
  const [showAddModal, setShowAddModal] = useState(false);
  const [deleteRowId, setDeleteRowId] = useState(null);
  const [editingCell, setEditingCell] = useState(null);

  const columns = tableInfo?.columns || [];
  const allColumns = ['id', ...columns.filter(c => !SYSTEM_COLUMNS.includes(c.name)).map(c => c.name), 'created_at', 'updated_at'];
  const totalPages = Math.max(1, Math.ceil(total / rowsPerPage));

  const selectedApp = apps.find(a => a.appId === selectedAppId);

  // ── Data fetching ──────────────────────────────────────

  const fetchApps = useCallback(async () => {
    try {
      const res = await getDataverseOverview();
      setApps(res.data?.apps || []);
    } catch (err) {
      console.error('Failed to fetch apps:', err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { fetchApps(); }, [fetchApps]);

  // Auto-select first app
  useEffect(() => {
    if (apps.length > 0 && !selectedAppId) {
      setSelectedAppId(apps[0].appId);
    }
  }, [apps, selectedAppId]);

  // Fetch table schema when table is selected
  useEffect(() => {
    if (!selectedAppId || !selectedTableName) {
      setTableInfo(null);
      setRows([]);
      setTotal(0);
      return;
    }
    let cancelled = false;
    (async () => {
      try {
        const res = await getDataverseTable(selectedAppId, selectedTableName);
        if (!cancelled) setTableInfo(res.data?.table || null);
      } catch {
        if (!cancelled) setTableInfo(null);
      }
    })();
    return () => { cancelled = true; };
  }, [selectedAppId, selectedTableName]);

  const fetchRows = useCallback(async () => {
    if (!selectedAppId || !selectedTableName) return;
    setLoadingRows(true);
    try {
      const res = await getDataverseRows(selectedAppId, selectedTableName, {
        limit: rowsPerPage,
        offset: (page - 1) * rowsPerPage,
        order_by: orderBy,
        order_desc: orderDesc,
      });
      setRows(res.data?.data?.rows || []);
      setTotal(res.data?.data?.total || 0);
    } catch (err) {
      console.error('Failed to fetch rows:', err);
    } finally {
      setLoadingRows(false);
    }
  }, [selectedAppId, selectedTableName, page, rowsPerPage, orderBy, orderDesc]);

  useEffect(() => { if (tableInfo) fetchRows(); }, [fetchRows, tableInfo]);

  // ── Navigation handlers ────────────────────────────────

  function selectApp(appId) {
    if (appId === selectedAppId) return;
    setSelectedAppId(appId);
    setSelectedTableName(null);
    setTableInfo(null);
    setRows([]);
    setTotal(0);
    setPage(1);
    setEditingCell(null);
  }

  function selectTable(tableName) {
    setSelectedTableName(tableName);
    setPage(1);
    setOrderBy('id');
    setOrderDesc(true);
    setEditingCell(null);
  }

  // ── Data handlers ──────────────────────────────────────

  function handleSort(col) {
    if (orderBy === col) {
      setOrderDesc(!orderDesc);
    } else {
      setOrderBy(col);
      setOrderDesc(false);
    }
    setPage(1);
  }

  async function handleAddRow(values) {
    await insertDataverseRows(selectedAppId, selectedTableName, [values]);
    fetchRows();
  }

  async function handleDeleteRow() {
    if (deleteRowId == null) return;
    try {
      await deleteDataverseRows(selectedAppId, selectedTableName, [{ column: 'id', op: 'eq', value: deleteRowId }]);
      setDeleteRowId(null);
      fetchRows();
    } catch (err) {
      console.error('Delete failed:', err);
    }
  }

  async function handleInlineSave(rowId, columnName, newValue) {
    setEditingCell(null);
    try {
      await updateDataverseRows(selectedAppId, selectedTableName, {
        updates: { [columnName]: newValue },
        filters: [{ column: 'id', op: 'eq', value: rowId }],
      });
      fetchRows();
    } catch (err) {
      console.error('Update failed:', err);
    }
  }

  function getColumnInfo(name) {
    return columns.find(c => c.name === name);
  }

  function formatCellValue(value) {
    if (value === null || value === undefined) return <span className="text-gray-600 italic">null</span>;
    if (typeof value === 'boolean') return value ? 'true' : 'false';
    if (typeof value === 'object') return JSON.stringify(value);
    const str = String(value);
    if (str.length > 120) return str.slice(0, 120) + '...';
    return str;
  }

  function changePage(p) {
    setPage(Math.max(1, Math.min(p, totalPages)));
  }

  function changeRowsPerPage(rpp) {
    setRowsPerPage(rpp);
    setPage(1);
  }

  // ── Pagination renderer ────────────────────────────────

  function renderPagination() {
    const pages = [];
    const maxVisible = 7;
    let start = Math.max(1, page - Math.floor(maxVisible / 2));
    let end = Math.min(totalPages, start + maxVisible - 1);
    if (end - start + 1 < maxVisible) start = Math.max(1, end - maxVisible + 1);

    for (let i = start; i <= end; i++) pages.push(i);

    const btnClass = 'px-2 py-1 text-sm rounded transition-colors disabled:opacity-30';
    const activeClass = 'bg-blue-600 text-white';
    const inactiveClass = 'text-gray-400 hover:bg-gray-700';

    return (
      <div className="flex items-center justify-center gap-1">
        <button onClick={() => changePage(1)} disabled={page === 1} className={btnClass + ' ' + inactiveClass}>
          <ChevronsLeft className="w-4 h-4" />
        </button>
        <button onClick={() => changePage(page - 1)} disabled={page === 1} className={btnClass + ' ' + inactiveClass}>
          <ChevronLeft className="w-4 h-4" />
        </button>
        {start > 1 && <span className="text-gray-600 px-1">...</span>}
        {pages.map(p => (
          <button key={p} onClick={() => changePage(p)} className={`${btnClass} min-w-[28px] ${p === page ? activeClass : inactiveClass}`}>
            {p}
          </button>
        ))}
        {end < totalPages && <span className="text-gray-600 px-1">...</span>}
        <button onClick={() => changePage(page + 1)} disabled={page === totalPages} className={btnClass + ' ' + inactiveClass}>
          <ChevronRight className="w-4 h-4" />
        </button>
        <button onClick={() => changePage(totalPages)} disabled={page === totalPages} className={btnClass + ' ' + inactiveClass}>
          <ChevronsRight className="w-4 h-4" />
        </button>
      </div>
    );
  }

  // ── Render ─────────────────────────────────────────────

  if (loading) {
    return (
      <div className="h-full flex flex-col">
        <PageHeader icon={Table2} title="Data Browser" />
        <div className="flex-1 flex items-center justify-center">
          <Loader2 className="w-8 h-8 text-blue-400 animate-spin" />
        </div>
      </div>
    );
  }

  if (apps.length === 0) {
    return (
      <div className="h-full flex flex-col">
        <PageHeader icon={Table2} title="Data Browser" />
        <div className="flex-1 flex items-center justify-center">
          <div className="text-center">
            <Database className="w-12 h-12 text-gray-600 mx-auto mb-3" />
            <p className="text-gray-400">Aucune application avec Dataverse.</p>
            <p className="text-gray-500 text-sm mt-1">Les donnees apparaitront ici quand une app aura cree des tables.</p>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="h-full flex flex-col">
      <PageHeader icon={Table2} title="Data Browser">
        {selectedApp && (
          <button
            onClick={() => downloadDataverseBackup(selectedAppId)}
            className="flex items-center gap-1.5 px-3 py-1.5 text-sm text-gray-300 hover:text-white bg-gray-700 hover:bg-gray-600 rounded transition-colors"
            title="Telecharger un backup SQLite"
          >
            <Download className="w-4 h-4" />
            Backup
          </button>
        )}
        {selectedTableName && (
          <span className="text-xs px-2 py-0.5 rounded bg-blue-900/40 text-blue-400">{total} lignes</span>
        )}
      </PageHeader>

      <div className="flex-1 min-h-0 flex">

        {/* Left panel: Apps + Tables navigation */}
        <div className="w-60 flex-shrink-0 border-r border-gray-700 flex flex-col bg-gray-800 overflow-y-auto">
          {apps.map(app => {
            const isExpanded = app.appId === selectedAppId;
            const appTables = app.tables || [];
            return (
              <div key={app.appId}>
                {/* App header */}
                <div
                  onClick={() => selectApp(app.appId)}
                  className={`flex items-center justify-between px-3 py-2 cursor-pointer border-b border-gray-700/50 transition-colors ${
                    isExpanded
                      ? 'bg-blue-600/20 text-white'
                      : 'text-gray-300 hover:bg-gray-700/50'
                  }`}
                >
                  <div className="flex items-center gap-2 min-w-0">
                    <Database className={`w-4 h-4 flex-shrink-0 ${isExpanded ? 'text-blue-400' : 'text-gray-500'}`} />
                    <div className="min-w-0">
                      <div className="text-sm font-medium truncate">{app.slug}</div>
                      <div className="text-xs text-gray-500">
                        {appTables.length} tables
                      </div>
                    </div>
                  </div>
                  <ChevronRightIcon className={`w-4 h-4 flex-shrink-0 transition-transform ${
                    isExpanded ? 'rotate-90 text-blue-400' : 'text-gray-600'
                  }`} />
                </div>

                {/* Tables list (expanded) */}
                {isExpanded && (
                  <div className="bg-gray-900/30">
                    {appTables.length === 0 ? (
                      <div className="px-4 py-3 text-xs text-gray-600 italic">Aucune table</div>
                    ) : (
                      appTables.map(table => {
                        const isActive = selectedTableName === table.name;
                        return (
                          <div
                            key={table.name}
                            onClick={() => selectTable(table.name)}
                            className={`flex items-center gap-2 px-4 pl-8 py-1.5 cursor-pointer transition-colors text-sm ${
                              isActive
                                ? 'bg-blue-600/90 text-white'
                                : 'text-gray-400 hover:bg-gray-700/50 hover:text-gray-200'
                            }`}
                          >
                            <Table2 className={`w-3.5 h-3.5 flex-shrink-0 ${isActive ? 'text-white' : 'text-green-500/60'}`} />
                            <span className="font-mono truncate">{table.name}</span>
                            <span className={`text-xs ml-auto flex-shrink-0 ${isActive ? 'text-blue-100' : 'text-gray-600'}`}>
                              {table.rowsCount ?? table.row_count ?? 0}
                            </span>
                          </div>
                        );
                      })
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>

        {/* Main panel: Data table */}
        <div className="flex-1 flex flex-col min-w-0">

          {!selectedTableName ? (
            /* Empty state */
            <div className="flex-1 flex items-center justify-center">
              <div className="text-center">
                <Table2 className="w-10 h-10 text-gray-600 mx-auto mb-2" />
                <p className="text-gray-500 text-sm">Selectionnez une table pour voir ses donnees.</p>
              </div>
            </div>
          ) : (
            <>
              {/* Toolbar */}
              <div className="bg-gray-800 border-b border-gray-700 px-4 py-2 flex items-center justify-between flex-shrink-0">
                <div className="flex items-center gap-3">
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-medium text-gray-200">{selectedTableName}</span>
                    <span className="text-xs text-gray-500">({selectedApp?.slug})</span>
                  </div>
                  <button
                    onClick={() => setShowAddModal(true)}
                    className="flex items-center gap-1.5 px-3 py-1.5 text-sm bg-blue-600 hover:bg-blue-500 text-white rounded transition-colors"
                  >
                    <Plus className="w-4 h-4" />
                    Ajouter
                  </button>
                  <div className="flex items-center gap-2">
                    <span className="text-xs text-gray-500">Lignes/page :</span>
                    <select
                      value={rowsPerPage}
                      onChange={e => changeRowsPerPage(Number(e.target.value))}
                      className="bg-gray-700 border border-gray-600 text-gray-300 text-sm rounded px-2 py-1 focus:outline-none"
                    >
                      {ROWS_PER_PAGE_OPTIONS.map(n => (
                        <option key={n} value={n}>{n}</option>
                      ))}
                    </select>
                  </div>
                </div>
                <div className="text-sm text-gray-400">
                  Page {page} / {totalPages}
                </div>
              </div>

              {/* Data table */}
              <div className="flex-1 min-h-0 overflow-auto">
                {loadingRows ? (
                  <div className="flex items-center justify-center py-12">
                    <Loader2 className="w-6 h-6 text-gray-500 animate-spin" />
                  </div>
                ) : rows.length === 0 ? (
                  <div className="flex items-center justify-center py-12">
                    <div className="text-center">
                      <Table2 className="w-10 h-10 text-gray-600 mx-auto mb-2" />
                      <p className="text-gray-500 text-sm">Aucune donnee dans cette table.</p>
                    </div>
                  </div>
                ) : (
                  <table className="w-full text-sm">
                    <thead className="sticky top-0 z-10">
                      <tr className="bg-gray-800 border-b border-gray-700">
                        {allColumns.map(col => (
                          <th
                            key={col}
                            onClick={() => handleSort(col)}
                            className="px-3 py-2 text-left text-xs font-semibold text-gray-400 uppercase tracking-wider cursor-pointer hover:text-gray-200 select-none whitespace-nowrap"
                          >
                            <div className="flex items-center gap-1">
                              {col}
                              {orderBy === col && (
                                orderDesc
                                  ? <ChevronDown className="w-3 h-3 text-blue-400" />
                                  : <ChevronUp className="w-3 h-3 text-blue-400" />
                              )}
                            </div>
                          </th>
                        ))}
                        <th className="px-3 py-2 w-10"></th>
                      </tr>
                    </thead>
                    <tbody>
                      {rows.map((row, idx) => (
                        <tr
                          key={row.id ?? idx}
                          className={`border-b border-gray-700/50 transition-colors ${
                            idx % 2 === 0 ? 'bg-gray-900/40' : 'bg-gray-900/20'
                          } hover:bg-gray-700/30`}
                        >
                          {allColumns.map(col => {
                            const isSystem = SYSTEM_COLUMNS.includes(col);
                            const isEditing = editingCell?.rowId === row.id && editingCell?.column === col;
                            const colInfo = getColumnInfo(col);

                            if (isEditing && !isSystem) {
                              return (
                                <td key={col} className="px-3 py-1">
                                  <InlineEdit
                                    value={row[col]}
                                    fieldType={colInfo?.field_type || 'text'}
                                    choices={colInfo?.choices}
                                    onSave={(val) => handleInlineSave(row.id, col, val)}
                                    onCancel={() => setEditingCell(null)}
                                  />
                                </td>
                              );
                            }

                            return (
                              <td
                                key={col}
                                className={`px-3 py-2 text-gray-300 whitespace-nowrap max-w-[300px] truncate ${
                                  !isSystem ? 'cursor-pointer hover:bg-gray-700/40' : ''
                                }`}
                                onClick={() => {
                                  if (!isSystem) setEditingCell({ rowId: row.id, column: col });
                                }}
                                title={row[col] != null ? String(row[col]) : ''}
                              >
                                {formatCellValue(row[col])}
                              </td>
                            );
                          })}
                          <td className="px-2 py-2">
                            <button
                              onClick={() => setDeleteRowId(row.id)}
                              className="text-gray-600 hover:text-red-400 transition-colors"
                              title="Supprimer"
                            >
                              <Trash2 className="w-4 h-4" />
                            </button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                )}
              </div>

              {/* Pagination */}
              {totalPages > 1 && (
                <div className="bg-gray-800 border-t border-gray-700 px-4 py-2 flex-shrink-0">
                  {renderPagination()}
                </div>
              )}
            </>
          )}
        </div>
      </div>

      {/* Modals */}
      {showAddModal && (
        <AddRowModal
          columns={columns}
          onClose={() => setShowAddModal(false)}
          onAdd={handleAddRow}
        />
      )}
      {deleteRowId != null && (
        <DeleteConfirm
          onConfirm={handleDeleteRow}
          onCancel={() => setDeleteRowId(null)}
        />
      )}
    </div>
  );
}

export default DataBrowser;
