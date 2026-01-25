import React from 'react';
import {
  LineChart, Line, XAxis, YAxis, CartesianGrid,
  Tooltip, Legend, ResponsiveContainer
} from 'recharts';

export default function TrafficTimeseriesChart({ data, metric }) {
  return (
    <ResponsiveContainer width="100%" height={300}>
      <LineChart data={data}>
        <CartesianGrid strokeDasharray="3 3" stroke="#374151" />
        <XAxis
          dataKey="timestamp"
          stroke="#9CA3AF"
          tickFormatter={(ts) => new Date(ts).toLocaleTimeString('fr-FR', {
            hour: '2-digit',
            minute: '2-digit'
          })}
        />
        <YAxis stroke="#9CA3AF" />
        <Tooltip
          contentStyle={{
            backgroundColor: '#1F2937',
            border: '1px solid #374151',
            borderRadius: '0.5rem'
          }}
          labelStyle={{ color: '#F3F4F6' }}
          labelFormatter={(ts) => new Date(ts).toLocaleString('fr-FR')}
        />
        <Legend />
        <Line
          type="monotone"
          dataKey="value"
          stroke="#3B82F6"
          strokeWidth={2}
          dot={false}
          name={metric === 'requests' ? 'Requêtes' : 'Bande passante (MB)'}
        />
      </LineChart>
    </ResponsiveContainer>
  );
}
