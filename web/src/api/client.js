import axios from 'axios';

const api = axios.create({
  baseURL: '/api',
  timeout: 30000,
  withCredentials: true  // Enable cookies for session-based auth
});

// Interceptor to handle session expiration
api.interceptors.response.use(
  (response) => {
    // Check if response indicates session expired
    if (response.data && response.data.success === false && response.data.error === 'Session expiree') {
      // Force cookie deletion by setting it to expire immediately
      document.cookie = 'auth_session=; path=/; expires=Thu, 01 Jan 1970 00:00:00 UTC; domain=' + window.location.hostname;
      document.cookie = 'auth_session=; path=/; expires=Thu, 01 Jan 1970 00:00:00 UTC';
    }
    return response;
  },
  (error) => {
    // Handle 401 errors
    if (error.response && error.response.status === 401) {
      // Force cookie deletion
      document.cookie = 'auth_session=; path=/; expires=Thu, 01 Jan 1970 00:00:00 UTC; domain=' + window.location.hostname;
      document.cookie = 'auth_session=; path=/; expires=Thu, 01 Jan 1970 00:00:00 UTC';
    }
    return Promise.reject(error);
  }
);

// DNS/DHCP
export const getDnsConfig = () => api.get('/dns');
export const getDhcpLeases = () => api.get('/dns/leases');

// Network
export const getInterfaces = () => api.get('/network/interfaces');
export const getRoutes = () => api.get('/network/routes');
export const getIpv6Routes = () => api.get('/network/routes6');

// NAT/Firewall
export const getNatRules = () => api.get('/nat/rules');
export const getFilterRules = () => api.get('/nat/filter');
export const getMasqueradeRules = () => api.get('/nat/masquerade');
export const getPortForwards = () => api.get('/nat/forwards');
export const getFirewallStatus = () => api.get('/nat/status');
export const getRoutingRules = () => api.get('/nat/routing-rules');
export const getChainStats = () => api.get('/nat/stats');

// AdBlock
export const getAdblockStats = () => api.get('/adblock/stats');
export const getWhitelist = () => api.get('/adblock/whitelist');
export const addToWhitelist = (domain) => api.post('/adblock/whitelist', { domain });
export const removeFromWhitelist = (domain) => api.delete(`/adblock/whitelist/${domain}`);
export const updateAdblockLists = () => api.post('/adblock/update');
export const searchBlocked = (query) => api.get('/adblock/search', { params: { q: query } });

// DDNS
export const getDdnsStatus = () => api.get('/ddns/status');
export const forceDdnsUpdate = () => api.post('/ddns/update');

// Reverse Proxy
export const getReverseProxyConfig = () => api.get('/reverseproxy/config');
export const getReverseProxyStatus = () => api.get('/reverseproxy/status');
export const getReverseProxyHosts = () => api.get('/reverseproxy/hosts');
export const addReverseProxyHost = (host) => api.post('/reverseproxy/hosts', host);
export const updateReverseProxyHost = (id, updates) => api.put(`/reverseproxy/hosts/${id}`, updates);
export const deleteReverseProxyHost = (id) => api.delete(`/reverseproxy/hosts/${id}`);
export const toggleReverseProxyHost = (id, enabled) => api.post(`/reverseproxy/hosts/${id}/toggle`, { enabled });
export const updateBaseDomain = (baseDomain) => api.put('/reverseproxy/config/domain', { baseDomain });
export const renewCertificates = () => api.post('/reverseproxy/certificates/renew');
export const reloadCaddy = () => api.post('/reverseproxy/reload');
export const getCertificatesStatus = () => api.get('/reverseproxy/certificates/status');

// Reverse Proxy - Environments
export const getReverseProxyEnvironments = () => api.get('/reverseproxy/environments');

// Reverse Proxy - Applications
export const getReverseProxyApplications = () => api.get('/reverseproxy/applications');
export const addReverseProxyApplication = (app) => api.post('/reverseproxy/applications', app);
export const updateReverseProxyApplication = (id, updates) => api.put(`/reverseproxy/applications/${id}`, updates);
export const deleteReverseProxyApplication = (id) => api.delete(`/reverseproxy/applications/${id}`);
export const toggleReverseProxyApplication = (id, enabled) => api.post(`/reverseproxy/applications/${id}/toggle`, { enabled });

// Reverse Proxy - Cloudflare
export const getCloudflareConfig = () => api.get('/reverseproxy/cloudflare');
export const updateCloudflareConfig = (config) => api.put('/reverseproxy/cloudflare', config);

// Auth - Session (login page)
export const login = (username, password, remember_me = false) => api.post('/auth/login', { username, password, remember_me });
export const logout = () => api.post('/auth/logout');
export const checkAuth = () => api.get('/auth/check');
export const getMe = () => api.get('/auth/me');

// System Updates
export const getUpdatesStatus = () => api.get('/updates/status');
export const getLastUpdatesCheck = () => api.get('/updates/last');
export const checkForUpdates = () => api.post('/updates/check', {}, { timeout: 300000 });
export const cancelUpdatesCheck = () => api.post('/updates/cancel');

// System Updates - Upgrade actions
export const getUpgradeStatus = () => api.get('/updates/upgrade/status');
export const runAptUpgrade = () => api.post('/updates/upgrade/apt', {}, { timeout: 1800000 });
export const runAptFullUpgrade = () => api.post('/updates/upgrade/apt-full', {}, { timeout: 1800000 });
export const runSnapRefresh = () => api.post('/updates/upgrade/snap', {}, { timeout: 1800000 });
export const cancelUpgrade = () => api.post('/updates/upgrade/cancel');

// Energy - CPU Info
export const getCpuInfo = () => api.get('/energy/cpu');

// Energy - Modes (unified: economy/auto/performance)
export const getEnergyModes = () => api.get('/energy/modes');
export const getCurrentEnergyMode = () => api.get('/energy/mode');
export const setEnergyMode = (mode) => api.post(`/energy/mode/${mode}`);

// Energy - Schedule
export const getEnergySchedule = () => api.get('/energy/schedule');
export const saveEnergySchedule = (config) => api.post('/energy/schedule', config);

// Energy - Auto-select
export const getAutoSelectConfig = () => api.get('/energy/autoselect');
export const saveAutoSelectConfig = (config) => api.post('/energy/autoselect', config);
export const getNetworkRps = () => api.get('/energy/autoselect/rps');
export const getAutoSelectStatus = () => api.get('/energy/autoselect/status');
export const getSelectableInterfaces = () => api.get('/energy/interfaces');

// Energy - Benchmark
export const getBenchmarkStatus = () => api.get('/energy/benchmark');
export const startBenchmark = (duration = 60) => api.post('/energy/benchmark/start', { duration });
export const stopBenchmark = () => api.post('/energy/benchmark/stop');

// Users - Authelia Status
export const getAutheliaStatus = () => api.get('/users/authelia/status');
export const getAutheliaInstallInstructions = () => api.get('/users/authelia/install');
export const bootstrapAdmin = (password) => api.post('/users/authelia/bootstrap', { password });

// Users - CRUD
export const getUsers = () => api.get('/users');
export const getUser = (username) => api.get(`/users/${username}`);
export const createUser = (data) => api.post('/users', data);
export const updateUser = (username, data) => api.put(`/users/${username}`, data);
export const deleteUser = (username) => api.delete(`/users/${username}`);
export const changeUserPassword = (username, password) => api.put(`/users/${username}/password`, { password });

// Users - Groups
export const getUserGroups = () => api.get('/users/groups');

export default api;
