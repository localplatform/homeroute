import { Container, Plus, Pencil, Trash2 } from 'lucide-react';
import ContainerCard, { CONTAINER_GRID } from './ContainerCard';

function AppGroupCard({
  group,
  baseDomain,
  appMetrics,
  migrations,
  hosts,
  onStart,
  onStop,
  onTerminal,
  onEditApp,
  onToggleSecurity,
  onMigrate,
  onDeleteApp,
  onMigrationDismiss,
  onCreatePaired,
  MigrationProgress,
}) {
  const { slug, name, dev, prod } = group;

  return (
    <div className="bg-gray-800 border-b border-gray-700 transition-[background-color] duration-700 ease-out hover:bg-gray-750 hover:bg-[#2a2f3a] hover:duration-0">
      {/* Group header */}
      <div className="px-4 py-2 border-b border-gray-700 bg-gray-800/60 flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Container className="w-4 h-4 text-blue-400" />
          <span className="font-semibold text-sm">{name || slug}</span>
          <span className="text-xs text-gray-500 font-mono">{slug}</span>
          <button
            onClick={() => onEditApp(group)}
            className="p-1 text-gray-500 hover:text-blue-400 hover:bg-gray-700 transition-colors"
            title="Modifier l'application"
          >
            <Pencil className="w-3.5 h-3.5" />
          </button>
          <button
            onClick={() => onDeleteApp(group)}
            className="p-1 text-gray-500 hover:text-red-400 hover:bg-gray-700 transition-colors"
            title="Supprimer l'application"
          >
            <Trash2 className="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      {/* Container rows */}
      {dev ? (
        <ContainerCard
          container={dev}
          baseDomain={baseDomain}
          metrics={appMetrics[dev.id]}
          migration={migrations[dev.id]}
          hosts={hosts}
          isHostOffline={dev.host_id && dev.host_id !== 'local' && hosts.find(h => h.id === dev.host_id)?.status !== 'online'}
          onStart={onStart}
          onStop={onStop}
          onTerminal={onTerminal}
          onToggleSecurity={onToggleSecurity}
          onMigrate={onMigrate}
          onMigrationDismiss={() => onMigrationDismiss(dev.id)}
          MigrationProgress={MigrationProgress}
        />
      ) : (
        <div
          className="grid items-center gap-x-3 px-4 py-1.5 border-b border-gray-700/30 hover:bg-gray-700/20 transition-colors"
          style={{ gridTemplateColumns: CONTAINER_GRID }}
        >
          <span className="text-xs px-1.5 py-0.5 font-medium text-center bg-blue-100 text-blue-800">DEV</span>
          <button
            onClick={() => onCreatePaired(slug, name, 'development', prod?.id)}
            className="flex items-center gap-1 text-xs text-gray-500 hover:text-blue-400 transition-colors col-span-9"
          >
            <Plus className="w-3.5 h-3.5" />
            Créer environnement DEV
          </button>
        </div>
      )}

      {prod ? (
        <ContainerCard
          container={prod}
          baseDomain={baseDomain}
          metrics={appMetrics[prod.id]}
          migration={migrations[prod.id]}
          hosts={hosts}
          isHostOffline={prod.host_id && prod.host_id !== 'local' && hosts.find(h => h.id === prod.host_id)?.status !== 'online'}
          onStart={onStart}
          onStop={onStop}
          onTerminal={onTerminal}
          onToggleSecurity={onToggleSecurity}
          onMigrate={onMigrate}
          onMigrationDismiss={() => onMigrationDismiss(prod.id)}
          MigrationProgress={MigrationProgress}
        />
      ) : (
        <div
          className="grid items-center gap-x-3 px-4 py-1.5 border-b border-gray-700/30 hover:bg-gray-700/20 transition-colors"
          style={{ gridTemplateColumns: CONTAINER_GRID }}
        >
          <span className="text-xs px-1.5 py-0.5 font-medium text-center bg-purple-100 text-purple-800">PROD</span>
          <button
            onClick={() => onCreatePaired(slug, name, 'production', dev?.id)}
            className="flex items-center gap-1 text-xs text-gray-500 hover:text-purple-400 transition-colors col-span-9"
          >
            <Plus className="w-3.5 h-3.5" />
            Créer environnement PROD
          </button>
        </div>
      )}
    </div>
  );
}

export default AppGroupCard;
