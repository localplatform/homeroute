import { useState, useEffect } from 'react';
import { X } from 'lucide-react';
import Button from './Button';

const COLOR_PRESETS = [
  { name: 'Violet', value: '#8B5CF6' },
  { name: 'Teal', value: '#14B8A6' },
  { name: 'Orange', value: '#F97316' },
  { name: 'Rose', value: '#F43F5E' },
  { name: 'Indigo', value: '#6366F1' },
  { name: 'Emerald', value: '#10B981' },
  { name: 'Amber', value: '#F59E0B' },
  { name: 'Cyan', value: '#06B6D4' }
];

function GroupModal({ isOpen, onClose, onSave, group = null, saving = false }) {
  const [form, setForm] = useState({ name: '', description: '', color: '#8B5CF6' });
  const [errors, setErrors] = useState({});
  const isEditing = !!group;

  useEffect(() => {
    if (group) setForm({ name: group.name || '', description: group.description || '', color: group.color || '#8B5CF6' });
    else setForm({ name: '', description: '', color: '#8B5CF6' });
    setErrors({});
  }, [group, isOpen]);

  function validate() {
    const newErrors = {};
    if (!form.name || form.name.trim().length < 2) newErrors.name = 'Minimum 2 caractères';
    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  }

  function handleSubmit(e) {
    e.preventDefault();
    if (!validate()) return;
    onSave({ name: form.name.trim(), description: form.description.trim(), color: form.color });
  }

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-gray-800 border border-gray-700 w-full max-w-md mx-4">
        <div className="flex items-center justify-between p-4 border-b border-gray-700">
          <h3 className="font-semibold">{isEditing ? 'Modifier le groupe' : 'Nouveau groupe'}</h3>
          <button onClick={onClose} className="text-gray-400 hover:text-white"><X className="w-5 h-5" /></button>
        </div>
        <form onSubmit={handleSubmit} className="p-4 space-y-4">
          <div>
            <label className="block text-sm text-gray-400 mb-1">Nom</label>
            <input type="text" value={form.name} onChange={e => setForm({ ...form, name: e.target.value })}
              className={`w-full bg-gray-700 border px-3 py-2 text-white ${errors.name ? 'border-red-500' : 'border-gray-600'}`} placeholder="Ex: Media, DevOps, Famille..." />
            {errors.name && <p className="text-red-400 text-xs mt-1">{errors.name}</p>}
          </div>
          <div>
            <label className="block text-sm text-gray-400 mb-1">Description</label>
            <input type="text" value={form.description} onChange={e => setForm({ ...form, description: e.target.value })}
              className="w-full bg-gray-700 border border-gray-600 px-3 py-2 text-white" placeholder="Description du groupe" />
          </div>
          <div>
            <label className="block text-sm text-gray-400 mb-2">Couleur</label>
            <div className="flex flex-wrap gap-2">
              {COLOR_PRESETS.map(preset => (
                <button key={preset.value} type="button" onClick={() => setForm({ ...form, color: preset.value })}
                  className={`w-8 h-8 border-2 transition-all ${form.color === preset.value ? 'border-white scale-110' : 'border-transparent hover:border-gray-500'}`}
                  style={{ backgroundColor: preset.value }} title={preset.name} />
              ))}
            </div>
          </div>
          <div className="flex gap-2 pt-2">
            <Button variant="secondary" onClick={onClose} className="flex-1">Annuler</Button>
            <Button variant="primary" onClick={handleSubmit} loading={saving} className="flex-1">{isEditing ? 'Modifier' : 'Créer'}</Button>
          </div>
        </form>
      </div>
    </div>
  );
}

export default GroupModal;
