module.exports = {
  apps: [
    {
      name: 'dashboard-api',
      cwd: '/ssd_pool/server-dashboard/api',
      script: 'npx',
      args: 'nodemon src/index.js',
      watch: false,
      env: {
        NODE_ENV: 'development'
      }
    },
    {
      name: 'dashboard-web',
      cwd: '/ssd_pool/server-dashboard/web',
      script: 'npx',
      args: 'vite',
      watch: false,
      env: {
        NODE_ENV: 'development'
      }
    },
    {
      name: 'auth-api',
      cwd: '/ssd_pool/server-dashboard/auth-api',
      script: 'npx',
      args: 'nodemon src/index.js',
      watch: false,
      env: {
        NODE_ENV: 'development',
        AUTH_PORT: 9100,
        AUTH_DATA_DIR: '/ssd_pool/auth-service/data'
      }
    },
    {
      name: 'auth-web',
      cwd: '/ssd_pool/server-dashboard/auth-web',
      script: 'npx',
      args: 'vite --port 5174',
      watch: false,
      env: {
        NODE_ENV: 'development'
      }
    }
  ]
};
