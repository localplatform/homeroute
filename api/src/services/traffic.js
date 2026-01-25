import { EventEmitter } from 'events';
import { getCollection, COLLECTIONS } from './mongodb.js';

// EventEmitter for real-time updates
export const trafficEvents = new EventEmitter();

/**
 * Parse time range parameter
 */
export function parseTimeRange(range) {
  const now = new Date();

  const presets = {
    '1h': 3600000,
    '24h': 86400000,
    '7d': 604800000,
    '30d': 2592000000
  };

  if (presets[range]) {
    const duration = presets[range];
    return {
      start: new Date(now - duration),
      end: now,
      granularity: range === '1h' ? 'minute' : 'hour'
    };
  }

  // Default to 24h if invalid
  return {
    start: new Date(now - 86400000),
    end: now,
    granularity: 'hour'
  };
}

/**
 * Get traffic overview
 */
export async function getTrafficOverview(timeRange = '24h') {
  const { start, end } = parseTimeRange(timeRange);

  const httpCollection = getCollection(COLLECTIONS.TRAFFIC_HTTP);

  // Get total requests and bytes
  const httpStats = await httpCollection.aggregate([
    { $match: { timestamp: { $gte: start, $lte: end } } },
    {
      $group: {
        _id: null,
        totalRequests: { $sum: '$metrics.requestCount' },
        totalBytes: { $sum: '$metrics.responseBytes' },
        uniqueDevices: { $addToSet: '$meta.deviceMac' },
        uniqueEndpoints: { $addToSet: '$meta.endpoint' }
      }
    }
  ]).toArray();

  const stats = httpStats[0] || {
    totalRequests: 0,
    totalBytes: 0,
    uniqueDevices: [],
    uniqueEndpoints: []
  };

  return {
    totalRequests: stats.totalRequests,
    totalBytes: stats.totalBytes,
    uniqueDevices: stats.uniqueDevices.filter(Boolean).length,
    uniqueEndpoints: stats.uniqueEndpoints.filter(Boolean).length,
    timeRange,
    start: start.toISOString(),
    end: end.toISOString()
  };
}

/**
 * Get time-series data
 */
export async function getTimeseries({ metric = 'requests', granularity = 'hour', timeRange = '24h' }) {
  const { start, end } = parseTimeRange(timeRange);

  const httpCollection = getCollection(COLLECTIONS.TRAFFIC_HTTP);

  // Determine the metric field
  const metricField = metric === 'requests' ? '$metrics.requestCount' : '$metrics.responseBytes';

  // Determine granularity
  const unit = granularity === 'minute' ? 'minute' : 'hour';

  const pipeline = [
    { $match: { timestamp: { $gte: start, $lte: end } } },
    {
      $group: {
        _id: {
          $dateTrunc: {
            date: '$timestamp',
            unit,
            timezone: 'UTC'
          }
        },
        value: { $sum: metricField }
      }
    },
    { $sort: { _id: 1 } },
    {
      $project: {
        _id: 0,
        timestamp: '$_id',
        value: 1
      }
    }
  ];

  const result = await httpCollection.aggregate(pipeline).toArray();

  return result;
}

/**
 * Get top devices by traffic
 */
export async function getTopDevices({ timeRange = '24h', limit = 10 }) {
  const { start, end } = parseTimeRange(timeRange);

  const httpCollection = getCollection(COLLECTIONS.TRAFFIC_HTTP);

  const result = await httpCollection.aggregate([
    {
      $match: {
        timestamp: { $gte: start, $lte: end },
        'meta.deviceMac': { $ne: null }
      }
    },
    {
      $group: {
        _id: '$meta.deviceMac',
        hostname: { $first: '$meta.deviceHostname' },
        ip: { $first: '$meta.deviceIp' },
        totalRequests: { $sum: '$metrics.requestCount' },
        totalBytes: { $sum: '$metrics.responseBytes' }
      }
    },
    { $sort: { totalBytes: -1 } },
    { $limit: limit },
    {
      $project: {
        _id: 0,
        mac: '$_id',
        hostname: { $ifNull: ['$hostname', '$ip'] },
        totalRequests: 1,
        totalBytes: 1
      }
    }
  ]).toArray();

  return result;
}

/**
 * Get top endpoints by traffic
 */
export async function getTopEndpoints({ timeRange = '24h', limit = 10 }) {
  const { start, end } = parseTimeRange(timeRange);

  const httpCollection = getCollection(COLLECTIONS.TRAFFIC_HTTP);

  const result = await httpCollection.aggregate([
    {
      $match: {
        timestamp: { $gte: start, $lte: end },
        'meta.endpoint': { $ne: null }
      }
    },
    {
      $group: {
        _id: '$meta.endpoint',
        totalRequests: { $sum: '$metrics.requestCount' },
        totalBytes: { $sum: '$metrics.responseBytes' },
        applications: { $addToSet: '$meta.application' }
      }
    },
    { $sort: { totalRequests: -1 } },
    { $limit: limit },
    {
      $project: {
        _id: 0,
        endpoint: '$_id',
        totalRequests: 1,
        totalBytes: 1,
        applications: {
          $filter: {
            input: '$applications',
            as: 'app',
            cond: { $ne: ['$$app', null] }
          }
        }
      }
    }
  ]).toArray();

  return result;
}

/**
 * Get traffic breakdown by application
 */
export async function getTopApplications({ timeRange = '24h' }) {
  const { start, end } = parseTimeRange(timeRange);

  const httpCollection = getCollection(COLLECTIONS.TRAFFIC_HTTP);

  const result = await httpCollection.aggregate([
    {
      $match: {
        timestamp: { $gte: start, $lte: end },
        'meta.application': { $ne: null }
      }
    },
    {
      $group: {
        _id: {
          application: '$meta.application',
          environment: '$meta.environment'
        },
        totalRequests: { $sum: '$metrics.requestCount' },
        totalBytes: { $sum: '$metrics.responseBytes' },
        topPaths: {
          $push: {
            path: '$meta.path',
            count: '$metrics.requestCount'
          }
        }
      }
    },
    { $sort: { totalBytes: -1 } },
    {
      $project: {
        _id: 0,
        application: '$_id.application',
        environment: '$_id.environment',
        totalRequests: 1,
        totalBytes: 1,
        topPaths: { $slice: ['$topPaths', 5] }
      }
    }
  ]).toArray();

  return result;
}

/**
 * Get detailed traffic for a specific device
 */
export async function getDeviceTraffic({ mac, timeRange = '7d' }) {
  const { start, end } = parseTimeRange(timeRange);

  const httpCollection = getCollection(COLLECTIONS.TRAFFIC_HTTP);

  // Get device timeline
  const timeline = await httpCollection.aggregate([
    {
      $match: {
        timestamp: { $gte: start, $lte: end },
        'meta.deviceMac': mac
      }
    },
    {
      $group: {
        _id: {
          $dateTrunc: {
            date: '$timestamp',
            unit: 'hour',
            timezone: 'UTC'
          }
        },
        requests: { $sum: '$metrics.requestCount' },
        bytes: { $sum: '$metrics.responseBytes' }
      }
    },
    { $sort: { _id: 1 } },
    {
      $project: {
        _id: 0,
        timestamp: '$_id',
        requests: 1,
        bytes: 1
      }
    }
  ]).toArray();

  // Get top endpoints for this device
  const topEndpoints = await httpCollection.aggregate([
    {
      $match: {
        timestamp: { $gte: start, $lte: end },
        'meta.deviceMac': mac,
        'meta.endpoint': { $ne: null }
      }
    },
    {
      $group: {
        _id: '$meta.endpoint',
        requests: { $sum: '$metrics.requestCount' },
        bytes: { $sum: '$metrics.responseBytes' }
      }
    },
    { $sort: { bytes: -1 } },
    { $limit: 10 },
    {
      $project: {
        _id: 0,
        endpoint: '$_id',
        requests: 1,
        bytes: 1
      }
    }
  ]).toArray();

  // Get top applications for this device
  const topApplications = await httpCollection.aggregate([
    {
      $match: {
        timestamp: { $gte: start, $lte: end },
        'meta.deviceMac': mac,
        'meta.application': { $ne: null }
      }
    },
    {
      $group: {
        _id: '$meta.application',
        requests: { $sum: '$metrics.requestCount' },
        bytes: { $sum: '$metrics.responseBytes' }
      }
    },
    { $sort: { bytes: -1 } },
    { $limit: 10 },
    {
      $project: {
        _id: 0,
        application: '$_id',
        requests: 1,
        bytes: 1
      }
    }
  ]).toArray();

  // Get device info
  const deviceInfo = await httpCollection.findOne({
    'meta.deviceMac': mac
  }, {
    projection: {
      'meta.deviceHostname': 1,
      'meta.deviceIp': 1
    },
    sort: { timestamp: -1 }
  });

  return {
    device: {
      mac,
      hostname: deviceInfo?.meta?.deviceHostname || null,
      ip: deviceInfo?.meta?.deviceIp || null
    },
    timeline,
    topEndpoints,
    topApplications
  };
}

/**
 * Get current real-time metrics
 */
export function getCurrentMetrics() {
  // This will be populated by the capture services
  return {
    rps: 0,
    bandwidthMbps: 0,
    timestamp: new Date().toISOString()
  };
}
