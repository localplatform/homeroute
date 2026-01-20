import { useNavigate } from 'react-router-dom';
import { Shield, User, Mail, Users, LogOut } from 'lucide-react';
import { useAuth } from '../App';

function Profile() {
  const { user, logout } = useAuth();
  const navigate = useNavigate();

  const handleLogout = async () => {
    await logout();
    navigate('/login');
  };

  if (!user) {
    return null;
  }

  return (
    <div className="min-h-screen p-4 md:p-8">
      <div className="max-w-2xl mx-auto">
        {/* Header */}
        <div className="flex items-center justify-between mb-8">
          <div className="flex items-center gap-3">
            <div className="w-12 h-12 bg-blue-600 rounded-xl flex items-center justify-center">
              <Shield className="w-6 h-6 text-white" />
            </div>
            <div>
              <h1 className="text-xl font-bold text-white">Mon compte</h1>
              <p className="text-gray-400 text-sm">mynetwk.biz</p>
            </div>
          </div>
          <button
            onClick={handleLogout}
            className="flex items-center gap-2 px-4 py-2 bg-gray-800/50 hover:bg-gray-700/50 border border-gray-700 rounded-xl text-gray-300 transition-colors"
          >
            <LogOut className="w-4 h-4" />
            <span className="hidden sm:inline">Déconnexion</span>
          </button>
        </div>

        {/* User Info Card */}
        <div className="bg-gray-800/50 backdrop-blur-sm rounded-2xl p-6 border border-gray-700 mb-6">
          <h2 className="text-lg font-semibold text-white mb-4">Informations du compte</h2>

          <div className="space-y-4">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 bg-gray-700/50 rounded-lg flex items-center justify-center">
                <User className="w-5 h-5 text-gray-400" />
              </div>
              <div>
                <div className="text-sm text-gray-400">Nom d'utilisateur</div>
                <div className="text-white font-medium">{user.username}</div>
              </div>
            </div>

            <div className="flex items-center gap-3">
              <div className="w-10 h-10 bg-gray-700/50 rounded-lg flex items-center justify-center">
                <span className="text-lg">{user.displayname?.charAt(0) || user.username.charAt(0)}</span>
              </div>
              <div>
                <div className="text-sm text-gray-400">Nom d'affichage</div>
                <div className="text-white font-medium">{user.displayname || user.username}</div>
              </div>
            </div>

            {user.email && (
              <div className="flex items-center gap-3">
                <div className="w-10 h-10 bg-gray-700/50 rounded-lg flex items-center justify-center">
                  <Mail className="w-5 h-5 text-gray-400" />
                </div>
                <div>
                  <div className="text-sm text-gray-400">Email</div>
                  <div className="text-white font-medium">{user.email}</div>
                </div>
              </div>
            )}

            <div className="flex items-center gap-3">
              <div className="w-10 h-10 bg-gray-700/50 rounded-lg flex items-center justify-center">
                <Users className="w-5 h-5 text-gray-400" />
              </div>
              <div>
                <div className="text-sm text-gray-400">Groupes</div>
                <div className="flex flex-wrap gap-2 mt-1">
                  {user.groups?.map(group => (
                    <span
                      key={group}
                      className="px-2 py-1 bg-blue-500/20 text-blue-400 text-xs rounded-lg"
                    >
                      {group}
                    </span>
                  ))}
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

export default Profile;
