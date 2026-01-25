import express from 'express';
import {
  parseTimeRange,
  getTrafficOverview,
  getTimeseries,
  getTopDevices,
  getTopEndpoints,
  getTopApplications,
  getDeviceTraffic,
  getCurrentMetrics,
  trafficEvents
} from '../services/traffic.js';
import { getCurrentNetworkMetrics } from '../services/trafficNetworkCapture.js';
import { getCollection, COLLECTIONS } from '../services/mongodb.js';

const router = express.Router();

/**
 * GET /api/traffic/overview
 * Get traffic overview with summary statistics
 */
router.get('/overview', async (req, res) => {
  try {
    const timeRange = req.query.timeRange || '24h';
    const overview = await getTrafficOverview(timeRange);

    res.json({
      success: true,
      data: overview
    });
  } catch (error) {
    console.error('Error getting traffic overview:', error);
    res.status(500).json({
      success: false,
      error: error.message
    });
  }
});

/**
 * GET /api/traffic/timeseries
 * Get time-series data for charts
 */
router.get('/timeseries', async (req, res) => {
  try {
    const { metric = 'requests', granularity = 'hour', timeRange = '24h' } = req.query;

    const data = await getTimeseries({ metric, granularity, timeRange });

    res.json({
      success: true,
      data
    });
  } catch (error) {
    console.error('Error getting timeseries:', error);
    res.status(500).json({
      success: false,
      error: error.message
    });
  }
});

/**
 * GET /api/traffic/by-device
 * Get traffic breakdown by device
 */
router.get('/by-device', async (req, res) => {
  try {
    const timeRange = req.query.timeRange || '24h';
    const limit = parseInt(req.query.limit || '10');

    const devices = await getTopDevices({ timeRange, limit });

    res.json({
      success: true,
      data: devices
    });
  } catch (error) {
    console.error('Error getting devices:', error);
    res.status(500).json({
      success: false,
      error: error.message
    });
  }
});

/**
 * GET /api/traffic/by-endpoint
 * Get traffic breakdown by endpoint
 */
router.get('/by-endpoint', async (req, res) => {
  try {
    const timeRange = req.query.timeRange || '24h';
    const limit = parseInt(req.query.limit || '10');

    const endpoints = await getTopEndpoints({ timeRange, limit });

    res.json({
      success: true,
      data: endpoints
    });
  } catch (error) {
    console.error('Error getting endpoints:', error);
    res.status(500).json({
      success: false,
      error: error.message
    });
  }
});

/**
 * GET /api/traffic/by-application
 * Get traffic breakdown by application
 */
router.get('/by-application', async (req, res) => {
  try {
    const timeRange = req.query.timeRange || '24h';

    const applications = await getTopApplications({ timeRange });

    res.json({
      success: true,
      data: applications
    });
  } catch (error) {
    console.error('Error getting applications:', error);
    res.status(500).json({
      success: false,
      error: error.message
    });
  }
});

/**
 * GET /api/traffic/device/:mac
 * Get detailed traffic for a specific device
 */
router.get('/device/:mac', async (req, res) => {
  try {
    const { mac } = req.params;
    const timeRange = req.query.timeRange || '7d';

    const deviceData = await getDeviceTraffic({ mac, timeRange });

    res.json({
      success: true,
      data: deviceData
    });
  } catch (error) {
    console.error('Error getting device traffic:', error);
    res.status(500).json({
      success: false,
      error: error.message
    });
  }
});

/**
 * GET /api/traffic/dns/top-domains
 * Get top DNS queries by domain
 */
router.get('/dns/top-domains', async (req, res) => {
  try {
    const timeRange = req.query.timeRange || '24h';
    const limit = parseInt(req.query.limit || '20');

    const { start, end } = parseTimeRange(timeRange);

    const db = getCollection(COLLECTIONS.TRAFFIC_HTTP).s.db;
    const dnsCollection = db.collection('traffic_dns');

    const topDomains = await dnsCollection.aggregate([
      {
        $match: {
          timestamp: { $gte: start, $lte: end }
        }
      },
      {
        $group: {
          _id: '$meta.domain',
          category: { $first: '$meta.category' },
          totalQueries: { $sum: '$metrics.queryCount' },
          devices: { $addToSet: '$meta.deviceMac' }
        }
      },
      { $sort: { totalQueries: -1 } },
      { $limit: limit },
      {
        $project: {
          _id: 0,
          domain: '$_id',
          category: 1,
          totalQueries: 1,
          uniqueDevices: { $size: { $filter: { input: '$devices', as: 'd', cond: { $ne: ['$$d', null] } } } }
        }
      }
    ]).toArray();

    res.json({
      success: true,
      data: topDomains
    });
  } catch (error) {
    console.error('Error getting top domains:', error);
    res.status(500).json({
      success: false,
      error: error.message
    });
  }
});

/**
 * GET /api/traffic/dns/by-category
 * Get DNS queries breakdown by category
 */
router.get('/dns/by-category', async (req, res) => {
  try {
    const timeRange = req.query.timeRange || '24h';
    const { start, end } = parseTimeRange(timeRange);

    const db = getCollection(COLLECTIONS.TRAFFIC_HTTP).s.db;
    const dnsCollection = db.collection('traffic_dns');

    const categories = await dnsCollection.aggregate([
      {
        $match: {
          timestamp: { $gte: start, $lte: end }
        }
      },
      {
        $group: {
          _id: '$meta.category',
          totalQueries: { $sum: '$metrics.queryCount' }
        }
      },
      { $sort: { totalQueries: -1 } },
      {
        $project: {
          _id: 0,
          category: '$_id',
          totalQueries: 1
        }
      }
    ]).toArray();

    res.json({
      success: true,
      data: categories
    });
  } catch (error) {
    console.error('Error getting DNS categories:', error);
    res.status(500).json({
      success: false,
      error: error.message
    });
  }
});

/**
 * GET /api/traffic/events
 * Server-Sent Events endpoint for real-time updates
 */
router.get('/events', (req, res) => {
  // Set SSE headers
  res.setHeader('Content-Type', 'text/event-stream');
  res.setHeader('Cache-Control', 'no-cache');
  res.setHeader('Connection', 'keep-alive');
  res.setHeader('X-Accel-Buffering', 'no'); // Disable nginx buffering

  // Send initial connection success
  res.write('event: connected\ndata: {"status":"connected"}\n\n');

  // Send heartbeat every 30 seconds
  const heartbeatInterval = setInterval(() => {
    res.write('event: heartbeat\ndata: {"timestamp":"' + new Date().toISOString() + '"}\n\n');
  }, 30000);

  // Handler for traffic updates (HTTP)
  const onTrafficUpdate = (data) => {
    res.write(`event: trafficUpdate\ndata: ${JSON.stringify(data)}\n\n`);
  };

  // Handler for network updates
  const onNetworkUpdate = (data) => {
    res.write(`event: networkUpdate\ndata: ${JSON.stringify(data)}\n\n`);
  };

  // Handler for DNS updates
  const onDnsUpdate = (data) => {
    res.write(`event: dnsUpdate\ndata: ${JSON.stringify(data)}\n\n`);
  };

  // Register event listeners
  trafficEvents.on('trafficUpdate', onTrafficUpdate);
  trafficEvents.on('networkUpdate', onNetworkUpdate);
  trafficEvents.on('dnsUpdate', onDnsUpdate);

  // Cleanup on client disconnect
  req.on('close', () => {
    clearInterval(heartbeatInterval);
    trafficEvents.off('trafficUpdate', onTrafficUpdate);
    trafficEvents.off('networkUpdate', onNetworkUpdate);
    trafficEvents.off('dnsUpdate', onDnsUpdate);
    res.end();
  });
});

export default router;
