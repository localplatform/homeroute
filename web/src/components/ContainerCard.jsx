import {
  Wifi,
  WifiOff,
  Clock,
  Loader2,
  Play,
  Square,
  Terminal,
  ArrowRightLeft,
  Code2,
  Key,
  Shield,
  AlertTriangle,
  HardDrive,
  ExternalLink,
} from 'lucide-react';

// Shared grid template — used by ContainerCard rows and column header in Containers.jsx
// 10 columns: Env | Status | URL | Auth | Local | IDE | CPU | RAM | Host | Actions
export const CONTAINER_GRID = '50px 1.2fr 1fr 26px 26px 48px 0.6fr 0.7fr 1fr 140px';

const STATUS_BADGES = {
  connected: { color: 'text-green-400 bg-green-900/30', icon: Wifi, label: 'Connecte' },
  deploying: { color: 'text-blue-400 bg-blue-900/30', icon: Loader2, label: 'Deploiement', spin: true },
  pending: { color: 'text-yellow-400 bg-yellow-900/30', icon: Clock, label: 'En attente' },
  running: { color: 'text-yellow-400 bg-yellow-900/30', icon: Clock, label: 'En attente' },
  stopped: { color: 'text-gray-400 bg-gray-900/30', icon: Square, label: 'Arrete' },
  disconnected: { color: 'text-red-400 bg-red-900/30', icon: WifiOff, label: 'Deconnecte' },
  error: { color: 'text-red-400 bg-red-900/30', icon: AlertTriangle, label: 'Erreur' },
};

function StatusBadge({ status }) {
  const badge = STATUS_BADGES[status] || STATUS_BADGES.disconnected;
  const Icon = badge.icon;
  return (
    <span className={`flex items-center gap-1 text-xs px-2 py-0.5 ${badge.color}`}>
      <Icon className={`w-3 h-3 ${badge.spin ? 'animate-spin' : ''}`} />
      {badge.label}
    </span>
  );
}

function formatBytes(bytes) {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return parseFloat((bytes / Math.pow(k, i)).toFixed(0)) + ' ' + sizes[i];
}

function ContainerCard({
  container,
  baseDomain,
  metrics,
  migration,
  hosts,
  isHostOffline,
  onStart,
  onStop,
  onTerminal,
  onToggleSecurity,
  onMigrate,
  onMigrationDismiss,
  MigrationProgress,
}) {
  const displayStatus = container.agent_status || container.status;
  const isDeploying = displayStatus === 'deploying' || container.status === 'deploying';
  const isMigrating = !!migration;
  const isDev = container.environment !== 'production';
  const host = container.host_id && container.host_id !== 'local'
    ? hosts?.find(h => h.id === container.host_id)
    : null;

  const appUrl = baseDomain
    ? isDev
      ? `code.${container.slug}.${baseDomain}`
      : `${container.slug}.${baseDomain}`
    : null;

  const ideUrl = baseDomain && isDev && container.code_server_enabled
    ? `code.${container.slug}.${baseDomain}/?folder=/root/workspace`
    : null;

  const isConnected = displayStatus === 'connected';

  return (
    <div className={isHostOffline ? 'opacity-60' : ''}>
      {/* Main row — grid aligned with column headers */}
      <div
        className="grid items-center gap-x-3 px-4 py-1.5 border-b border-gray-700/30 transition-[background-color] duration-500 ease-out hover:bg-gray-600/30 hover:duration-0"
        style={{ gridTemplateColumns: CONTAINER_GRID }}
      >
        {/* Env */}
        <span className={`text-xs px-1.5 py-0.5 font-medium text-center ${
          isDev ? 'bg-blue-100 text-blue-800' : 'bg-purple-100 text-purple-800'
        }`}>
          {isDev ? 'DEV' : 'PROD'}
        </span>

        {/* Status */}
        <StatusBadge status={displayStatus} />

        {/* URL */}
        <div className="flex items-center gap-1 min-w-0 overflow-hidden">
          {appUrl && (
            <a
              href={`https://${appUrl}`}
              target="_blank"
              rel="noopener noreferrer"
              className="font-mono text-xs text-gray-400 hover:text-blue-400 truncate flex items-center gap-1"
            >
              {appUrl}
              <ExternalLink className="w-3 h-3 shrink-0" />
            </a>
          )}
          {isDeploying && container._deployMessage && (
            <span className="text-xs text-gray-500 truncate">{container._deployMessage}</span>
          )}
        </div>

        {/* Auth toggle */}
        <button
          onClick={() => onToggleSecurity(container.id, 'auth_required', !container.frontend?.auth_required)}
          className={`p-0.5 justify-self-center transition-colors ${container.frontend?.auth_required ? 'text-purple-400 hover:text-purple-300' : 'text-purple-400 opacity-30 hover:opacity-60'}`}
          title={container.frontend?.auth_required ? 'Auth requis (cliquer pour desactiver)' : 'Auth non requis (cliquer pour activer)'}
        >
          <Key className="w-3 h-3" />
        </button>

        {/* Local-only toggle */}
        <button
          onClick={() => onToggleSecurity(container.id, 'local_only', !container.frontend?.local_only)}
          className={`p-0.5 justify-self-center transition-colors ${container.frontend?.local_only ? 'text-yellow-400 hover:text-yellow-300' : 'text-yellow-400 opacity-30 hover:opacity-60'}`}
          title={container.frontend?.local_only ? 'Local uniquement (cliquer pour desactiver)' : 'Acces externe (cliquer pour restreindre au local)'}
        >
          <Shield className="w-3 h-3" />
        </button>

        {/* IDE link */}
        <div className="justify-self-center">
          {ideUrl ? (
            <a
              href={`https://${ideUrl}`}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 px-1 py-0.5 text-xs text-cyan-400 hover:text-cyan-300 bg-cyan-900/20"
            >
              <Code2 className="w-3 h-3" />
              IDE
            </a>
          ) : null}
        </div>

        {/* CPU */}
        <span className={`font-mono text-xs text-right ${
          isConnected && metrics?.cpuPercent > 80 ? 'text-red-400' :
          isConnected && metrics?.cpuPercent > 50 ? 'text-yellow-400' :
          isConnected && metrics?.cpuPercent > 0 ? 'text-green-400' : 'text-gray-600'
        }`}>
          {isConnected && metrics?.cpuPercent !== undefined
            ? `${metrics.cpuPercent.toFixed(1)}%`
            : '—'}
        </span>

        {/* RAM */}
        <span className="font-mono text-xs text-gray-400 text-right">
          {isConnected && metrics?.memoryBytes
            ? formatBytes(metrics.memoryBytes)
            : '—'}
        </span>

        {/* Host */}
        <span className="flex items-center gap-1 text-xs text-gray-400 truncate">
          <HardDrive className="w-3 h-3 shrink-0" />
          {host ? host.name : 'Local'}
        </span>

        {/* Actions */}
        <div className={`flex items-center gap-0.5 justify-end ${isMigrating || isHostOffline ? 'opacity-50 pointer-events-none' : ''}`}>
          {isConnected ? (
            <button
              onClick={() => onStop(container.id)}
              className="p-1 text-yellow-400 hover:text-yellow-300 hover:bg-yellow-900/30 transition-colors"
              title="Arreter"
            >
              <Square className="w-3.5 h-3.5" />
            </button>
          ) : displayStatus !== 'deploying' ? (
            <button
              onClick={() => onStart(container.id)}
              className="p-1 text-green-400 hover:text-green-300 hover:bg-green-900/30 transition-colors"
              title="Demarrer"
            >
              <Play className="w-3.5 h-3.5" />
            </button>
          ) : null}
          <button
            onClick={() => onTerminal(container)}
            disabled={isMigrating}
            className="p-1 text-emerald-400 hover:text-emerald-300 hover:bg-emerald-900/30 transition-colors"
            title="Terminal"
          >
            <Terminal className="w-3.5 h-3.5" />
          </button>
          <button
            onClick={() => onMigrate(container)}
            disabled={isMigrating}
            className="p-1 text-gray-400 hover:text-blue-400 hover:bg-gray-700 transition-colors"
            title="Migrer"
          >
            <ArrowRightLeft className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* Migration progress (sub-row) */}
      {isMigrating && MigrationProgress && (
        <MigrationProgress
          appId={container.id}
          migration={migration}
          onDismiss={onMigrationDismiss}
        />
      )}
    </div>
  );
}

export default ContainerCard;
