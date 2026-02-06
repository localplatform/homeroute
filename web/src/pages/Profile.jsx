import { useNavigate } from 'react-router-dom';
import { Shield, User, Mail, Users, LogOut } from 'lucide-react';
import { useAuth } from '../context/AuthContext';
import PageHeader from '../components/PageHeader';

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
    <div>
      <PageHeader title="Mon compte" icon={User}>
        <button
          onClick={handleLogout}
          className="flex items-center gap-2 px-4 py-2 bg-gray-800/50 hover:bg-gray-700/50 border border-gray-700 text-gray-300 transition-colors"
        >
          <LogOut className="w-4 h-4" />
          <span className="hidden sm:inline">Deconnexion</span>
        </button>
      </PageHeader>

      <div>
        {/* User Info Card */}
        <div className="bg-gray-800/50 backdrop-blur-sm p-6 border border-gray-700">
          <h2 className="text-lg font-semibold text-white mb-4">Informations du compte</h2>

          <div className="space-y-px">
            <div className="flex items-center gap-3">
              <div className="w-10 h-10 bg-gray-700/50  flex items-center justify-center">
                <User className="w-5 h-5 text-gray-400" />
              </div>
              <div>
                <div className="text-sm text-gray-400">Nom d'utilisateur</div>
                <div className="text-white font-medium">{user.username}</div>
              </div>
            </div>

            <div className="flex items-center gap-3">
              <div className="w-10 h-10 bg-gray-700/50  flex items-center justify-center">
                <span className="text-lg">{(user.displayName || user.username)?.charAt(0)?.toUpperCase()}</span>
              </div>
              <div>
                <div className="text-sm text-gray-400">Nom d'affichage</div>
                <div className="text-white font-medium">{user.displayName || user.username}</div>
              </div>
            </div>

            {user.email && (
              <div className="flex items-center gap-3">
                <div className="w-10 h-10 bg-gray-700/50  flex items-center justify-center">
                  <Mail className="w-5 h-5 text-gray-400" />
                </div>
                <div>
                  <div className="text-sm text-gray-400">Email</div>
                  <div className="text-white font-medium">{user.email}</div>
                </div>
              </div>
            )}

            <div className="flex items-center gap-3">
              <div className="w-10 h-10 bg-gray-700/50  flex items-center justify-center">
                <Users className="w-5 h-5 text-gray-400" />
              </div>
              <div>
                <div className="text-sm text-gray-400">Groupes</div>
                <div className="flex flex-wrap gap-2 mt-1">
                  {user.groups?.map(group => (
                    <span
                      key={group}
                      className="px-2 py-1 bg-blue-500/20 text-blue-400 text-xs "
                    >
                      {group}
                    </span>
                  ))}
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* Admin badge */}
        {user.isAdmin && (
          <div className="bg-amber-500/10 border border-amber-500/30 p-4">
            <div className="flex items-center gap-2 text-amber-400">
              <Shield className="w-5 h-5" />
              <span className="font-medium">Compte administrateur</span>
            </div>
            <p className="text-amber-400/70 text-sm mt-1">
              Vous avez acces a toutes les fonctionnalites du systeme.
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

export default Profile;
