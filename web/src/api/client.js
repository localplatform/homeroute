import axios from 'axios';

const api = axios.create({
  baseURL: '/api',
  timeout: 30000,
  withCredentials: true  // Enable cookies for session-based auth
});

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

// Backup SMB
export const getBackupConfig = () => api.get('/backup/config');
export const saveBackupConfig = (config) => api.post('/backup/config', config);
export const runBackup = () => api.post('/backup/run', {}, { timeout: 3600000 });
export const getBackupHistory = () => api.get('/backup/history');
export const testBackupConnection = () => api.post('/backup/test');
export const cancelBackup = () => api.post('/backup/cancel');
export const getBackupStatus = () => api.get('/backup/status');
export const wakeBackupServer = () => api.post('/backup/wake');
export const getBackupServerStatus = () => api.get('/backup/server-status');
export const getRemoteBackups = (path = '') => api.get('/backup/remote', { params: { path } });
export const deleteRemoteItem = (path) => api.delete('/backup/remote', { data: { path } });
export const shutdownBackupServer = () => api.post('/backup/shutdown');

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
export const getSystemRouteStatus = () => api.get('/reverseproxy/system-route');
export const getCertificatesStatus = () => api.get('/reverseproxy/certificates/status');

// Auth - Session (login page)
export const login = (username, password) => api.post('/auth/login', { username, password });
export const logout = () => api.post('/auth/logout');
export const checkAuth = () => api.get('/auth/check');
export const getMe = () => api.get('/auth/me');

// Samba - Configuration
export const getSambaConfig = () => api.get('/samba/config');
export const updateSambaGlobalConfig = (config) => api.put('/samba/config', config);

// Samba - Service Status
export const getSambaStatus = () => api.get('/samba/status');
export const restartSamba = () => api.post('/samba/restart');
export const reloadSamba = () => api.post('/samba/reload');

// Samba - Shares CRUD
export const getSambaShares = () => api.get('/samba/shares');
export const getSambaShare = (id) => api.get(`/samba/shares/${id}`);
export const addSambaShare = (share) => api.post('/samba/shares', share);
export const updateSambaShare = (id, updates) => api.put(`/samba/shares/${id}`, updates);
export const deleteSambaShare = (id) => api.delete(`/samba/shares/${id}`);
export const toggleSambaShare = (id, enabled) => api.post(`/samba/shares/${id}/toggle`, { enabled });

// Samba - Apply Configuration
export const applySambaConfig = () => api.post('/samba/apply');
export const testSambaConfig = () => api.post('/samba/testparm');
export const previewSambaConfig = () => api.get('/samba/preview');
export const importSambaShares = () => api.post('/samba/import');

// Samba - Monitoring
export const getSambaSessions = () => api.get('/samba/sessions');
export const getSambaOpenFiles = () => api.get('/samba/files');
export const getSambaShareConnections = (shareName) => api.get(`/samba/connections/${shareName}`);

// Samba - Users
export const getSambaUsers = () => api.get('/samba/users');
export const addSambaUser = (username, password) => api.post('/samba/users', { username, password });
export const deleteSambaUser = (username) => api.delete(`/samba/users/${username}`);
export const changeSambaUserPassword = (username, password) => api.put(`/samba/users/${username}/password`, { password });
export const enableSambaUser = (username) => api.post(`/samba/users/${username}/enable`);
export const disableSambaUser = (username) => api.post(`/samba/users/${username}/disable`);

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
export const getEnergyStatus = () => api.get('/energy/status');
export const setEnergyGovernor = (governor) => api.post('/energy/governor', { governor });

// Energy - Modes (unified: economy/auto/performance)
export const getEnergyModes = () => api.get('/energy/modes');
export const getCurrentEnergyMode = () => api.get('/energy/mode');
export const setEnergyMode = (mode) => api.post(`/energy/mode/${mode}`);

// Energy - Fans
export const getFansStatus = () => api.get('/energy/fans');
export const setFanSpeed = (id, pwm, mode) => api.post(`/energy/fans/${id}`, { pwm, mode });
export const getFanProfiles = () => api.get('/energy/fans/profiles');
export const saveFanProfile = (profile) => api.post('/energy/fans/profiles', profile);
export const applyFanProfile = (name) => api.post(`/energy/fans/profiles/${name}/apply`);

// Energy - Schedule
export const getEnergySchedule = () => api.get('/energy/schedule');
export const saveEnergySchedule = (config) => api.post('/energy/schedule', config);

// Energy - Auto-select
export const getAutoSelectConfig = () => api.get('/energy/autoselect');
export const saveAutoSelectConfig = (config) => api.post('/energy/autoselect', config);
export const getNetworkRps = () => api.get('/energy/autoselect/rps');
export const getAutoSelectStatus = () => api.get('/energy/autoselect/status');

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

// Users - MFA
export const getUserMfa = (username) => api.get(`/users/${username}/mfa`);
export const resetUserTotp = (username) => api.delete(`/users/${username}/mfa/totp`);
export const resetUserWebauthn = (username) => api.delete(`/users/${username}/mfa/webauthn`);

export default api;
