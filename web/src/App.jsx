import { Routes, Route, Navigate, useSearchParams } from 'react-router-dom';
import { AuthProvider, useAuth } from './context/AuthContext';
import Layout from './components/Layout';
import Dashboard from './pages/Dashboard';
import Dns from './pages/Dns';
import Network from './pages/Network';
import Adblock from './pages/Adblock';
import Ddns from './pages/Ddns';
import ReverseProxy from './pages/ReverseProxy';
import Updates from './pages/Updates';
import Energy from './pages/Energy';
import Users from './pages/Users';
import Hosts from './pages/Hosts';
import Certificates from './pages/Certificates';
import Firewall from './pages/Firewall';
import Applications from './pages/Applications';
import Dataverse from './pages/Dataverse';
import Login from './pages/Login';
import Profile from './pages/Profile';

// Component to protect routes that require authentication
function ProtectedRoute({ children }) {
  const { isAuthenticated, loading } = useAuth();

  if (loading) {
    return (
      <div className="min-h-screen bg-gray-900 flex items-center justify-center">
        <div className="text-center">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500 mx-auto"></div>
          <p className="mt-4 text-gray-400">Chargement...</p>
        </div>
      </div>
    );
  }

  if (!isAuthenticated) {
    return <Navigate to="/login" replace />;
  }

  return children;
}

// Component to redirect authenticated users away from login
function PublicRoute({ children }) {
  const { isAuthenticated, loading } = useAuth();
  const [searchParams] = useSearchParams();
  const redirectUrl = searchParams.get('rd');

  if (loading) {
    return (
      <div className="min-h-screen bg-gray-900 flex items-center justify-center">
        <div className="text-center">
          <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-500 mx-auto"></div>
          <p className="mt-4 text-gray-400">Chargement...</p>
        </div>
      </div>
    );
  }

  if (isAuthenticated) {
    // If rd parameter is present, redirect to the original URL (cross-domain)
    if (redirectUrl) {
      window.location.href = redirectUrl;
      return null;
    }
    return <Navigate to="/" replace />;
  }

  return children;
}

function AppRoutes() {
  return (
    <Routes>
      {/* Public routes */}
      <Route path="/login" element={
        <PublicRoute>
          <Login />
        </PublicRoute>
      } />

      {/* Profile - protected but outside layout */}
      <Route path="/profile" element={
        <ProtectedRoute>
          <Profile />
        </ProtectedRoute>
      } />

      {/* Protected routes with Layout */}
      <Route path="/*" element={
        <ProtectedRoute>
          <Layout>
            <Routes>
              <Route path="/" element={<Dashboard />} />
              <Route path="/dns" element={<Dns />} />
              <Route path="/network" element={<Network />} />
              <Route path="/adblock" element={<Adblock />} />
              <Route path="/ddns" element={<Ddns />} />
              <Route path="/reverseproxy" element={<ReverseProxy />} />
              <Route path="/users" element={<Users />} />
              <Route path="/updates" element={<Updates />} />
              <Route path="/energy" element={<Energy />} />
              <Route path="/hosts" element={<Hosts />} />
              <Route path="/firewall" element={<Firewall />} />
              <Route path="/applications" element={<Applications />} />
              <Route path="/dataverse" element={<Dataverse />} />
              <Route path="/certificates" element={<Certificates />} />
            </Routes>
          </Layout>
        </ProtectedRoute>
      } />
    </Routes>
  );
}

function App() {
  return (
    <AuthProvider>
      <AppRoutes />
    </AuthProvider>
  );
}

export default App;
