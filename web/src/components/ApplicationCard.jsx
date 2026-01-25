import { useState } from 'react';
import {
  Globe,
  Server,
  Power,
  Pencil,
  Trash2,
  Shield,
  Key,
  ExternalLink,
  ChevronDown,
  ChevronUp,
  Layers,
  Lock
} from 'lucide-react';

function ApplicationCard({ app, environments, baseDomain, certStatuses, onToggle, onEdit, onDelete }) {
  const [expanded, setExpanded] = useState(false);

  // Get active environments for this app (those that have endpoints configured)
  const activeEnvIds = app.endpoints ? Object.keys(app.endpoints) : [];
  const activeEnvs = environments.filter(env => activeEnvIds.includes(env.id));

  // Generate domain for endpoint
  const getDomain = (type, env, apiSlug = '') => {
    if (type === 'api') {
      // Format: {app}-{slug}.{apiPrefix}.{baseDomain} or {app}.{apiPrefix}.{baseDomain}
      const hostPart = apiSlug ? `${app.slug}-${apiSlug}` : app.slug;
      return `${hostPart}.${env.apiPrefix}.${baseDomain}`;
    }
    return env.prefix ? `${app.slug}.${env.prefix}.${baseDomain}` : `${app.slug}.${baseDomain}`;
  };

  // Get certificate status for an endpoint
  const getCertStatus = (type, envId, apiSlug = '') => {
    if (!certStatuses) return null;
    const key = apiSlug
      ? `${app.id}-api-${apiSlug}-${envId}`
      : `${app.id}-${type}-${envId}`;
    return certStatuses[key];
  };

  // Render certificate status badge
  const CertBadge = ({ type, envId, apiSlug = '' }) => {
    const status = getCertStatus(type, envId, apiSlug);
    if (!status) return null;

    const isValid = status.valid;
    const isExpiringSoon = isValid && status.daysRemaining <= 14;

    return (
      <span className={`flex items-center gap-1 text-xs px-1.5 py-0.5 rounded ${
        isValid
          ? isExpiringSoon ? 'text-yellow-400 bg-yellow-900/30' : 'text-green-400 bg-green-900/30'
          : 'text-red-400 bg-red-900/30'
      }`} title={isValid ? `Expire dans ${status.daysRemaining} jours` : status.error || 'Erreur'}>
        <Lock className="w-3 h-3" />
        {isValid ? (isExpiringSoon ? `${status.daysRemaining}j` : 'OK') : '!'}
      </span>
    );
  };

  // Get first env endpoint for header display
  const firstEnvId = activeEnvIds[0];
  const firstEndpoint = firstEnvId ? app.endpoints[firstEnvId] : null;

  return (
    <div className={`bg-gray-800/50 border rounded-lg overflow-hidden transition-colors ${
      app.enabled ? 'border-gray-700' : 'border-gray-800 opacity-60'
    }`}>
      {/* Header */}
      <div className="p-4 flex items-center gap-4">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <h3 className="font-semibold text-white truncate">{app.name}</h3>
            <span className="text-xs font-mono text-gray-500 bg-gray-700/50 px-2 py-0.5 rounded">
              {app.slug}
            </span>
            {activeEnvs.map(env => (
              <span key={env.id} className="text-xs text-gray-400 bg-gray-700/30 px-2 py-0.5 rounded">
                {env.name}
              </span>
            ))}
          </div>
          <div className="flex items-center gap-2 mt-1 text-sm text-gray-400">
            {firstEndpoint && (
              <>
                <Globe className="w-3 h-3" />
                <span className="font-mono text-xs">
                  {firstEndpoint.frontend?.targetHost}:{firstEndpoint.frontend?.targetPort}
                  {/* Show first API from apis[] or legacy api */}
                  {firstEndpoint.apis?.[0] && ` / ${firstEndpoint.apis[0].targetHost}:${firstEndpoint.apis[0].targetPort}`}
                  {!firstEndpoint.apis?.[0] && firstEndpoint.api && ` / ${firstEndpoint.api.targetHost}:${firstEndpoint.api.targetPort}`}
                  {/* Show count if multiple APIs */}
                  {(firstEndpoint.apis?.length || 0) > 1 && ` +${firstEndpoint.apis.length - 1} api`}
                </span>
                {activeEnvIds.length > 1 && (
                  <span className="text-gray-500">+{activeEnvIds.length - 1} env</span>
                )}
              </>
            )}
          </div>
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2">
          <button
            onClick={() => onToggle(app.id, !app.enabled)}
            className={`p-2 rounded transition-colors ${
              app.enabled
                ? 'text-green-400 bg-green-900/30 hover:bg-green-900/50'
                : 'text-gray-500 bg-gray-700/30 hover:bg-gray-700/50'
            }`}
            title={app.enabled ? 'Disable' : 'Enable'}
          >
            <Power className="w-4 h-4" />
          </button>
          <button
            onClick={() => onEdit(app)}
            className="p-2 text-blue-400 hover:bg-blue-900/30 rounded"
            title="Edit"
          >
            <Pencil className="w-4 h-4" />
          </button>
          <button
            onClick={() => onDelete(app.id)}
            className="p-2 text-red-400 hover:bg-red-900/30 rounded"
            title="Delete"
          >
            <Trash2 className="w-4 h-4" />
          </button>
          <button
            onClick={() => setExpanded(!expanded)}
            className="p-2 text-gray-400 hover:bg-gray-700/30 rounded"
          >
            {expanded ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
          </button>
        </div>
      </div>

      {/* Expanded: Show all URLs per environment */}
      {expanded && (
        <div className="border-t border-gray-700 p-4 bg-gray-900/30">
          <div className="space-y-4">
            {activeEnvs.map(env => {
              const envEndpoint = app.endpoints[env.id];
              if (!envEndpoint) return null;

              return (
                <div key={env.id} className="space-y-2">
                  <div className="flex items-center gap-2 text-xs font-medium text-gray-400">
                    <Layers className="w-3 h-3" />
                    <span className="uppercase tracking-wider">{env.name}</span>
                    {/* Check if any endpoint has localOnly */}
                    {(envEndpoint.frontend?.localOnly || (envEndpoint.apis || []).some(a => a.localOnly) || envEndpoint.api?.localOnly) && (
                      <span className="flex items-center gap-1 text-yellow-400 bg-yellow-900/30 px-1.5 py-0.5 rounded">
                        <Shield className="w-3 h-3" />
                      </span>
                    )}
                    {/* Check if any endpoint has requireAuth */}
                    {(envEndpoint.frontend?.requireAuth || (envEndpoint.apis || []).some(a => a.requireAuth) || envEndpoint.api?.requireAuth) && (
                      <span className="flex items-center gap-1 text-purple-400 bg-purple-900/30 px-1.5 py-0.5 rounded">
                        <Key className="w-3 h-3" />
                      </span>
                    )}
                  </div>
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-2">
                    {/* Frontend URL */}
                    {envEndpoint.frontend && (
                      <div className="flex items-center gap-2 bg-gray-800/50 px-3 py-2 rounded">
                        <a
                          href={`https://${getDomain('frontend', env)}`}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="flex items-center gap-2 text-sm font-mono text-blue-400 hover:text-blue-300 flex-1 min-w-0 group"
                        >
                          <Globe className="w-4 h-4 text-gray-500 group-hover:text-blue-400 flex-shrink-0" />
                          <span className="truncate">{getDomain('frontend', env)}</span>
                          <span className="text-gray-500 text-xs flex-shrink-0">:{envEndpoint.frontend.targetPort}</span>
                          <ExternalLink className="w-3 h-3 opacity-0 group-hover:opacity-100 flex-shrink-0" />
                        </a>
                        <CertBadge type="frontend" envId={env.id} />
                      </div>
                    )}

                    {/* API URLs (multiple APIs supported) */}
                    {(envEndpoint.apis || []).map((api, apiIndex) => (
                      <div key={apiIndex} className="flex items-center gap-2 bg-gray-800/50 px-3 py-2 rounded">
                        <a
                          href={`https://${getDomain('api', env, api.slug)}`}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="flex items-center gap-2 text-sm font-mono text-green-400 hover:text-green-300 flex-1 min-w-0 group"
                        >
                          <Server className="w-4 h-4 text-gray-500 group-hover:text-green-400 flex-shrink-0" />
                          <span className="truncate">{getDomain('api', env, api.slug)}</span>
                          <span className="text-gray-500 text-xs flex-shrink-0">:{api.targetPort}</span>
                          <ExternalLink className="w-3 h-3 opacity-0 group-hover:opacity-100 flex-shrink-0" />
                        </a>
                        <CertBadge type="api" envId={env.id} apiSlug={api.slug} />
                      </div>
                    ))}

                    {/* Legacy: single API support for backward compatibility */}
                    {envEndpoint.api && (!envEndpoint.apis || envEndpoint.apis.length === 0) && (
                      <div className="flex items-center gap-2 bg-gray-800/50 px-3 py-2 rounded">
                        <a
                          href={`https://${getDomain('api', env)}`}
                          target="_blank"
                          rel="noopener noreferrer"
                          className="flex items-center gap-2 text-sm font-mono text-green-400 hover:text-green-300 flex-1 min-w-0 group"
                        >
                          <Server className="w-4 h-4 text-gray-500 group-hover:text-green-400 flex-shrink-0" />
                          <span className="truncate">{getDomain('api', env)}</span>
                          <span className="text-gray-500 text-xs flex-shrink-0">:{envEndpoint.api.targetPort}</span>
                          <ExternalLink className="w-3 h-3 opacity-0 group-hover:opacity-100 flex-shrink-0" />
                        </a>
                        <CertBadge type="api" envId={env.id} />
                      </div>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}
    </div>
  );
}

export default ApplicationCard;
