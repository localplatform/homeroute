import { NavLink } from 'react-router-dom';
import {
  LayoutDashboard, Server, Network, Shield, Globe, Settings,
  ArrowLeftRight, RefreshCw, Zap, Users, BarChart3, LogOut,
  User, Power, HardDrive, KeyRound
} from 'lucide-react';
import { useAuth } from '../context/AuthContext';

const navItems = [
  { to: '/', icon: LayoutDashboard, label: 'Dashboard' },
  { to: '/dns', icon: Server, label: 'DNS / DHCP' },
  { to: '/network', icon: Network, label: 'Réseau / Firewall' },
  { to: '/adblock', icon: Shield, label: 'AdBlock' },
  { to: '/ddns', icon: Globe, label: 'Dynamic DNS' },
  { to: '/reverseproxy', icon: ArrowLeftRight, label: 'Reverse Proxy' },
  { to: '/ca', icon: KeyRound, label: 'Certificats (CA)' },
  { to: '/users', icon: Users, label: 'Utilisateurs' },
  { to: '/updates', icon: RefreshCw, label: 'Mises a jour' },
  { to: '/energy', icon: Zap, label: 'Énergie' },
  { to: '/traffic', icon: BarChart3, label: 'Trafic' },
  { to: '/servers', icon: HardDrive, label: 'Serveurs' },
  { to: '/wol', icon: Power, label: 'Wake-on-LAN' },
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

      <nav className="flex-1 py-2">
        <ul className="space-y-0.5">
          {navItems.map(({ to, icon: Icon, label }) => (
            <li key={to}>
              <NavLink
                to={to}
                className={({ isActive }) =>
                  `flex items-center gap-3 px-4 py-2 transition-colors text-sm ${
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
              className="p-2 text-gray-400 hover:text-red-400 hover:bg-gray-700 transition-colors"
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
