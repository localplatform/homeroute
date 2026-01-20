const API_BASE = '/api';

async function request(endpoint, options = {}) {
  const url = `${API_BASE}${endpoint}`;

  const config = {
    credentials: 'include',
    headers: {
      'Content-Type': 'application/json',
      ...options.headers
    },
    ...options
  };

  if (options.body && typeof options.body === 'object') {
    config.body = JSON.stringify(options.body);
  }

  const response = await fetch(url, config);
  const data = await response.json();

  if (!response.ok) {
    throw new Error(data.error || 'Une erreur est survenue');
  }

  return data;
}

// Auth API
export const auth = {
  login: (username, password, remember_me = false) =>
    request('/auth/login', {
      method: 'POST',
      body: { username, password, remember_me }
    }),

  logout: () =>
    request('/auth/logout', { method: 'POST' }),

  check: () =>
    request('/auth/check'),

  me: () =>
    request('/auth/me'),

  getSessions: () =>
    request('/auth/sessions'),

  revokeSession: (id) =>
    request(`/auth/sessions/${id}`, { method: 'DELETE' })
};
