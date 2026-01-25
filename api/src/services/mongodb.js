import { MongoClient } from 'mongodb';

// Singleton connection
let client = null;
let db = null;

const MONGODB_URI = process.env.MONGODB_URI || 'mongodb://localhost:27017';
const MONGODB_DATABASE = process.env.MONGODB_DATABASE || 'homeroute_analytics';
const MAX_POOL_SIZE = parseInt(process.env.MONGODB_MAX_POOL_SIZE || '10');

// Collection names
export const COLLECTIONS = {
  TRAFFIC_HTTP: 'traffic_http',
  TRAFFIC_NETWORK: 'traffic_network',
  TRAFFIC_HOURLY: 'traffic_hourly',
  TRAFFIC_DAILY: 'traffic_daily'
};

/**
 * Initialize MongoDB connection and collections
 */
export async function initMongoDB() {
  try {
    console.log(`Connecting to MongoDB at ${MONGODB_URI}...`);

    client = new MongoClient(MONGODB_URI, {
      maxPoolSize: MAX_POOL_SIZE,
      minPoolSize: 2,
      serverSelectionTimeoutMS: 5000,
      socketTimeoutMS: 45000,
    });

    await client.connect();
    db = client.db(MONGODB_DATABASE);

    console.log(`✓ Connected to MongoDB database: ${MONGODB_DATABASE}`);

    // Initialize collections and indexes
    await initCollections();

    console.log('✓ MongoDB collections and indexes initialized');

    // Setup connection monitoring
    setupConnectionMonitoring();

    return db;
  } catch (error) {
    console.error('MongoDB connection error:', error);
    throw error;
  }
}

/**
 * Initialize collections with time-series configuration and indexes
 */
async function initCollections() {
  const collections = await db.listCollections().toArray();
  const collectionNames = collections.map(c => c.name);

  // Create traffic_http as time-series collection
  if (!collectionNames.includes(COLLECTIONS.TRAFFIC_HTTP)) {
    await db.createCollection(COLLECTIONS.TRAFFIC_HTTP, {
      timeseries: {
        timeField: 'timestamp',
        metaField: 'meta',
        granularity: 'seconds'
      },
      expireAfterSeconds: 2592000 // 30 days TTL
    });
    console.log(`  ✓ Created time-series collection: ${COLLECTIONS.TRAFFIC_HTTP}`);
  }

  // Create traffic_network as time-series collection
  if (!collectionNames.includes(COLLECTIONS.TRAFFIC_NETWORK)) {
    await db.createCollection(COLLECTIONS.TRAFFIC_NETWORK, {
      timeseries: {
        timeField: 'timestamp',
        metaField: 'meta',
        granularity: 'seconds'
      },
      expireAfterSeconds: 2592000 // 30 days TTL
    });
    console.log(`  ✓ Created time-series collection: ${COLLECTIONS.TRAFFIC_NETWORK}`);
  }

  // Create standard collections for aggregates
  if (!collectionNames.includes(COLLECTIONS.TRAFFIC_HOURLY)) {
    await db.createCollection(COLLECTIONS.TRAFFIC_HOURLY);
    console.log(`  ✓ Created collection: ${COLLECTIONS.TRAFFIC_HOURLY}`);
  }

  if (!collectionNames.includes(COLLECTIONS.TRAFFIC_DAILY)) {
    await db.createCollection(COLLECTIONS.TRAFFIC_DAILY);
    console.log(`  ✓ Created collection: ${COLLECTIONS.TRAFFIC_DAILY}`);
  }

  // Create indexes for hourly collection
  const hourlyIndexes = await db.collection(COLLECTIONS.TRAFFIC_HOURLY).listIndexes().toArray();
  const hourlyIndexNames = hourlyIndexes.map(i => i.name);

  if (!hourlyIndexNames.includes('timestamp_-1')) {
    await db.collection(COLLECTIONS.TRAFFIC_HOURLY).createIndex(
      { timestamp: -1 },
      { name: 'timestamp_-1' }
    );
    console.log(`  ✓ Created index on ${COLLECTIONS.TRAFFIC_HOURLY}: timestamp_-1`);
  }

  if (!hourlyIndexNames.includes('deviceMac_1_timestamp_-1')) {
    await db.collection(COLLECTIONS.TRAFFIC_HOURLY).createIndex(
      { 'meta.deviceMac': 1, timestamp: -1 },
      { name: 'deviceMac_1_timestamp_-1' }
    );
    console.log(`  ✓ Created index on ${COLLECTIONS.TRAFFIC_HOURLY}: deviceMac_1_timestamp_-1`);
  }

  if (!hourlyIndexNames.includes('endpoint_1_timestamp_-1')) {
    await db.collection(COLLECTIONS.TRAFFIC_HOURLY).createIndex(
      { 'meta.endpoint': 1, timestamp: -1 },
      { name: 'endpoint_1_timestamp_-1' }
    );
    console.log(`  ✓ Created index on ${COLLECTIONS.TRAFFIC_HOURLY}: endpoint_1_timestamp_-1`);
  }

  if (!hourlyIndexNames.includes('application_1_timestamp_-1')) {
    await db.collection(COLLECTIONS.TRAFFIC_HOURLY).createIndex(
      { 'meta.application': 1, timestamp: -1 },
      { name: 'application_1_timestamp_-1' }
    );
    console.log(`  ✓ Created index on ${COLLECTIONS.TRAFFIC_HOURLY}: application_1_timestamp_-1`);
  }

  // Create indexes for daily collection
  const dailyIndexes = await db.collection(COLLECTIONS.TRAFFIC_DAILY).listIndexes().toArray();
  const dailyIndexNames = dailyIndexes.map(i => i.name);

  if (!dailyIndexNames.includes('timestamp_-1')) {
    await db.collection(COLLECTIONS.TRAFFIC_DAILY).createIndex(
      { timestamp: -1 },
      { name: 'timestamp_-1' }
    );
    console.log(`  ✓ Created index on ${COLLECTIONS.TRAFFIC_DAILY}: timestamp_-1`);
  }
}

/**
 * Setup connection monitoring for auto-reconnect
 */
function setupConnectionMonitoring() {
  client.on('serverHeartbeatSucceeded', () => {
    // Connection is healthy
  });

  client.on('serverHeartbeatFailed', (event) => {
    console.error('MongoDB heartbeat failed:', event);
  });

  client.on('topologyClosed', () => {
    console.error('MongoDB topology closed - connection lost');
  });

  client.on('error', (error) => {
    console.error('MongoDB client error:', error);
  });
}

/**
 * Get database instance
 */
export function getDB() {
  if (!db) {
    throw new Error('MongoDB not initialized. Call initMongoDB() first.');
  }
  return db;
}

/**
 * Get collection
 */
export function getCollection(collectionName) {
  return getDB().collection(collectionName);
}

/**
 * Health check
 */
export async function healthCheck() {
  try {
    if (!db) {
      return { healthy: false, error: 'Not initialized' };
    }

    const result = await db.admin().ping();
    const stats = await db.stats();

    return {
      healthy: true,
      ping: result.ok === 1,
      database: MONGODB_DATABASE,
      collections: stats.collections,
      dataSize: stats.dataSize,
      storageSize: stats.storageSize
    };
  } catch (error) {
    return {
      healthy: false,
      error: error.message
    };
  }
}

/**
 * Close connection (for graceful shutdown)
 */
export async function closeMongoDB() {
  if (client) {
    await client.close();
    client = null;
    db = null;
    console.log('MongoDB connection closed');
  }
}

// Graceful shutdown
process.on('SIGINT', async () => {
  await closeMongoDB();
  process.exit(0);
});

process.on('SIGTERM', async () => {
  await closeMongoDB();
  process.exit(0);
});
