import { useState, useEffect } from 'react';
import { X, Eye, EyeOff, Shield, User } from 'lucide-react';
import Button from './Button';

const GROUPS = [
  { id: 'admins', label: 'Administrateurs', icon: Shield, description: 'Accès complet aux services' },
  { id: 'users', label: 'Utilisateurs', icon: User, description: 'Accès basique' }
];

function UserModal({ isOpen, onClose, onSave, user = null, saving = false }) {
  const [form, setForm] = useState({
    username: '',
    password: '',
    confirmPassword: '',
    displayname: '',
    email: '',
    groups: ['users']
  });
  const [showPassword, setShowPassword] = useState(false);
  const [errors, setErrors] = useState({});

  const isEditing = !!user;

  useEffect(() => {
    if (user) {
      setForm({
        username: user.username || '',
        password: '',
        confirmPassword: '',
        displayname: user.displayname || '',
        email: user.email || '',
        groups: user.groups || ['users']
      });
    } else {
      setForm({
        username: '',
        password: '',
        confirmPassword: '',
        displayname: '',
        email: '',
        groups: ['users']
      });
    }
    setErrors({});
  }, [user, isOpen]);

  function validate() {
    const newErrors = {};

    if (!isEditing) {
      if (!form.username || form.username.length < 3) {
        newErrors.username = 'Minimum 3 caractères';
      } else if (!/^[a-zA-Z0-9_]+$/.test(form.username)) {
        newErrors.username = 'Lettres, chiffres et underscores uniquement';
      }

      if (!form.password || form.password.length < 8) {
        newErrors.password = 'Minimum 8 caractères';
      }

      if (form.password !== form.confirmPassword) {
        newErrors.confirmPassword = 'Les mots de passe ne correspondent pas';
      }
    }

    if (form.groups.length === 0) {
      newErrors.groups = 'Au moins un groupe requis';
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  }

  function handleSubmit(e) {
    e.preventDefault();
    if (!validate()) return;

    const data = {
      displayname: form.displayname || form.username,
      email: form.email,
      groups: form.groups
    };

    if (!isEditing) {
      data.username = form.username.toLowerCase();
      data.password = form.password;
    }

    onSave(data);
  }

  function toggleGroup(groupId) {
    setForm(prev => {
      const groups = prev.groups.includes(groupId)
        ? prev.groups.filter(g => g !== groupId)
        : [...prev.groups, groupId];
      return { ...prev, groups };
    });
  }

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-gray-800 rounded-lg border border-gray-700 w-full max-w-md mx-4">
        <div className="flex items-center justify-between p-4 border-b border-gray-700">
          <h3 className="font-semibold">
            {isEditing ? 'Modifier l\'utilisateur' : 'Nouvel utilisateur'}
          </h3>
          <button onClick={onClose} className="text-gray-400 hover:text-white">
            <X className="w-5 h-5" />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="p-4 space-y-4">
          {/* Username */}
          <div>
            <label className="block text-sm text-gray-400 mb-1">
              Nom d'utilisateur
            </label>
            <input
              type="text"
              value={form.username}
              onChange={e => setForm({ ...form, username: e.target.value })}
              disabled={isEditing}
              className={`w-full bg-gray-700 border rounded-lg px-3 py-2 text-white ${
                errors.username ? 'border-red-500' : 'border-gray-600'
              } ${isEditing ? 'opacity-50 cursor-not-allowed' : ''}`}
              placeholder="johndoe"
            />
            {errors.username && (
              <p className="text-red-400 text-xs mt-1">{errors.username}</p>
            )}
          </div>

          {/* Display Name */}
          <div>
            <label className="block text-sm text-gray-400 mb-1">
              Nom affiché
            </label>
            <input
              type="text"
              value={form.displayname}
              onChange={e => setForm({ ...form, displayname: e.target.value })}
              className="w-full bg-gray-700 border border-gray-600 rounded-lg px-3 py-2 text-white"
              placeholder="John Doe"
            />
          </div>

          {/* Email */}
          <div>
            <label className="block text-sm text-gray-400 mb-1">
              Email
            </label>
            <input
              type="email"
              value={form.email}
              onChange={e => setForm({ ...form, email: e.target.value })}
              className="w-full bg-gray-700 border border-gray-600 rounded-lg px-3 py-2 text-white"
              placeholder="john@example.com"
            />
          </div>

          {/* Password (only for new users) */}
          {!isEditing && (
            <>
              <div>
                <label className="block text-sm text-gray-400 mb-1">
                  Mot de passe
                </label>
                <div className="relative">
                  <input
                    type={showPassword ? 'text' : 'password'}
                    value={form.password}
                    onChange={e => setForm({ ...form, password: e.target.value })}
                    className={`w-full bg-gray-700 border rounded-lg px-3 py-2 text-white pr-10 ${
                      errors.password ? 'border-red-500' : 'border-gray-600'
                    }`}
                    placeholder="Minimum 8 caractères"
                  />
                  <button
                    type="button"
                    onClick={() => setShowPassword(!showPassword)}
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-white"
                  >
                    {showPassword ? <EyeOff className="w-4 h-4" /> : <Eye className="w-4 h-4" />}
                  </button>
                </div>
                {errors.password && (
                  <p className="text-red-400 text-xs mt-1">{errors.password}</p>
                )}
              </div>

              <div>
                <label className="block text-sm text-gray-400 mb-1">
                  Confirmer le mot de passe
                </label>
                <input
                  type={showPassword ? 'text' : 'password'}
                  value={form.confirmPassword}
                  onChange={e => setForm({ ...form, confirmPassword: e.target.value })}
                  className={`w-full bg-gray-700 border rounded-lg px-3 py-2 text-white ${
                    errors.confirmPassword ? 'border-red-500' : 'border-gray-600'
                  }`}
                  placeholder="Confirmer le mot de passe"
                />
                {errors.confirmPassword && (
                  <p className="text-red-400 text-xs mt-1">{errors.confirmPassword}</p>
                )}
              </div>
            </>
          )}

          {/* Groups */}
          <div>
            <label className="block text-sm text-gray-400 mb-2">
              Groupes
            </label>
            <div className="space-y-2">
              {GROUPS.map(group => {
                const Icon = group.icon;
                const isSelected = form.groups.includes(group.id);
                return (
                  <button
                    key={group.id}
                    type="button"
                    onClick={() => toggleGroup(group.id)}
                    className={`w-full flex items-center gap-3 p-3 rounded-lg border transition-colors ${
                      isSelected
                        ? 'bg-blue-500/20 border-blue-500/50 text-blue-400'
                        : 'bg-gray-700 border-gray-600 text-gray-300 hover:border-gray-500'
                    }`}
                  >
                    <Icon className="w-5 h-5" />
                    <div className="text-left">
                      <div className="font-medium">{group.label}</div>
                      <div className="text-xs opacity-70">{group.description}</div>
                    </div>
                    {isSelected && (
                      <div className="ml-auto w-2 h-2 rounded-full bg-blue-400" />
                    )}
                  </button>
                );
              })}
            </div>
            {errors.groups && (
              <p className="text-red-400 text-xs mt-1">{errors.groups}</p>
            )}
          </div>

          {/* Actions */}
          <div className="flex gap-2 pt-2">
            <Button variant="secondary" onClick={onClose} className="flex-1">
              Annuler
            </Button>
            <Button
              variant="primary"
              onClick={handleSubmit}
              loading={saving}
              className="flex-1"
            >
              {isEditing ? 'Modifier' : 'Créer'}
            </Button>
          </div>
        </form>
      </div>
    </div>
  );
}

export default UserModal;
