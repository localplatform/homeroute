import { NavLink } from 'react-router-dom';
import {
  LayoutDashboard, Server, Shield, Globe, Settings,
  ArrowLeftRight, RefreshCw, Zap, Users, LogOut,
  User, HardDrive, Lock, Database, Cloud, Container, Table2,
  Store as StoreIcon
} from 'lucide-react';
import { useAuth } from '../context/AuthContext';

const navGroups = [
  {
    items: [
      { to: '/', icon: LayoutDashboard, label: 'Dashboard' },
    ],
  },
  {
    label: 'Réseau',
    items: [
      { to: '/dns', icon: Server, label: 'DNS / DHCP' },
      { to: '/adblock', icon: Shield, label: 'AdBlock' },
      { to: '/ddns', icon: Globe, label: 'Dynamic DNS' },
    ],
  },
  {
    label: 'Services',
    items: [
      { to: '/reverseproxy', icon: ArrowLeftRight, label: 'Reverse Proxy' },
      { to: '/certificates', icon: Lock, label: 'Certificats' },
      { to: '/cloud-relay', icon: Cloud, label: 'Cloud Relay' },
    ],
  },
  {
    label: 'Applications',
    items: [
      { to: '/containers', icon: Container, label: 'Applications' },
      { to: '/dataverse', icon: Database, label: 'Dataverse' },
      { to: '/data-browser', icon: Table2, label: 'Data Browser' },
      { to: '/store', icon: StoreIcon, label: 'Store' },
    ],
  },
  {
    label: 'Système',
    items: [
      { to: '/hosts', icon: HardDrive, label: 'Hotes' },
      { to: '/users', icon: Users, label: 'Utilisateurs' },
      { to: '/updates', icon: RefreshCw, label: 'Mises à jour' },
      { to: '/energy', icon: Zap, label: 'Énergie' },
    ],
  },
];

function Sidebar() {
  const { user, logout } = useAuth();

  return (
    <aside className="w-64 bg-gray-800 border-r border-gray-700 flex flex-col">
      <div className="p-4 border-b border-gray-700">
        <h1 className="text-xl font-bold flex items-center gap-2">
          <Settings className="w-6 h-6 text-blue-400" />
          HomeRoute
        </h1>
        <p className="text-xs text-gray-400 mt-1">cloudmaster</p>
      </div>

      <nav className="flex-1 py-2 overflow-y-auto">
        {navGroups.map((group, gi) => (
          <div key={gi}>
            {group.label && (
              <div className="px-4 pt-4 pb-1 text-xs text-gray-500 uppercase tracking-wider">
                {group.label}
              </div>
            )}
            <ul className="space-y-0.5">
              {group.items.map(({ to, icon: Icon, label }) => (
                <li key={to}>
                  <NavLink
                    to={to}
                    className={({ isActive }) =>
                      `flex items-center gap-3 px-4 py-2 transition-[background-color,color] duration-300 ease-out hover:duration-0 text-sm ${
                        isActive
                          ? 'border-l-3 border-blue-400 bg-gray-700/50 text-white'
                          : 'border-l-3 border-transparent text-gray-300 hover:bg-gray-700/30'
                      }`
                    }
                  >
                    <Icon className="w-5 h-5" />
                    {label}
                  </NavLink>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </nav>

      <div className="p-4 border-t border-gray-700">
        {user && (
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 min-w-0">
              <User className="w-4 h-4 text-gray-400 flex-shrink-0" />
              <div className="min-w-0">
                <p className="text-sm text-gray-300 truncate">
                  {user.displayName || user.username}
                </p>
                {user.isAdmin && (
                  <p className="text-xs text-blue-400">Admin</p>
                )}
              </div>
            </div>
            <button
              onClick={logout}
              className="p-2 text-gray-400 hover:text-red-400 hover:bg-gray-700 transition-[background-color,color] duration-300 ease-out hover:duration-0"
              title="Deconnexion"
            >
              <LogOut className="w-4 h-4" />
            </button>
          </div>
        )}
        <p className="text-xs text-gray-500 mt-2">HomeRoute</p>
      </div>
    </aside>
  );
}

export default Sidebar;
