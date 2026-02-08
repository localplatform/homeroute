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

// Services Status
export const getServicesStatus = () => api.get('/services/status');

// DNS/DHCP
export const getDnsConfig = () => api.get('/dns-dhcp/config');
export const getDhcpLeases = () => api.get('/dns-dhcp/leases');

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
export const updateDdnsToken = (token) => api.put('/ddns/token', { token });
export const updateDdnsConfig = (config) => api.put('/ddns/config', config);

// Reverse Proxy
export const getReverseProxyConfig = () => api.get('/reverseproxy/config');
export const getReverseProxyStatus = () => api.get('/reverseproxy/status');
export const getReverseProxyHosts = () => api.get('/reverseproxy/hosts');
export const addReverseProxyHost = (host) => api.post('/reverseproxy/hosts', host);
export const updateReverseProxyHost = (id, updates) => api.put(`/reverseproxy/hosts/${id}`, updates);
export const deleteReverseProxyHost = (id) => api.delete(`/reverseproxy/hosts/${id}`);
export const toggleReverseProxyHost = (id, enabled) => api.post(`/reverseproxy/hosts/${id}/toggle`, { enabled });
export const updateBaseDomain = (baseDomain) => api.put('/reverseproxy/config/domain', { baseDomain });
export const updateLocalNetworks = (networks) => api.put('/reverseproxy/config/networks', { networks });
export const renewCertificates = () => api.post('/reverseproxy/certificates/renew');
export const reloadProxy = () => api.post('/reverseproxy/reload');
export const getCertificatesStatus = () => api.get('/reverseproxy/certificates/status');

// Applications (Agent-based LXC)
export const getApplications = () => api.get('/applications');
export const createApplication = (app) => api.post('/applications', app);
export const updateApplication = (id, updates) => api.put(`/applications/${id}`, updates);
export const deleteApplication = (id) => api.delete(`/applications/${id}`);
export const toggleApplication = (id, enabled) => api.post(`/applications/${id}/toggle`, { enabled });

// Application Service Control (powersave)
export const startApplicationService = (appId, serviceType) =>
  api.post(`/applications/${appId}/services/${serviceType}/start`);
export const stopApplicationService = (appId, serviceType) =>
  api.post(`/applications/${appId}/services/${serviceType}/stop`);
// Application Migration
export const migrateApplication = (id, targetHostId) => api.post(`/applications/${id}/migrate`, { target_host_id: targetHostId });
export const getActiveMigrations = () => api.get('/applications/active-migrations');

// Rust Proxy
export const getRustProxyStatus = () => api.get('/rust-proxy/status');

// Auth - Session (login page)
export const login = (username, password, remember_me = false) => api.post('/auth/login', { username, password, remember_me });
export const logout = () => api.post('/auth/logout');
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
export const getCurrentEnergyMode = () => api.get('/energy/mode');
export const setEnergyMode = (mode) => api.post(`/energy/mode/${mode}`);

// Energy - Schedule
export const getEnergySchedule = () => api.get('/energy/schedule');
export const saveEnergySchedule = (config) => api.post('/energy/schedule', config);

// Energy - Auto-select
export const getAutoSelectConfig = () => api.get('/energy/autoselect');
export const saveAutoSelectConfig = (config) => api.post('/energy/autoselect', config);
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
export const createUserGroup = (data) => api.post('/users/groups', data);
export const updateUserGroup = (id, data) => api.put(`/users/groups/${id}`, data);
export const deleteUserGroup = (id) => api.delete(`/users/groups/${id}`);

export default api;

// ========== Hosts (unified servers + WoL) ==========

export const getHosts = () => api.get('/hosts');
export const addHost = (data) => api.post('/hosts', data);
export const updateHost = (id, data) => api.put(`/hosts/${id}`, data);
export const deleteHost = (id) => api.delete(`/hosts/${id}`);
export const testHostConnection = (id) => api.post(`/hosts/${id}/test`);
// Hosts - Power actions
export const wakeHost = (id) => api.post(`/hosts/${id}/wake`);
export const shutdownHost = (id) => api.post(`/hosts/${id}/shutdown`);
export const rebootHost = (id) => api.post(`/hosts/${id}/reboot`);
export const sleepHost = (id) => api.post(`/hosts/${id}/sleep`);
export const setWolMac = (id, mac) => api.post(`/hosts/${id}/wol-mac`, { mac });
export const setAutoOff = (id, mode, minutes) => api.post(`/hosts/${id}/auto-off`, { mode, minutes });
export const updateHostAgents = () => api.post('/hosts/agents/update');

// Cloud Relay
export const getCloudRelayStatus = () => api.get('/cloud-relay/status');
export const enableCloudRelay = () => api.post('/cloud-relay/enable');
export const disableCloudRelay = () => api.post('/cloud-relay/disable');
export const bootstrapCloudRelay = (data) => api.post('/cloud-relay/bootstrap', data, { timeout: 300000 });
export const updateCloudRelayConfig = (config) => api.put('/cloud-relay/config', config);

// Dataverse
export const getDataverseOverview = () => api.get('/dataverse/overview');
export const getDataverseSchema = (appId) => api.get(`/dataverse/apps/${appId}/schema`);
export const getDataverseTables = (appId) => api.get(`/dataverse/apps/${appId}/tables`);
export const getDataverseTable = (appId, tableName) => api.get(`/dataverse/apps/${appId}/tables/${tableName}`);
export const getDataverseRelations = (appId) => api.get(`/dataverse/apps/${appId}/relations`);
export const getDataverseStats = (appId) => api.get(`/dataverse/apps/${appId}/stats`);


