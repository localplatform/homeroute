import cron from 'node-cron';
import { getCollection, COLLECTIONS } from './mongodb.js';

let aggregationJobs = [];

/**
 * Start aggregation jobs
 */
export function startAggregation() {
  console.log('Starting traffic aggregation jobs...');

  // Every 5 minutes: aggregate to hourly
  const hourlyJob = cron.schedule('*/5 * * * *', async () => {
    console.log('Running hourly aggregation...');
    await aggregateToHourly();
  });

  // Every day at 00:30: aggregate to daily and cleanup
  const dailyJob = cron.schedule('30 0 * * *', async () => {
    console.log('Running daily aggregation and cleanup...');
    await aggregateToDaily();
    await cleanupOldData();
  });

  aggregationJobs.push(hourlyJob, dailyJob);

  console.log('✓ Aggregation jobs started (hourly every 5min, daily at 00:30)');
}

/**
 * Stop aggregation jobs
 */
export function stopAggregation() {
  aggregationJobs.forEach(job => job.stop());
  aggregationJobs = [];
  console.log('Aggregation jobs stopped');
}

/**
 * Aggregate HTTP traffic to hourly collection
 */
async function aggregateToHourly() {
  try {
    const httpCollection = getCollection(COLLECTIONS.TRAFFIC_HTTP);
    const hourlyCollection = getCollection(COLLECTIONS.TRAFFIC_HOURLY);

    // Get data from last hour
    const oneHourAgo = new Date(Date.now() - 3600000);

    const result = await httpCollection.aggregate([
      { $match: { timestamp: { $gte: oneHourAgo } } },
      {
        $group: {
          _id: {
            hour: {
              $dateTrunc: {
                date: '$timestamp',
                unit: 'hour',
                timezone: 'UTC'
              }
            },
            deviceMac: '$meta.deviceMac',
            endpoint: '$meta.endpoint',
            application: '$meta.application',
            environment: '$meta.environment'
          },
          totalRequests: { $sum: '$metrics.requestCount' },
          totalBytes: { $sum: '$metrics.responseBytes' },
          avgResponseTime: { $avg: '$metrics.responseTimeMs' },
          count: { $sum: 1 }
        }
      },
      {
        $project: {
          _id: 0,
          timestamp: '$_id.hour',
          meta: {
            deviceMac: '$_id.deviceMac',
            endpoint: '$_id.endpoint',
            application: '$_id.application',
            environment: '$_id.environment'
          },
          metrics: {
            totalRequests: '$totalRequests',
            totalBytes: '$totalBytes',
            avgResponseTime: '$avgResponseTime',
            eventCount: '$count'
          }
        }
      }
    ]).toArray();

    if (result.length > 0) {
      // Use bulk operations for better performance
      const bulkOps = result.map(doc => ({
        updateOne: {
          filter: {
            timestamp: doc.timestamp,
            'meta.deviceMac': doc.meta.deviceMac,
            'meta.endpoint': doc.meta.endpoint,
            'meta.application': doc.meta.application
          },
          update: { $set: doc },
          upsert: true
        }
      }));

      await hourlyCollection.bulkWrite(bulkOps);
      console.log(`✓ Aggregated ${result.length} hourly records`);
    }
  } catch (error) {
    console.error('Error in hourly aggregation:', error);
  }
}

/**
 * Aggregate hourly data to daily collection
 */
async function aggregateToDaily() {
  try {
    const hourlyCollection = getCollection(COLLECTIONS.TRAFFIC_HOURLY);
    const dailyCollection = getCollection(COLLECTIONS.TRAFFIC_DAILY);

    // Get data from last 2 days (to handle timezone issues)
    const twoDaysAgo = new Date(Date.now() - 2 * 86400000);

    const result = await hourlyCollection.aggregate([
      { $match: { timestamp: { $gte: twoDaysAgo } } },
      {
        $group: {
          _id: {
            day: {
              $dateTrunc: {
                date: '$timestamp',
                unit: 'day',
                timezone: 'UTC'
              }
            },
            deviceMac: '$meta.deviceMac',
            endpoint: '$meta.endpoint',
            application: '$meta.application',
            environment: '$meta.environment'
          },
          totalRequests: { $sum: '$metrics.totalRequests' },
          totalBytes: { $sum: '$metrics.totalBytes' },
          avgResponseTime: { $avg: '$metrics.avgResponseTime' },
          hourlyRecordCount: { $sum: 1 }
        }
      },
      {
        $project: {
          _id: 0,
          timestamp: '$_id.day',
          meta: {
            deviceMac: '$_id.deviceMac',
            endpoint: '$_id.endpoint',
            application: '$_id.application',
            environment: '$_id.environment'
          },
          metrics: {
            totalRequests: '$totalRequests',
            totalBytes: '$totalBytes',
            avgResponseTime: '$avgResponseTime',
            hourlyRecordCount: '$hourlyRecordCount'
          }
        }
      }
    ]).toArray();

    if (result.length > 0) {
      const bulkOps = result.map(doc => ({
        updateOne: {
          filter: {
            timestamp: doc.timestamp,
            'meta.deviceMac': doc.meta.deviceMac,
            'meta.endpoint': doc.meta.endpoint,
            'meta.application': doc.meta.application
          },
          update: { $set: doc },
          upsert: true
        }
      }));

      await dailyCollection.bulkWrite(bulkOps);
      console.log(`✓ Aggregated ${result.length} daily records`);
    }
  } catch (error) {
    console.error('Error in daily aggregation:', error);
  }
}

/**
 * Cleanup old aggregated data
 */
async function cleanupOldData() {
  try {
    const hourlyCollection = getCollection(COLLECTIONS.TRAFFIC_HOURLY);
    const dailyCollection = getCollection(COLLECTIONS.TRAFFIC_DAILY);

    // Delete hourly data older than 90 days
    const ninetyDaysAgo = new Date(Date.now() - 90 * 86400000);
    const hourlyResult = await hourlyCollection.deleteMany({
      timestamp: { $lt: ninetyDaysAgo }
    });

    // Delete daily data older than 1 year
    const oneYearAgo = new Date(Date.now() - 365 * 86400000);
    const dailyResult = await dailyCollection.deleteMany({
      timestamp: { $lt: oneYearAgo }
    });

    console.log(`✓ Cleanup: removed ${hourlyResult.deletedCount} old hourly records, ${dailyResult.deletedCount} old daily records`);
  } catch (error) {
    console.error('Error in cleanup:', error);
  }
}

// Manual aggregation functions (can be called via API if needed)
export { aggregateToHourly, aggregateToDaily, cleanupOldData };

// Graceful shutdown
process.on('SIGINT', stopAggregation);
process.on('SIGTERM', stopAggregation);
