import { Activity, CheckCircle, XCircle, MinusCircle, AlertTriangle } from 'lucide-react';

const stateConfig = {
  running:  { icon: CheckCircle,   color: 'text-green-400',  label: 'Actif' },
  failed:   { icon: XCircle,       color: 'text-red-400',    label: 'Erreur' },
  stopped:  { icon: MinusCircle,   color: 'text-gray-400',   label: 'Arrete' },
  starting: { icon: AlertTriangle, color: 'text-yellow-400', label: 'Demarrage' },
  disabled: { icon: MinusCircle,   color: 'text-gray-600',   label: 'Desactive' },
};

const priorityLabel = {
  critical:   'Critique',
  important:  'Important',
  background: 'Arriere-plan',
};

function ServiceStatusPanel({ services }) {
  if (!services || services.length === 0) return null;

  const grouped = { critical: [], important: [], background: [] };
  for (const svc of services) {
    (grouped[svc.priority] || grouped.background).push(svc);
  }

  return (
    <div className="bg-gray-800 border border-gray-700">
      <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-700">
        <Activity className="w-4 h-4 text-blue-400" />
        <h3 className="font-semibold text-sm">Services</h3>
      </div>
      <div className="p-3 space-y-4">
        {Object.entries(grouped).map(([priority, svcs]) =>
          svcs.length > 0 && (
            <div key={priority}>
              <p className="text-xs text-gray-500 uppercase tracking-wider mb-2">
                {priorityLabel[priority]}
              </p>
              <div className="space-y-1">
                {svcs.map(svc => {
                  const cfg = stateConfig[svc.state] || stateConfig.stopped;
                  const Icon = cfg.icon;
                  return (
                    <div key={svc.name} className="flex items-center justify-between px-2 py-1.5 hover:bg-gray-700/50" title={svc.error || cfg.label}>
                      <span className="text-sm font-mono truncate">{svc.name}</span>
                      <div className="flex items-center gap-1.5">
                        {svc.restartCount > 0 && <span className="text-xs text-yellow-500">{svc.restartCount}x</span>}
                        <Icon className={`w-4 h-4 ${cfg.color}`} />
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          )
        )}
      </div>
    </div>
  );
}

export default ServiceStatusPanel;
