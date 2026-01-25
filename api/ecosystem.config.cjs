require('dotenv').config({ path: '/opt/homeroute/.env' });

module.exports = {
  apps: [{
    name: 'homeroute-api',
    script: 'src/index.js',
    cwd: '/opt/homeroute/api',
    instances: 1,
    autorestart: true,
    watch: false,
    max_memory_restart: '200M'
  }]
};
