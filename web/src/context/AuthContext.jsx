import { createContext, useContext, useState, useEffect, useCallback } from 'react';
import { getMe, logout as apiLogout, login as apiLogin } from '../api/client';

const AuthContext = createContext(null);

export function AuthProvider({ children }) {
  const [user, setUser] = useState(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [isAuthenticated, setIsAuthenticated] = useState(false);

  const checkAuth = useCallback(async () => {
    try {
      const res = await getMe();

      if (res.data.success && res.data.user) {
        setUser(res.data.user);
        setIsAuthenticated(true);
        setError(null);
      } else {
        // Session expired or invalid - ensure clean state
        setUser(null);
        setIsAuthenticated(false);

        // If session expired, the interceptor already cleared the cookie
        // but we log it for debugging
        if (res.data.error === 'Session expiree') {
          console.log('Session expired, cookie cleared');
        }
      }
    } catch (err) {
      console.error('Auth check failed:', err);
      setUser(null);
      setIsAuthenticated(false);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    checkAuth();
  }, [checkAuth]);

  const login = useCallback(async (username, password, rememberMe = false) => {
    try {
      const res = await apiLogin(username, password, rememberMe);

      if (res.data.success) {
        setUser(res.data.user);
        setIsAuthenticated(true);
        setError(null);
        return { success: true };
      } else {
        const errorMsg = res.data.error || 'Identifiants invalides';
        setError(errorMsg);
        throw new Error(errorMsg);
      }
    } catch (err) {
      const errorMsg = err.response?.data?.error || err.message || 'Erreur de connexion';
      setError(errorMsg);
      throw new Error(errorMsg);
    }
  }, []);

  const logout = useCallback(async () => {
    try {
      await apiLogout();
      setUser(null);
      setIsAuthenticated(false);
    } catch (err) {
      console.error('Logout failed:', err);
    }
  }, []);

  const value = {
    user,
    loading,
    error,
    isAuthenticated,
    login,
    logout,
    checkAuth
  };

  return (
    <AuthContext.Provider value={value}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth() {
  const context = useContext(AuthContext);
  if (!context) {
    throw new Error('useAuth must be used within an AuthProvider');
  }
  return context;
}

export default AuthContext;
