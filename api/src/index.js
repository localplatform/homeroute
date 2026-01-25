// Load env FIRST before any other imports
import dotenv from 'dotenv';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
dotenv.config({ path: path.resolve(__dirname, '../../.env') });

// Now import everything else
import express from 'express';
import cors from 'cors';
import cookieParser from 'cookie-parser';
import { createServer } from 'http';
import { Server } from 'socket.io';
import { setIO } from './socket.js';
import { authMiddleware } from './middleware/auth.js';
import { startScheduler as startDdnsScheduler } from './services/cloudflare.js';
import { initDatabase } from './services/authdb.js';

// Routes
import dnsRoutes from './routes/dns.js';
import networkRoutes from './routes/network.js';
import natRoutes from './routes/nat.js';
import adblockRoutes from './routes/adblock.js';
import ddnsRoutes from './routes/ddns.js';
import reverseproxyRoutes from './routes/reverseproxy.js';
import authRoutes from './routes/auth.js';
import forwardAuthRoutes from './routes/forward-auth.js';
import updatesRoutes from './routes/updates.js';
import energyRoutes from './routes/energy.js';
import usersRoutes from './routes/users.js';
import authproxyRoutes from './routes/authproxy.js';

const app = express();
const httpServer = createServer(app);
const io = new Server(httpServer, {
  cors: {
    origin: '*',
    methods: ['GET', 'POST']
  }
});
const PORT = process.env.PORT || 3001;

// Set io instance for use in other modules
setIO(io);

// Trust proxy (Caddy) pour les headers X-Forwarded-*
app.set('trust proxy', 1);

// Middleware
app.use(cors({
  origin: true,
  credentials: true
}));
app.use(express.json());
app.use(cookieParser());

// Demarrage async
async function startServer() {
  // Initialiser la base de donnees SQLite pour les sessions
  await initDatabase();

  // Middleware Auth - verifie le cookie auth_session localement
  app.use(authMiddleware);

  // Routes
  app.use('/api/auth', authRoutes);
  app.use('/api/authz', forwardAuthRoutes);
  app.use('/api/dns', dnsRoutes);
  app.use('/api/network', networkRoutes);
  app.use('/api/nat', natRoutes);
  app.use('/api/adblock', adblockRoutes);
  app.use('/api/ddns', ddnsRoutes);
  app.use('/api/reverseproxy', reverseproxyRoutes);
  app.use('/api/updates', updatesRoutes);
  app.use('/api/energy', energyRoutes);
  app.use('/api/users', usersRoutes);
  app.use('/api/authproxy', authproxyRoutes);

  // Health check
  app.get('/api/health', (req, res) => {
    res.json({ status: 'ok', timestamp: new Date().toISOString() });
  });

  // Servir les fichiers statiques du frontend en production
  const distPath = path.join(__dirname, '../../web/dist');
  app.use(express.static(distPath));

  // Fallback pour SPA routing
  app.get('*', (req, res) => {
    res.sendFile(path.join(distPath, 'index.html'));
  });

  httpServer.listen(PORT, () => {
    console.log(`API server running on http://localhost:${PORT}`);
  });

  // Démarrer le scheduler DDNS Cloudflare
  startDdnsScheduler();
}

startServer();
