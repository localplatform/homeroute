import { useState, useEffect, useRef } from 'react';
import { HardDrive, Play, Settings, History, Plus, Trash2, CheckCircle, XCircle, Clock, Square, AlertCircle, Power, PowerOff, Radio, Server, Folder, RefreshCw, File, ChevronRight, ArrowLeft } from 'lucide-react';
import { io } from 'socket.io-client';
import Card from '../components/Card';
import Button from '../components/Button';
import ConfirmModal from '../components/ConfirmModal';
import {
  getBackupConfig,
  saveBackupConfig,
  runBackup,
  getBackupHistory,
  cancelBackup,
  getBackupStatus,
  wakeBackupServer,
  getBackupServerStatus,
  getRemoteBackups,
  deleteRemoteItem,
  shutdownBackupServer
} from '../api/client';

function Backup() {
  const [config, setConfig] = useState(null);
  const [history, setHistory] = useState([]);
  const [sources, setSources] = useState([]);
  const [newSource, setNewSource] = useState('');
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [saving, setSaving] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [message, setMessage] = useState(null);
  const [progress, setProgress] = useState(null);
  const socketRef = useRef(null);

  // Wake-on-LAN states
  const [serverStatus, setServerStatus] = useState(null); // { online: boolean, pingMs: number }
  const [checkingServer, setCheckingServer] = useState(false);
  const [waking, setWaking] = useState(false);
  const [shuttingDown, setShuttingDown] = useState(false);
  const [wolMacAddress, setWolMacAddress] = useState('');
  const [wolStatus, setWolStatus] = useState(null); // 'wol-sent', 'ping-waiting', 'ping-ok', 'smb-waiting', 'ready'
  const [wolProgress, setWolProgress] = useState(null); // { attempt, elapsed }

  // Remote backups / File explorer
  const [remoteItems, setRemoteItems] = useState([]);
  const [remotePath, setRemotePath] = useState('');
  const [loadingRemote, setLoadingRemote] = useState(false);
  const [deletingItem, setDeletingItem] = useState(null);

  // Confirmation modals
  const [confirmModal, setConfirmModal] = useState({ type: null, data: null });

  // WebSocket connection
  useEffect(() => {
    const socket = io(window.location.origin);
    socketRef.current = socket;

    socket.on('backup:started', (data) => {
      setRunning(true);
      setProgress({
        status: 'started',
        sourcesCount: data.sourcesCount,
        sources: data.sources,
        currentSource: null,
        percent: 0,
        speed: null
      });
    });

    socket.on('backup:source-start', (data) => {
      setProgress(prev => ({
        ...prev,
        status: 'syncing',
        currentSource: data.sourceName,
        sourceIndex: data.sourceIndex,
        sourcesCount: data.sourcesCount,
        percent: 0,
        speed: null
      }));
    });

    socket.on('backup:progress', (data) => {
      setProgress(prev => ({
        ...prev,
        status: 'syncing',
        currentSource: data.sourceName,
        sourceIndex: data.sourceIndex,
        sourcesCount: data.sourcesCount,
        percent: data.percent,
        speed: data.speed,
        transferredBytes: data.transferredBytes
      }));
    });

    socket.on('backup:source-complete', (data) => {
      setProgress(prev => ({
        ...prev,
        status: 'source-complete',
        percent: 100
      }));
    });

    socket.on('backup:complete', (data) => {
      setRunning(false);
      setProgress(null);
      fetchData();
      if (data.cancelled) {
        setMessage({ type: 'warning', text: 'Backup annulé' });
      } else if (data.success) {
        setMessage({
          type: 'success',
          text: `Backup terminé: ${data.totalFiles} fichiers, ${formatSize(data.totalSize)}`
        });
      } else {
        setMessage({ type: 'error', text: 'Backup terminé avec des erreurs' });
      }
    });

    socket.on('backup:cancelled', () => {
      setRunning(false);
      setProgress(null);
      setCancelling(false);
      setMessage({ type: 'warning', text: 'Backup annulé' });
      fetchData();
    });

    socket.on('backup:error', (data) => {
      setRunning(false);
      setProgress(null);
      setWolStatus(null);
      setWolProgress(null);
      setMessage({ type: 'error', text: data.error });
      fetchData();
    });

    // Wake-on-LAN events
    socket.on('backup:wol-sent', (data) => {
      setWolStatus('wol-sent');
      setWolProgress(null);
    });

    socket.on('backup:wol-ping-waiting', (data) => {
      setWolStatus('ping-waiting');
      setWolProgress({ attempt: data.attempt, elapsed: data.elapsed });
    });

    socket.on('backup:wol-ping-ok', (data) => {
      setWolStatus('ping-ok');
      setWolProgress(null);
    });

    socket.on('backup:wol-smb-waiting', () => {
      setWolStatus('smb-waiting');
    });

    socket.on('backup:wol-ready', () => {
      setWolStatus('ready');
      checkServerStatus(); // Refresh server status
    });

    return () => {
      socket.disconnect();
    };
  }, []);

  // Initial data fetch + check if backup is running
  useEffect(() => {
    fetchData();
    checkBackupStatus();
    checkServerStatus();
  }, []);

  // Auto-fetch remote backups when server comes online
  useEffect(() => {
    if (serverStatus?.online && serverStatus?.smbOk) {
      fetchRemoteBackups('');
    }
  }, [serverStatus]);

  async function checkBackupStatus() {
    try {
      const res = await getBackupStatus();
      if (res.data.running) {
        setRunning(true);
        setProgress({ status: 'syncing', percent: 0 });
      }
    } catch (error) {
      console.error('Error checking backup status:', error);
    }
  }

  async function checkServerStatus() {
    setCheckingServer(true);
    try {
      const res = await getBackupServerStatus();
      setServerStatus(res.data);
    } catch (error) {
      console.error('Error checking server status:', error);
      setServerStatus({ online: false, pingMs: null });
    } finally {
      setCheckingServer(false);
    }
  }

  async function fetchData() {
    try {
      const [configRes, historyRes] = await Promise.all([
        getBackupConfig(),
        getBackupHistory()
      ]);

      if (configRes.data.success) {
        setConfig(configRes.data.config);
        setSources(configRes.data.config.sources || []);
        setWolMacAddress(configRes.data.config.wolMacAddress || '');
      }
      if (historyRes.data.success) {
        setHistory(historyRes.data.history);
      }
    } catch (error) {
      console.error('Error:', error);
      setMessage({ type: 'error', text: 'Erreur de chargement' });
    } finally {
      setLoading(false);
    }
  }

  async function handleRunBackup() {
    setRunning(true);
    setMessage(null);
    setProgress({ status: 'starting', percent: 0 });
    setWolStatus(null);
    setWolProgress(null);
    try {
      // Fire and forget - progress comes via WebSocket
      runBackup().catch(error => {
        // Only handle network errors, backup errors come via WebSocket
        if (!error.response) {
          setMessage({ type: 'error', text: 'Erreur réseau' });
          setRunning(false);
          setProgress(null);
          setWolStatus(null);
          setWolProgress(null);
        }
      });
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur de lancement' });
      setRunning(false);
      setProgress(null);
      setWolStatus(null);
      setWolProgress(null);
    }
  }

  async function handleWakeServer() {
    if (!wolMacAddress) {
      setMessage({ type: 'error', text: 'Adresse MAC non configurée' });
      return;
    }
    setWaking(true);
    setMessage(null);
    try {
      const res = await wakeBackupServer();
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Magic packet WOL envoyé' });
        // Check server status after a delay
        setTimeout(checkServerStatus, 5000);
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur WOL' });
    } finally {
      setWaking(false);
    }
  }

  function openShutdownModal() {
    setConfirmModal({ type: 'shutdown', data: null });
  }

  async function handleShutdown() {
    setConfirmModal({ type: null, data: null });
    setShuttingDown(true);
    setMessage(null);
    try {
      const res = await shutdownBackupServer();
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Commande d\'arrêt envoyée' });
        // Rafraîchir le statut après quelques secondes
        setTimeout(checkServerStatus, 5000);
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur lors de l\'arrêt' });
    } finally {
      setShuttingDown(false);
    }
  }

  async function fetchRemoteBackups(path = '') {
    if (!serverStatus?.online || !serverStatus?.smbOk) return;
    setLoadingRemote(true);
    try {
      const res = await getRemoteBackups(path);
      if (res.data.success) {
        setRemoteItems(res.data.items);
        setRemotePath(res.data.currentPath || '');
      }
    } catch (error) {
      console.error('Error fetching remote backups:', error);
    } finally {
      setLoadingRemote(false);
    }
  }

  function navigateToFolder(folderName) {
    const newPath = remotePath ? `${remotePath}/${folderName}` : folderName;
    fetchRemoteBackups(newPath);
  }

  function navigateUp() {
    const parts = remotePath.split('/').filter(Boolean);
    parts.pop();
    const newPath = parts.join('/');
    fetchRemoteBackups(newPath);
  }

  function navigateToBreadcrumb(index) {
    const parts = remotePath.split('/').filter(Boolean);
    const newPath = parts.slice(0, index + 1).join('/');
    fetchRemoteBackups(newPath);
  }

  function openDeleteModal(itemName) {
    setConfirmModal({ type: 'delete', data: { name: itemName } });
  }

  async function handleDeleteItem() {
    const itemName = confirmModal.data?.name;
    if (!itemName) return;

    const fullPath = remotePath ? `${remotePath}/${itemName}` : itemName;
    setConfirmModal({ type: null, data: null });
    setDeletingItem(itemName);
    try {
      const res = await deleteRemoteItem(fullPath);
      if (res.data.success) {
        setMessage({ type: 'success', text: `"${itemName}" supprimé` });
        fetchRemoteBackups(remotePath);
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || 'Erreur de suppression' });
    } finally {
      setDeletingItem(null);
    }
  }

  async function handleCancelBackup() {
    setCancelling(true);
    try {
      await cancelBackup();
      // WebSocket will handle the rest
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur d\'annulation' });
      setCancelling(false);
    }
  }

  async function handleSaveConfig() {
    setSaving(true);
    try {
      const res = await saveBackupConfig({
        sources,
        wolMacAddress: wolMacAddress.trim()
      });
      if (res.data.success) {
        setMessage({ type: 'success', text: 'Configuration sauvegardée' });
      } else {
        setMessage({ type: 'error', text: res.data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: 'Erreur de sauvegarde' });
    } finally {
      setSaving(false);
    }
  }

  function handleAddSource() {
    if (!newSource.trim()) return;
    if (sources.includes(newSource.trim())) {
      setMessage({ type: 'error', text: 'Chemin déjà dans la liste' });
      return;
    }
    setSources([...sources, newSource.trim()]);
    setNewSource('');
  }

  function handleRemoveSource(path) {
    setSources(sources.filter(s => s !== path));
  }

  function formatSize(bytes) {
    if (!bytes) return '-';
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
    return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
  }

  function formatDuration(ms) {
    if (!ms) return '-';
    if (ms < 1000) return ms + 'ms';
    if (ms < 60000) return (ms / 1000).toFixed(1) + 's';
    return (ms / 60000).toFixed(1) + ' min';
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  const smbConfigured = config?.smbServer && config?.smbShare;

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-4">
          <h1 className="text-2xl font-bold">Backup SMB</h1>
          {/* Server status indicator */}
          {smbConfigured && (
            <div className="flex items-center gap-2">
              {checkingServer ? (
                <div className="flex items-center gap-2 text-gray-400 text-sm">
                  <div className="animate-spin rounded-full h-3 w-3 border border-gray-400 border-t-transparent"></div>
                  Vérification...
                </div>
              ) : serverStatus?.online ? (
                <div className="flex items-center gap-2 text-sm">
                  <div className="flex items-center gap-1.5 text-green-400">
                    <div className="w-2 h-2 bg-green-400 rounded-full"></div>
                    Ping OK ({serverStatus.pingMs}ms)
                  </div>
                  <span className="text-gray-600">|</span>
                  {serverStatus.smbOk ? (
                    <span className="text-green-400">SMB OK</span>
                  ) : (
                    <span className="text-red-400">SMB erreur</span>
                  )}
                </div>
              ) : (
                <div className="flex items-center gap-1.5 text-red-400 text-sm">
                  <div className="w-2 h-2 bg-red-400 rounded-full"></div>
                  Hors ligne
                </div>
              )}
              <button
                onClick={checkServerStatus}
                disabled={checkingServer}
                className="text-gray-500 hover:text-gray-300 p-1"
                title="Rafraîchir"
              >
                <Radio className="w-3.5 h-3.5" />
              </button>
            </div>
          )}
        </div>
        <div className="flex gap-2">
          {/* Dynamic Wake/Shutdown button - only show after status check is complete */}
          {smbConfigured && wolMacAddress && serverStatus !== null && (
            serverStatus?.online ? (
              <Button onClick={openShutdownModal} loading={shuttingDown} variant="danger">
                <PowerOff className="w-4 h-4" />
                Arrêter
              </Button>
            ) : (
              <Button onClick={handleWakeServer} loading={waking} variant="warning">
                <Power className="w-4 h-4" />
                Réveiller
              </Button>
            )
          )}
          <Button
            onClick={handleRunBackup}
            loading={running}
            variant="success"
            disabled={!smbConfigured || sources.length === 0}
          >
            <Play className="w-4 h-4" />
            Lancer backup
          </Button>
        </div>
      </div>

      {message && (
        <div className={`p-4 rounded-lg flex items-center gap-2 ${
          message.type === 'success' ? 'bg-green-900/50 text-green-400' :
          message.type === 'warning' ? 'bg-yellow-900/50 text-yellow-400' :
          'bg-red-900/50 text-red-400'
        }`}>
          {message.type === 'success' ? <CheckCircle className="w-5 h-5" /> :
           message.type === 'warning' ? <AlertCircle className="w-5 h-5" /> :
           <XCircle className="w-5 h-5" />}
          {message.text}
        </div>
      )}

      {/* Progress bar when backup is running */}
      {running && progress && (
        <div className="bg-gray-800 border border-gray-700 rounded-lg p-4 space-y-3">
          {/* WOL Status - shown during wake-on-lan phase */}
          {wolStatus && wolStatus !== 'ready' && (
            <div className="flex items-center gap-3 p-3 bg-gray-900/50 rounded-lg mb-2">
              <div className="animate-spin rounded-full h-4 w-4 border-2 border-yellow-400 border-t-transparent"></div>
              <div className="flex-1">
                <div className="font-medium text-yellow-400">
                  {wolStatus === 'wol-sent' && 'Magic packet envoyé, réveil en cours...'}
                  {wolStatus === 'ping-waiting' && (
                    <>
                      Attente réponse ping...
                      {wolProgress && (
                        <span className="text-gray-400 font-normal ml-2">
                          (tentative {wolProgress.attempt}, {wolProgress.elapsed}s)
                        </span>
                      )}
                    </>
                  )}
                  {wolStatus === 'ping-ok' && 'Serveur en ligne!'}
                  {wolStatus === 'smb-waiting' && 'Vérification accès SMB...'}
                </div>
              </div>
              {(wolStatus === 'ping-ok' || wolStatus === 'smb-waiting') && (
                <CheckCircle className="w-5 h-5 text-green-400" />
              )}
            </div>
          )}

          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <div className="animate-spin rounded-full h-4 w-4 border-2 border-blue-400 border-t-transparent"></div>
              <span className="font-medium">
                {progress.currentSource
                  ? `Backup: ${progress.currentSource} (${(progress.sourceIndex ?? 0) + 1}/${progress.sourcesCount || '?'})`
                  : wolStatus && wolStatus !== 'ready'
                    ? 'Réveil du serveur...'
                    : 'Démarrage du backup...'}
              </span>
            </div>
            <div className="flex items-center gap-3">
              {progress.speed && (
                <span className="text-sm text-gray-400">{progress.speed}</span>
              )}
              <span className="text-sm font-mono text-blue-400">{progress.percent || 0}%</span>
            </div>
          </div>

          {/* Progress bar */}
          <div className="h-2 bg-gray-700 rounded-full overflow-hidden">
            <div
              className="h-full bg-blue-500 transition-all duration-150"
              style={{ width: `${progress.percent || 0}%` }}
            />
          </div>

          {/* Cancel button */}
          <div className="flex justify-end">
            <Button
              onClick={handleCancelBackup}
              loading={cancelling}
              variant="danger"
              className="text-sm"
            >
              <Square className="w-3 h-3" />
              Annuler
            </Button>
          </div>
        </div>
      )}

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        <Card title="Configuration SMB" icon={HardDrive}>
          <div className="space-y-3 text-sm">
            <div className="flex justify-between">
              <span className="text-gray-400">Serveur</span>
              <span className="font-mono">{config?.smbServer || 'Non configuré'}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-400">Partage</span>
              <span className="font-mono">{config?.smbShare || '-'}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-400">Utilisateur</span>
              <span className="font-mono">{config?.smbUsername || '-'}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-400">Mot de passe</span>
              <span className={config?.smbPasswordSet ? 'text-green-400' : 'text-yellow-400'}>
                {config?.smbPasswordSet ? 'Configuré' : 'Non configuré'}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-400">Point de montage</span>
              <span className="font-mono text-xs">{config?.mountPoint || '-'}</span>
            </div>
            <p className="text-xs text-gray-500 mt-4 pt-3 border-t border-gray-700">
              Configuration via fichier .env
            </p>
          </div>
        </Card>

        <Card
          title="Dossiers à sauvegarder"
          icon={Settings}
          actions={
            <Button onClick={handleSaveConfig} loading={saving} variant="primary" className="text-sm">
              Sauvegarder
            </Button>
          }
        >
          <div className="space-y-4">
            <div className="flex gap-2">
              <input
                type="text"
                placeholder="/chemin/vers/dossier"
                value={newSource}
                onChange={e => setNewSource(e.target.value)}
                onKeyDown={e => e.key === 'Enter' && handleAddSource()}
                className="flex-1 px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm focus:outline-none focus:border-blue-500"
              />
              <Button onClick={handleAddSource}>
                <Plus className="w-4 h-4" />
              </Button>
            </div>

            <div className="space-y-2 max-h-32 overflow-y-auto">
              {sources.length === 0 ? (
                <p className="text-gray-500 text-sm text-center py-4">Aucun dossier configuré</p>
              ) : (
                sources.map(source => (
                  <div
                    key={source}
                    className="flex items-center justify-between bg-gray-900 rounded px-3 py-2"
                  >
                    <span className="font-mono text-sm truncate">{source}</span>
                    <button
                      onClick={() => handleRemoveSource(source)}
                      className="text-red-400 hover:text-red-300 ml-2"
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                ))
              )}
            </div>

            {/* Wake-on-LAN Configuration */}
            <div className="pt-3 mt-3 border-t border-gray-700 space-y-3">
              <h4 className="text-sm font-medium text-gray-300 flex items-center gap-2">
                <Power className="w-4 h-4" />
                Wake-on-LAN
              </h4>
              <div>
                <label className="text-xs text-gray-500 block mb-1">Adresse MAC du serveur</label>
                <input
                  type="text"
                  placeholder="AA:BB:CC:DD:EE:FF"
                  value={wolMacAddress}
                  onChange={e => setWolMacAddress(e.target.value)}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 rounded text-sm font-mono focus:outline-none focus:border-blue-500"
                />
              </div>
              <p className="text-xs text-gray-500">
                Si configuré, le serveur sera réveillé automatiquement avant le backup
              </p>
            </div>

            <p className="text-xs text-gray-500 pt-2">
              Utilise rsync --delete (miroir exact)
            </p>
          </div>
        </Card>

        {/* Remote backups file explorer */}
        <Card
          title="Fichiers distants"
          icon={Server}
          actions={
            serverStatus?.online && serverStatus?.smbOk && (
              <button
                onClick={() => fetchRemoteBackups(remotePath)}
                disabled={loadingRemote}
                className="text-gray-400 hover:text-gray-200 p-1"
                title="Rafraîchir"
              >
                <RefreshCw className={`w-4 h-4 ${loadingRemote ? 'animate-spin' : ''}`} />
              </button>
            )
          }
        >
          {!serverStatus?.online || !serverStatus?.smbOk ? (
            <div className="text-center py-8 text-gray-500">
              <Server className="w-8 h-8 mx-auto mb-2 opacity-50" />
              <p className="text-sm">Serveur hors ligne</p>
            </div>
          ) : (
            <>
              {/* Breadcrumb navigation */}
              <div className="flex items-center gap-1 text-sm mb-3 flex-wrap">
                <button
                  onClick={() => fetchRemoteBackups('')}
                  className="text-blue-400 hover:text-blue-300"
                >
                  Racine
                </button>
                {remotePath && remotePath.split('/').filter(Boolean).map((part, index) => (
                  <span key={index} className="flex items-center gap-1">
                    <ChevronRight className="w-3 h-3 text-gray-500" />
                    <button
                      onClick={() => navigateToBreadcrumb(index)}
                      className="text-blue-400 hover:text-blue-300"
                    >
                      {part}
                    </button>
                  </span>
                ))}
              </div>

              {/* Back button when in subfolder */}
              {remotePath && (
                <button
                  onClick={navigateUp}
                  className="flex items-center gap-2 text-gray-400 hover:text-gray-200 mb-2 text-sm"
                >
                  <ArrowLeft className="w-4 h-4" />
                  Retour
                </button>
              )}

              {/* Loading state */}
              {loadingRemote && remoteItems.length === 0 ? (
                <div className="flex items-center justify-center py-8">
                  <div className="animate-spin rounded-full h-6 w-6 border-2 border-blue-400 border-t-transparent"></div>
                </div>
              ) : remoteItems.length === 0 ? (
                <p className="text-gray-500 text-sm text-center py-4">Dossier vide</p>
              ) : (
                <div className="space-y-1 max-h-64 overflow-y-auto">
                  {remoteItems.map(item => (
                    <div
                      key={item.name}
                      className="flex items-center justify-between bg-gray-900 rounded px-3 py-2 group hover:bg-gray-800"
                    >
                      <div
                        className={`flex items-center gap-2 flex-1 min-w-0 ${item.type === 'directory' ? 'cursor-pointer' : ''}`}
                        onClick={() => item.type === 'directory' && navigateToFolder(item.name)}
                      >
                        {item.type === 'directory' ? (
                          <Folder className="w-4 h-4 text-blue-400 flex-shrink-0" />
                        ) : (
                          <File className="w-4 h-4 text-gray-400 flex-shrink-0" />
                        )}
                        <span className="font-mono text-sm truncate">{item.name}</span>
                      </div>
                      <div className="flex items-center gap-2 text-sm text-gray-400 flex-shrink-0">
                        <span className="text-xs hidden xl:inline">{formatSize(item.size)}</span>
                        <button
                          onClick={() => openDeleteModal(item.name)}
                          disabled={deletingItem === item.name}
                          className="text-red-400 hover:text-red-300 opacity-0 group-hover:opacity-100 transition-opacity p-1"
                          title="Supprimer"
                        >
                          {deletingItem === item.name ? (
                            <div className="animate-spin rounded-full h-4 w-4 border border-red-400 border-t-transparent"></div>
                          ) : (
                            <Trash2 className="w-4 h-4" />
                          )}
                        </button>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </>
          )}
        </Card>
      </div>

      <Card title="Historique des backups" icon={History}>
        <div className="overflow-x-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-gray-400 border-b border-gray-700">
                <th className="pb-2">Date</th>
                <th className="pb-2">Durée</th>
                <th className="pb-2">Fichiers</th>
                <th className="pb-2">Transféré</th>
                <th className="pb-2">Status</th>
              </tr>
            </thead>
            <tbody>
              {history.length === 0 ? (
                <tr>
                  <td colSpan={5} className="text-center py-4 text-gray-500">
                    Aucun backup effectué
                  </td>
                </tr>
              ) : (
                history.slice(0, 10).map((entry, i) => (
                  <tr key={i} className="border-b border-gray-700/50">
                    <td className="py-2">
                      <div className="flex items-center gap-2">
                        <Clock className="w-4 h-4 text-gray-500" />
                        {new Date(entry.timestamp).toLocaleString('fr-FR')}
                      </div>
                    </td>
                    <td className="py-2">{formatDuration(entry.duration)}</td>
                    <td className="py-2">{entry.filesTransferred ?? '-'}</td>
                    <td className="py-2">{formatSize(entry.transferredSize)}</td>
                    <td className="py-2">
                      {entry.status === 'success' ? (
                        <span className="text-green-400 flex items-center gap-1">
                          <CheckCircle className="w-4 h-4" /> OK
                        </span>
                      ) : entry.status === 'partial' ? (
                        <span className="text-yellow-400 flex items-center gap-1">
                          <AlertCircle className="w-4 h-4" /> Partiel
                        </span>
                      ) : entry.status === 'cancelled' ? (
                        <span className="text-gray-400 flex items-center gap-1">
                          <Square className="w-4 h-4" /> Annulé
                        </span>
                      ) : (
                        <span className="text-red-400 flex items-center gap-1" title={entry.error}>
                          <XCircle className="w-4 h-4" /> Erreur
                        </span>
                      )}
                    </td>
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
      </Card>

      {/* Confirmation Modals */}
      <ConfirmModal
        isOpen={confirmModal.type === 'delete'}
        onClose={() => setConfirmModal({ type: null, data: null })}
        onConfirm={handleDeleteItem}
        title="Supprimer cet élément ?"
        message={`Voulez-vous vraiment supprimer "${confirmModal.data?.name}" ? Cette action est irréversible.`}
        confirmText="Supprimer"
        variant="danger"
        loading={deletingItem !== null}
      />

      <ConfirmModal
        isOpen={confirmModal.type === 'shutdown'}
        onClose={() => setConfirmModal({ type: null, data: null })}
        onConfirm={handleShutdown}
        title="Arrêter le serveur ?"
        message="Voulez-vous vraiment arrêter le serveur distant ? Vous devrez le réveiller manuellement pour accéder aux sauvegardes."
        confirmText="Arrêter"
        variant="warning"
        loading={shuttingDown}
      />
    </div>
  );
}

export default Backup;
