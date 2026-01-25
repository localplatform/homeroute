import { Shield, User } from 'lucide-react';

const GROUP_STYLES = {
  admins: {
    bg: 'bg-red-500/20',
    border: 'border-red-500/50',
    text: 'text-red-400',
    icon: Shield,
    label: 'Admin'
  },
  users: {
    bg: 'bg-blue-500/20',
    border: 'border-blue-500/50',
    text: 'text-blue-400',
    icon: User,
    label: 'User'
  }
};

function GroupBadge({ group, showIcon = true, size = 'sm' }) {
  const style = GROUP_STYLES[group] || GROUP_STYLES.users;
  const Icon = style.icon;

  const sizeClasses = {
    xs: 'text-xs px-1.5 py-0.5',
    sm: 'text-xs px-2 py-1',
    md: 'text-sm px-3 py-1'
  };

  return (
    <span
      className={`inline-flex items-center gap-1 rounded-full border ${style.bg} ${style.border} ${style.text} ${sizeClasses[size]}`}
    >
      {showIcon && <Icon className="w-3 h-3" />}
      {style.label}
    </span>
  );
}

export default GroupBadge;
