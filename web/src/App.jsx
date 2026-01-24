import { Routes, Route } from 'react-router-dom';
import { AuthProvider } from './context/AuthContext';
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

function App() {
  return (
    <AuthProvider>
      <Routes>
        <Route path="/*" element={
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
            </Routes>
          </Layout>
        } />
      </Routes>
    </AuthProvider>
  );
}

export default App;
