import { useState } from 'react';
import { Key, Shield, Code2, HardDrive } from 'lucide-react';
import Button from './Button';

function CreateContainerModal({
  baseDomain,
  hosts,
  containers,
  onClose,
  onCreate,
  saving,
  initialEnvironment,
  initialSlug,
  initialName,
  initialLinkedAppId,
}) {
  const isPaired = !!initialSlug;

  const [form, setForm] = useState({
    name: initialName || '',
    slug: initialSlug || '',
    host_id: 'local',
    environment: isPaired ? (initialEnvironment || 'development') : 'development',
    frontend: { auth_required: false, allowed_groups: [], local_only: false },
    code_server_enabled: isPaired ? (initialEnvironment !== 'production') : true,
    linked_app_id: initialLinkedAppId || '',
  });

  const isDev = form.environment === 'development';

  function handleSubmit() {
    if (!form.name || !form.slug) return;

    const payload = {
      name: form.name,
      slug: form.slug.toLowerCase(),
      host_id: form.host_id,
      environment: form.environment,
      frontend: {
        target_port: 3000,
        auth_required: form.frontend.auth_required,
        allowed_groups: form.frontend.allowed_groups,
        local_only: form.frontend.local_only,
      },
      code_server_enabled: isDev ? form.code_server_enabled : false,
      linked_app_id: form.linked_app_id || null,
    };

    onCreate(payload);
  }

  const previewUrl = form.slug && baseDomain
    ? isDev ? `code.${form.slug}.${baseDomain}` : `${form.slug}.${baseDomain}`
    : null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50 p-4">
      <div className="bg-gray-800 p-6 w-full max-w-lg border border-gray-700 max-h-[90vh] overflow-y-auto">
        <h2 className="text-xl font-bold mb-4">
          {isPaired
            ? `Nouvel environnement ${isDev ? 'DEV' : 'PROD'}`
            : 'Nouvelle application'
          }
        </h2>
        <div className="space-y-4">
          {/* Environment selector removed — from-scratch always creates DEV, PROD is auto-created */}

          {/* When paired: show environment as badge + slug as read-only info */}
          {isPaired && (
            <div className="flex items-center gap-3">
              <span className={`text-xs px-2 py-1 font-medium ${
                isDev ? 'bg-blue-100 text-blue-800' : 'bg-purple-100 text-purple-800'
              }`}>
                {isDev ? 'DEV' : 'PROD'}
              </span>
              <span className="text-sm text-gray-400 font-mono">{form.slug}</span>
              {previewUrl && (
                <span className="text-xs text-gray-500 font-mono">{previewUrl}</span>
              )}
            </div>
          )}

          {/* Name + Slug (only when creating from scratch) */}
          {!isPaired && (
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-sm text-gray-400 mb-1">Nom</label>
                <input
                  type="text"
                  placeholder="Mon App"
                  value={form.name}
                  onChange={e => {
                    const name = e.target.value;
                    const autoSlug = name.toLowerCase().replace(/[\s_]+/g, '-').replace(/[^a-z0-9-]/g, '').replace(/-+/g, '-').replace(/^-|-$/g, '');
                    setForm(f => ({
                      ...f,
                      name,
                      slug: f.slug === '' || f.slug === f.name.toLowerCase().replace(/[\s_]+/g, '-').replace(/[^a-z0-9-]/g, '').replace(/-+/g, '-').replace(/^-|-$/g, '')
                        ? autoSlug
                        : f.slug,
                    }));
                  }}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 text-sm"
                />
              </div>
              <div>
                <label className="block text-sm text-gray-400 mb-1">Slug</label>
                <input
                  type="text"
                  placeholder="mon-app"
                  value={form.slug}
                  onChange={e => setForm({ ...form, slug: e.target.value.toLowerCase().replace(/[^a-z0-9-]/g, '') })}
                  className="w-full px-3 py-2 bg-gray-900 border border-gray-600 text-sm font-mono"
                />
                {previewUrl && (
                  <p className="text-xs text-gray-500 mt-1 font-mono">{previewUrl}</p>
                )}
              </div>
            </div>
          )}

          {/* Host selector */}
          <div>
            <label className="block text-sm text-gray-400 mb-1">
              <HardDrive className="w-3.5 h-3.5 inline mr-1" />
              Hote
            </label>
            <select
              value={form.host_id}
              onChange={e => setForm({ ...form, host_id: e.target.value })}
              className="w-full px-3 py-2 bg-gray-900 border border-gray-600 text-sm"
            >
              <option value="local">HomeRoute (local)</option>
              {hosts.filter(h => h.status === 'online').map(h => (
                <option key={h.id} value={h.id}>{h.name} ({h.host})</option>
              ))}
            </select>
          </div>

          {/* Auto-creation note (only when creating from scratch) */}
          {!isPaired && (
            <p className="text-xs text-gray-500">Un conteneur de production sera automatiquement créé</p>
          )}

          {/* Auth options */}
          <div className="flex items-center gap-4">
            <label className="flex items-center gap-1.5 text-xs cursor-pointer">
              <input
                type="checkbox"
                checked={form.frontend.auth_required}
                onChange={e => setForm({ ...form, frontend: { ...form.frontend, auth_required: e.target.checked } })}
                className="rounded"
              />
              <Key className="w-3 h-3 text-purple-400" /> Auth requise
            </label>
            <label className="flex items-center gap-1.5 text-xs cursor-pointer">
              <input
                type="checkbox"
                checked={form.frontend.local_only}
                onChange={e => setForm({ ...form, frontend: { ...form.frontend, local_only: e.target.checked } })}
                className="rounded"
              />
              <Shield className="w-3 h-3 text-yellow-400" /> Local seulement
            </label>
          </div>

          {/* code-server (dev only) */}
          {isDev && (
            <label className="flex items-center gap-2 text-sm cursor-pointer">
              <input
                type="checkbox"
                checked={form.code_server_enabled}
                onChange={e => setForm({ ...form, code_server_enabled: e.target.checked })}
                className="rounded"
              />
              <Code2 className="w-4 h-4 text-cyan-400" />
              code-server IDE
              {form.slug && baseDomain && form.code_server_enabled && (
                <span className="text-xs text-gray-500 font-mono ml-2">code.{form.slug}.{baseDomain}</span>
              )}
            </label>
          )}
        </div>
        <div className="flex justify-end gap-2 mt-6">
          <Button variant="secondary" onClick={onClose}>Annuler</Button>
          <Button onClick={handleSubmit} loading={saving}>Creer</Button>
        </div>
      </div>
    </div>
  );
}

export default CreateContainerModal;
