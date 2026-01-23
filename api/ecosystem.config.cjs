module.exports = {
  apps: [{
    name: 'server-dashboard-api',
    script: 'src/index.js',
    cwd: '/ssd_pool/server-dashboard/api',
    instances: 1,
    autorestart: true,
    watch: ['src'],
    watch_delay: 1000,
    ignore_watch: ['node_modules', 'logs'],
    max_memory_restart: '200M',
    env: {
      NODE_ENV: 'development'
    }
  }]
};
