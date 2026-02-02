import { Shield, User, Users } from 'lucide-react';

const BUILTIN_STYLES = {
  admins: { bg: 'bg-red-500/20', border: 'border-red-500/50', text: 'text-red-400', icon: Shield, label: 'Admin' },
  users: { bg: 'bg-blue-500/20', border: 'border-blue-500/50', text: 'text-blue-400', icon: User, label: 'User' }
};

function hexToRgb(hex) {
  const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex);
  if (!result) return null;
  return { r: parseInt(result[1], 16), g: parseInt(result[2], 16), b: parseInt(result[3], 16) };
}

function GroupBadge({ group, color, label, showIcon = true, size = 'sm' }) {
  const builtIn = BUILTIN_STYLES[group];
  const sizeClasses = { xs: 'text-xs px-1.5 py-0.5', sm: 'text-xs px-2 py-1', md: 'text-sm px-3 py-1' };

  if (builtIn) {
    const Icon = builtIn.icon;
    return (
      <span className={`inline-flex items-center gap-1 border ${builtIn.bg} ${builtIn.border} ${builtIn.text} ${sizeClasses[size]}`}>
        {showIcon && <Icon className="w-3 h-3" />}
        {label || builtIn.label}
      </span>
    );
  }

  const rgb = hexToRgb(color || '#8B5CF6');
  const colorStr = rgb ? `${rgb.r}, ${rgb.g}, ${rgb.b}` : '139, 92, 246';

  return (
    <span className={`inline-flex items-center gap-1 border ${sizeClasses[size]}`}
      style={{ backgroundColor: `rgba(${colorStr}, 0.2)`, borderColor: `rgba(${colorStr}, 0.5)`, color: color || '#8B5CF6' }}>
      {showIcon && <Users className="w-3 h-3" />}
      {label || group}
    </span>
  );
}

export default GroupBadge;
