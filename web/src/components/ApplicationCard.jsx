import { useState } from 'react';
import { Globe, Server, Power, Pencil, Trash2, Shield, Key, ExternalLink, ChevronDown, ChevronUp, Layers, Lock, Users } from 'lucide-react';
import GroupBadge from './GroupBadge';

function ApplicationCard({ app, environments, baseDomain, certStatuses, onToggle, onEdit, onDelete, userGroups = [] }) {
  const [expanded, setExpanded] = useState(false);
  const activeEnvIds = app.endpoints ? Object.keys(app.endpoints) : [];
  const activeEnvs = environments.filter(env => activeEnvIds.includes(env.id));

  const getDomain = (type, env, apiSlug = '') => {
    if (type === 'api') {
      const hostPart = apiSlug ? `${app.slug}-${apiSlug}` : app.slug;
      return `${hostPart}.${env.apiPrefix}.${baseDomain}`;
    }
    return env.prefix ? `${app.slug}.${env.prefix}.${baseDomain}` : `${app.slug}.${baseDomain}`;
  };

  const getCertStatus = (type, envId, apiSlug = '') => {
    if (!certStatuses) return null;
    const key = apiSlug ? `${app.id}-api-${apiSlug}-${envId}` : `${app.id}-${type}-${envId}`;
    return certStatuses[key];
  };

  const CertBadge = ({ type, envId, apiSlug = '' }) => {
    const status = getCertStatus(type, envId, apiSlug);
    if (!status) return null;
    const isValid = status.valid;
    const isExpiringSoon = isValid && status.daysRemaining <= 14;
    return (
      <span className={`flex items-center gap-1 text-xs px-1.5 py-0.5 ${
        isValid ? isExpiringSoon ? 'text-yellow-400 bg-yellow-900/30' : 'text-green-400 bg-green-900/30' : 'text-red-400 bg-red-900/30'
      }`} title={isValid ? `Expire dans ${status.daysRemaining} jours` : status.error || 'Erreur'}>
        <Lock className="w-3 h-3" />
        {isValid ? (isExpiringSoon ? `${status.daysRemaining}j` : 'OK') : '!'}
      </span>
    );
  };

  const firstEnvId = activeEnvIds[0];
  const firstEndpoint = firstEnvId ? app.endpoints[firstEnvId] : null;

  return (
    <div className={`bg-gray-800/50 border overflow-hidden transition-colors ${app.enabled ? 'border-gray-700' : 'border-gray-800 opacity-60'}`}>
      <div className="p-4 flex items-center gap-4">
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2">
            <h3 className="font-semibold text-white truncate">{app.name}</h3>
            <span className="text-xs font-mono text-gray-500 bg-gray-700/50 px-2 py-0.5">{app.slug}</span>
            {activeEnvs.map(env => (<span key={env.id} className="text-xs text-gray-400 bg-gray-700/30 px-2 py-0.5">{env.name}</span>))}
          </div>
          {app.allowedGroups && app.allowedGroups.length > 0 && (
            <div className="flex items-center gap-1.5 mt-1.5">
              <Users className="w-3 h-3 text-gray-500" />
              {app.allowedGroups.map(gid => {
                const group = userGroups.find(g => g.id === gid);
                return group ? <GroupBadge key={gid} group={gid} color={group.color} label={group.name} /> : <GroupBadge key={gid} group={gid} />;
              })}
            </div>
          )}
          <div className="flex items-center gap-2 mt-1 text-sm text-gray-400">
            {firstEndpoint && (
              <>
                <Globe className="w-3 h-3" />
                <span className="font-mono text-xs">
                  {firstEndpoint.frontend?.targetHost}:{firstEndpoint.frontend?.targetPort}
                  {firstEndpoint.apis?.[0] && ` / ${firstEndpoint.apis[0].targetHost}:${firstEndpoint.apis[0].targetPort}`}
                  {!firstEndpoint.apis?.[0] && firstEndpoint.api && ` / ${firstEndpoint.api.targetHost}:${firstEndpoint.api.targetPort}`}
                  {(firstEndpoint.apis?.length || 0) > 1 && ` +${firstEndpoint.apis.length - 1} api`}
                </span>
                {activeEnvIds.length > 1 && <span className="text-gray-500">+{activeEnvIds.length - 1} env</span>}
              </>
            )}
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button onClick={() => onToggle(app.id, !app.enabled)} className={`p-2 transition-colors ${app.enabled ? 'text-green-400 bg-green-900/30 hover:bg-green-900/50' : 'text-gray-500 bg-gray-700/30 hover:bg-gray-700/50'}`} title={app.enabled ? 'Disable' : 'Enable'}><Power className="w-4 h-4" /></button>
          <button onClick={() => onEdit(app)} className="p-2 text-blue-400 hover:bg-blue-900/30" title="Edit"><Pencil className="w-4 h-4" /></button>
          <button onClick={() => onDelete(app.id)} className="p-2 text-red-400 hover:bg-red-900/30" title="Delete"><Trash2 className="w-4 h-4" /></button>
          <button onClick={() => setExpanded(!expanded)} className="p-2 text-gray-400 hover:bg-gray-700/30">{expanded ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}</button>
        </div>
      </div>
      {expanded && (
        <div className="border-t border-gray-700 p-4 bg-gray-900/30">
          <div className="space-y-4">
            {activeEnvs.map(env => {
              const envEndpoint = app.endpoints[env.id];
              if (!envEndpoint) return null;
              return (
                <div key={env.id} className="space-y-2">
                  <div className="flex items-center gap-2 text-xs font-medium text-gray-400">
                    <Layers className="w-3 h-3" /><span className="uppercase tracking-wider">{env.name}</span>
                    {(envEndpoint.frontend?.localOnly || (envEndpoint.apis || []).some(a => a.localOnly) || envEndpoint.api?.localOnly) && (<span className="flex items-center gap-1 text-yellow-400 bg-yellow-900/30 px-1.5 py-0.5"><Shield className="w-3 h-3" /></span>)}
                    {(envEndpoint.frontend?.requireAuth || (envEndpoint.apis || []).some(a => a.requireAuth) || envEndpoint.api?.requireAuth) && (<span className="flex items-center gap-1 text-purple-400 bg-purple-900/30 px-1.5 py-0.5"><Key className="w-3 h-3" /></span>)}
                  </div>
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-0">
                    {envEndpoint.frontend && (
                      <div className="flex items-center gap-2 bg-gray-800/50 px-3 py-2 border border-gray-700/50">
                        <a href={`https://${getDomain('frontend', env)}`} target="_blank" rel="noopener noreferrer" className="flex items-center gap-2 text-sm font-mono text-blue-400 hover:text-blue-300 flex-1 min-w-0 group">
                          <Globe className="w-4 h-4 text-gray-500 group-hover:text-blue-400 flex-shrink-0" /><span className="truncate">{getDomain('frontend', env)}</span><span className="text-gray-500 text-xs flex-shrink-0">:{envEndpoint.frontend.targetPort}</span><ExternalLink className="w-3 h-3 opacity-0 group-hover:opacity-100 flex-shrink-0" />
                        </a>
                        <CertBadge type="frontend" envId={env.id} />
                      </div>
                    )}
                    {(envEndpoint.apis || []).map((api, apiIndex) => (
                      <div key={apiIndex} className="flex items-center gap-2 bg-gray-800/50 px-3 py-2 border border-gray-700/50">
                        <a href={`https://${getDomain('api', env, api.slug)}`} target="_blank" rel="noopener noreferrer" className="flex items-center gap-2 text-sm font-mono text-green-400 hover:text-green-300 flex-1 min-w-0 group">
                          <Server className="w-4 h-4 text-gray-500 group-hover:text-green-400 flex-shrink-0" /><span className="truncate">{getDomain('api', env, api.slug)}</span><span className="text-gray-500 text-xs flex-shrink-0">:{api.targetPort}</span><ExternalLink className="w-3 h-3 opacity-0 group-hover:opacity-100 flex-shrink-0" />
                        </a>
                        <CertBadge type="api" envId={env.id} apiSlug={api.slug} />
                      </div>
                    ))}
                    {envEndpoint.api && (!envEndpoint.apis || envEndpoint.apis.length === 0) && (
                      <div className="flex items-center gap-2 bg-gray-800/50 px-3 py-2 border border-gray-700/50">
                        <a href={`https://${getDomain('api', env)}`} target="_blank" rel="noopener noreferrer" className="flex items-center gap-2 text-sm font-mono text-green-400 hover:text-green-300 flex-1 min-w-0 group">
                          <Server className="w-4 h-4 text-gray-500 group-hover:text-green-400 flex-shrink-0" /><span className="truncate">{getDomain('api', env)}</span><span className="text-gray-500 text-xs flex-shrink-0">:{envEndpoint.api.targetPort}</span><ExternalLink className="w-3 h-3 opacity-0 group-hover:opacity-100 flex-shrink-0" />
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
