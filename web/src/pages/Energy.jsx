import { useState, useEffect, useRef } from 'react';
import { Zap, Thermometer, Cpu, Clock, Moon, Rocket, Play, Square, Activity } from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import {
  getCpuInfo,
  getCurrentEnergyMode,
  setEnergyMode,
  getEnergySchedule,
  saveEnergySchedule,
  getAutoSelectConfig,
  saveAutoSelectConfig,
  getSelectableInterfaces,
  getBenchmarkStatus,
  startBenchmark,
  stopBenchmark
} from '../api/client';

const MODE_ICONS = {
  economy: Moon,
  auto: Zap,
  performance: Rocket
};

const MODE_LABELS = {
  economy: 'Économie',
  auto: 'Auto',
  performance: 'Performance'
};

const MODE_DESCRIPTIONS = {
  economy: 'CPU en mode économie',
  auto: 'Adaptatif selon la charge',
  performance: 'CPU en mode performance'
};

const MODE_COLORS = {
  economy: { bg: 'bg-indigo-600', ring: 'ring-indigo-400', hover: 'hover:bg-indigo-700' },
  auto: { bg: 'bg-blue-600', ring: 'ring-blue-400', hover: 'hover:bg-blue-700' },
  performance: { bg: 'bg-orange-600', ring: 'ring-orange-400', hover: 'hover:bg-orange-700' }
};

function Energy() {
  // CPU state
  const [cpuInfo, setCpuInfo] = useState({ temperature: null, frequency: null, usage: null });

  // Mode state
  const [currentMode, setCurrentMode] = useState('auto');
  const [changingMode, setChangingMode] = useState(false);

  // Schedule state
  const [schedule, setSchedule] = useState({
    enabled: false,
    nightStart: '00:00',
    nightEnd: '08:00'
  });
  const [savingSchedule, setSavingSchedule] = useState(false);
  const [scheduleMessage, setScheduleMessage] = useState(null);

  // Auto-select state
  const [autoSelect, setAutoSelect] = useState({
    enabled: false,
    networkInterface: null,
    thresholds: { low: 1000, high: 10000 },
    averagingTime: 3,
    sampleInterval: 1000
  });
  const [savingAutoSelect, setSavingAutoSelect] = useState(false);
  const [autoSelectMessage, setAutoSelectMessage] = useState(null);
  const [networkRps, setNetworkRps] = useState({ current: 0, averaged: 0, appliedMode: null });
  const [interfaces, setInterfaces] = useState([]);
  const [interfaceError, setInterfaceError] = useState(null);

  // Benchmark state
  const [benchmark, setBenchmark] = useState({ running: false, elapsed: 0 });

  const [loading, setLoading] = useState(true);
  const pollingRef = useRef(null);

  // Initial data fetch
  useEffect(() => {
    async function fetchInitialData() {
      try {
        const [modeRes, scheduleRes, autoSelectRes, interfacesRes] = await Promise.all([
          getCurrentEnergyMode(),
          getEnergySchedule(),
          getAutoSelectConfig(),
          getSelectableInterfaces()
        ]);

        if (modeRes.data.success) {
          setCurrentMode(modeRes.data.mode);
        }

        if (scheduleRes.data.success) {
          setSchedule(scheduleRes.data.config);
        }

        if (autoSelectRes.data.success) {
          setAutoSelect(autoSelectRes.data.config);
        }

        if (interfacesRes.data.success) {
          setInterfaces(interfacesRes.data.interfaces);
        }
      } catch (error) {
        console.error('Error fetching initial data:', error);
      } finally {
        setLoading(false);
      }
    }

    fetchInitialData();
  }, []);

  // SSE connection for real-time mode and RPS updates
  useEffect(() => {
    const eventSource = new EventSource('/api/energy/events');

    eventSource.addEventListener('modeChange', (e) => {
      const data = JSON.parse(e.data);
      setCurrentMode(data.mode);
    });

    eventSource.addEventListener('rpsUpdate', (e) => {
      const data = JSON.parse(e.data);
      setNetworkRps({
        current: data.rps || 0,
        averaged: data.averagedRps || 0,
        appliedMode: data.appliedMode || null
      });
      setInterfaceError(data.interfaceError || null);
    });

    eventSource.onerror = () => {
      console.error('SSE connection error, reconnecting...');
    };

    return () => {
      eventSource.close();
    };
  }, []);

  // CPU and benchmark polling (less frequent, SSE handles mode/RPS)
  useEffect(() => {
    async function pollData() {
      try {
        const [cpuRes, benchRes] = await Promise.all([
          getCpuInfo(),
          getBenchmarkStatus()
        ]);

        if (cpuRes.data.success) {
          setCpuInfo({
            temperature: cpuRes.data.temperature,
            frequency: cpuRes.data.frequency,
            usage: cpuRes.data.usage
          });
        }

        if (benchRes.data.success) {
          setBenchmark({
            running: benchRes.data.running,
            elapsed: benchRes.data.elapsed || 0
          });
        }
      } catch (error) {
        console.error('Error polling data:', error);
      }
    }

    pollData();
    pollingRef.current = setInterval(pollData, 2000); // Slower polling, SSE handles real-time

    return () => {
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
      }
    };
  }, []);

  // Mode change handler
  async function handleModeChange(mode) {
    if (mode === currentMode) return;

    setChangingMode(true);
    try {
      const res = await setEnergyMode(mode);
      if (res.data.success) {
        setCurrentMode(mode);
      }
    } catch (error) {
      console.error('Error changing mode:', error);
    } finally {
      setChangingMode(false);
    }
  }

  // Benchmark handlers
  async function handleStartBenchmark() {
    try {
      const res = await startBenchmark(60);
      if (res.data.success) {
        setBenchmark({ running: true, elapsed: 0 });
      }
    } catch (error) {
      console.error('Error starting benchmark:', error);
    }
  }

  async function handleStopBenchmark() {
    try {
      await stopBenchmark();
      setBenchmark({ running: false, elapsed: 0 });
    } catch (error) {
      console.error('Error stopping benchmark:', error);
    }
  }

  // Schedule save handler
  async function handleSaveSchedule() {
    setSavingSchedule(true);
    setScheduleMessage(null);
    try {
      const res = await saveEnergySchedule(schedule);
      if (res.data.success) {
        setScheduleMessage({ type: 'success', text: 'Programmation enregistrée' });
      } else {
        setScheduleMessage({ type: 'error', text: res.data.error || 'Erreur' });
      }
    } catch (error) {
      setScheduleMessage({ type: 'error', text: error.message });
    } finally {
      setSavingSchedule(false);
      setTimeout(() => setScheduleMessage(null), 3000);
    }
  }

  // Auto-select save handler
  async function handleSaveAutoSelect() {
    setSavingAutoSelect(true);
    setAutoSelectMessage(null);
    try {
      const res = await saveAutoSelectConfig(autoSelect);
      if (res.data.success) {
        setAutoSelectMessage({ type: 'success', text: 'Configuration enregistrée' });
      } else {
        setAutoSelectMessage({ type: 'error', text: res.data.error || 'Erreur' });
      }
    } catch (error) {
      setAutoSelectMessage({ type: 'error', text: error.message });
    } finally {
      setSavingAutoSelect(false);
      setTimeout(() => setAutoSelectMessage(null), 3000);
    }
  }

  // Temperature color helper
  function getTempColor(temp) {
    if (temp < 50) return 'text-green-400';
    if (temp < 70) return 'text-yellow-400';
    if (temp < 85) return 'text-orange-400';
    return 'text-red-400';
  }

  function getTempBarColor(temp) {
    if (temp < 50) return 'bg-green-500';
    if (temp < 70) return 'bg-yellow-500';
    if (temp < 85) return 'bg-orange-500';
    return 'bg-red-500';
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <h1 className="text-2xl font-bold flex items-center gap-2">
        <Zap className="text-yellow-400" />
        Énergie
      </h1>

      {/* CPU Info + Mode side by side */}
      <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
        {/* CPU Info Card */}
        <Card title="Infos CPU (Ryzen 9 3900X)" icon={Cpu}>
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
          {/* Temperature */}
          <div className="bg-gray-900 rounded-lg p-4">
            <div className="flex items-center gap-2 text-gray-400 mb-2">
              <Thermometer size={16} />
              <span className="text-sm">Température</span>
            </div>
            <div className={`text-3xl font-bold ${getTempColor(cpuInfo.temperature || 0)}`}>
              {cpuInfo.temperature !== null ? `${cpuInfo.temperature.toFixed(0)}°C` : '--'}
            </div>
            <div className="mt-2 h-2 bg-gray-700 rounded-full overflow-hidden">
              <div
                className={`h-full ${getTempBarColor(cpuInfo.temperature || 0)} transition-all`}
                style={{ width: `${Math.min(100, ((cpuInfo.temperature || 0) / 95) * 100)}%` }}
              />
            </div>
            <div className="text-xs text-gray-500 mt-1">max 95°C</div>
          </div>

          {/* Frequency */}
          <div className="bg-gray-900 rounded-lg p-4">
            <div className="flex items-center gap-2 text-gray-400 mb-2">
              <Zap size={16} />
              <span className="text-sm">Fréquence</span>
            </div>
            <div className="text-3xl font-bold text-blue-400">
              {cpuInfo.frequency?.current != null
                ? `${cpuInfo.frequency.current.toFixed(1)} GHz`
                : '--'}
            </div>
            <div className="text-sm text-gray-500 mt-2">
              {cpuInfo.frequency?.min && cpuInfo.frequency?.max
                ? `${cpuInfo.frequency.min.toFixed(1)} - ${cpuInfo.frequency.max.toFixed(1)} GHz`
                : '--'}
            </div>
          </div>

          {/* Usage */}
          <div className="bg-gray-900 rounded-lg p-4">
            <div className="flex items-center gap-2 text-gray-400 mb-2">
              <Cpu size={16} />
              <span className="text-sm">Usage CPU</span>
            </div>
            <div className="text-3xl font-bold text-purple-400">
              {cpuInfo.usage !== null ? `${cpuInfo.usage.toFixed(0)}%` : '--'}
            </div>
            <div className="mt-2 h-2 bg-gray-700 rounded-full overflow-hidden">
              <div
                className="h-full bg-purple-500 transition-all"
                style={{ width: `${cpuInfo.usage || 0}%` }}
              />
            </div>
          </div>
        </div>

        {/* Benchmark button */}
        <div className="mt-4 pt-4 border-t border-gray-700 flex items-center justify-between">
          <div className="text-sm text-gray-400">
            {benchmark.running
              ? `Benchmark en cours... ${benchmark.elapsed}s / 60s`
              : 'Stress test CPU (1 min max)'}
          </div>
          {benchmark.running ? (
            <button
              onClick={handleStopBenchmark}
              className="flex items-center gap-2 px-4 py-2 bg-red-600 hover:bg-red-700 text-white rounded-lg font-medium transition-colors"
            >
              <Square size={16} />
              Arrêter
            </button>
          ) : (
            <button
              onClick={handleStartBenchmark}
              className="flex items-center gap-2 px-4 py-2 bg-green-600 hover:bg-green-700 text-white rounded-lg font-medium transition-colors"
            >
              <Play size={16} />
              Benchmark
            </button>
          )}
        </div>
      </Card>

        {/* Mode Card */}
        <Card title="Mode" icon={Zap}>
        <div className="flex flex-col md:flex-row gap-6">
          {/* Mode buttons */}
          <div className="flex flex-wrap gap-4 justify-center md:justify-start">
            {['economy', 'auto', 'performance'].map(mode => {
              const Icon = MODE_ICONS[mode];
              const isActive = currentMode === mode;
              const colors = MODE_COLORS[mode];
              const isDisabled = changingMode || autoSelect.enabled;

              return (
                <button
                  key={mode}
                  onClick={() => handleModeChange(mode)}
                  disabled={isDisabled}
                  className={`flex flex-col items-center justify-center w-28 h-28 rounded-xl transition-all ${
                    isActive
                      ? `${colors.bg} text-white ring-2 ${colors.ring} shadow-lg`
                      : `bg-gray-800 text-gray-300 ${isDisabled ? '' : colors.hover}`
                  } ${isDisabled ? 'opacity-50 cursor-not-allowed' : ''}`}
                >
                  <Icon size={32} className={isActive ? 'text-white' : 'text-gray-400'} />
                  <span className="mt-2 font-medium">{MODE_LABELS[mode]}</span>
                  {isActive && (
                    <span className="text-xs opacity-75 mt-1">
                      {autoSelect.enabled ? 'auto' : 'actif'}
                    </span>
                  )}
                </button>
              );
            })}
          </div>

          {/* Mode details */}
          <div className="flex-1 bg-gray-900 rounded-lg p-4 flex items-center">
            <p className="text-gray-300 font-medium">{MODE_DESCRIPTIONS[currentMode]}</p>
          </div>
        </div>
      </Card>
      </div>

      {/* Programmation Card */}
      <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
        {/* Programmation Card */}
        <Card title="Programmation" icon={Clock}>
          <div className="space-y-6">
          {/* Schedule */}
          <div className="space-y-4">
            <div className="flex items-center gap-3">
              <button
                onClick={async () => {
                  const newSchedule = { ...schedule, enabled: !schedule.enabled };
                  setSchedule(newSchedule);
                  // Auto-save when toggling
                  try {
                    await saveEnergySchedule(newSchedule);
                  } catch (error) {
                    console.error('Error saving schedule:', error);
                  }
                }}
                className={`relative w-12 h-6 rounded-full transition-colors ${
                  schedule.enabled ? 'bg-blue-600' : 'bg-gray-600'
                }`}
              >
                <span
                  className={`absolute top-0.5 left-0.5 w-5 h-5 bg-white rounded-full transition-transform ${
                    schedule.enabled ? 'translate-x-6' : ''
                  }`}
                />
              </button>
              <span className="text-gray-300 font-medium">Programmation horaire</span>
            </div>

            {schedule.enabled && (
              <div className="bg-gray-900 rounded-lg p-4 space-y-3">
                <div className="flex flex-wrap items-center gap-3">
                  <span className="text-gray-400">Forcer économie de</span>
                  <input
                    type="time"
                    value={schedule.nightStart}
                    onChange={e => setSchedule(prev => ({ ...prev, nightStart: e.target.value }))}
                    className="bg-gray-800 border border-gray-700 rounded-lg px-3 py-2 text-white"
                  />
                  <span className="text-gray-400">à</span>
                  <input
                    type="time"
                    value={schedule.nightEnd}
                    onChange={e => setSchedule(prev => ({ ...prev, nightEnd: e.target.value }))}
                    className="bg-gray-800 border border-gray-700 rounded-lg px-3 py-2 text-white"
                  />
                </div>

                <div className="flex items-center gap-3">
                  <Button onClick={handleSaveSchedule} loading={savingSchedule}>
                    Enregistrer
                  </Button>
                  {scheduleMessage && (
                    <span className={scheduleMessage.type === 'success' ? 'text-green-400' : 'text-red-400'}>
                      {scheduleMessage.text}
                    </span>
                  )}
                </div>
              </div>
            )}
          </div>

          {/* Auto-select */}
          <div className="space-y-4">
            <div className="flex items-center gap-3">
              <button
                onClick={async () => {
                  // Prevent enabling without interface
                  if (!autoSelect.enabled && !autoSelect.networkInterface) {
                    setAutoSelectMessage({ type: 'error', text: 'Sélectionnez d\'abord une interface' });
                    setTimeout(() => setAutoSelectMessage(null), 3000);
                    return;
                  }
                  const newAutoSelect = { ...autoSelect, enabled: !autoSelect.enabled };
                  setAutoSelect(newAutoSelect);
                  // Auto-save when toggling
                  try {
                    const res = await saveAutoSelectConfig(newAutoSelect);
                    if (!res.data.success) {
                      setAutoSelect(autoSelect); // Revert
                      setAutoSelectMessage({ type: 'error', text: res.data.error });
                      setTimeout(() => setAutoSelectMessage(null), 3000);
                    }
                  } catch (error) {
                    setAutoSelect(autoSelect); // Revert
                    console.error('Error saving auto-select config:', error);
                  }
                }}
                className={`relative w-12 h-6 rounded-full transition-colors ${
                  autoSelect.enabled ? 'bg-green-600' : 'bg-gray-600'
                }`}
              >
                <span
                  className={`absolute top-0.5 left-0.5 w-5 h-5 bg-white rounded-full transition-transform ${
                    autoSelect.enabled ? 'translate-x-6' : ''
                  }`}
                />
              </button>
              <span className="text-gray-300 font-medium">Sélection automatique</span>
              {autoSelectMessage && !autoSelect.enabled && (
                <span className={autoSelectMessage.type === 'success' ? 'text-green-400 text-sm' : 'text-red-400 text-sm'}>
                  {autoSelectMessage.text}
                </span>
              )}
            </div>

            {/* Interface selector and RPS indicator */}
            <div className="bg-gray-900 rounded-lg p-3 space-y-3">
              <div>
                <div className="flex items-center gap-2 text-sm text-gray-400 mb-2">
                  <Activity size={14} />
                  <span>Interface réseau</span>
                </div>
                {interfaces.length === 0 ? (
                  <p className="text-gray-500 text-sm">Aucune interface réseau détectée</p>
                ) : (
                  <select
                    value={autoSelect.networkInterface || ''}
                    onChange={e => setAutoSelect(prev => ({
                      ...prev,
                      networkInterface: e.target.value || null
                    }))}
                    className="w-full bg-gray-800 border border-gray-700 rounded-lg px-3 py-2 text-white text-sm"
                  >
                    <option value="">Sélectionner une interface...</option>
                    {interfaces.map(iface => (
                      <option key={iface.name} value={iface.name}>
                        {iface.name} ({iface.primaryIp}){iface.state !== 'UP' ? ' - DOWN' : ''}
                      </option>
                    ))}
                  </select>
                )}
                {interfaceError === 'not_configured' && (
                  <p className="text-yellow-400 text-xs mt-1">
                    Sélectionnez une interface pour activer l'auto-select
                  </p>
                )}
                {interfaceError === 'not_found' && (
                  <p className="text-red-400 text-xs mt-1">
                    L'interface configurée n'existe plus
                  </p>
                )}
              </div>

              {autoSelect.networkInterface && !interfaceError && (
                <>
                  <div className="flex items-center justify-between">
                    <span className="text-gray-400 text-sm">Charge (moy. {autoSelect.averagingTime}s):</span>
                    <span className="text-white font-mono font-bold text-lg">{networkRps.averaged.toLocaleString()} req/s</span>
                  </div>
                  {autoSelect.enabled && currentMode && (
                    <div className="flex items-center justify-between">
                      <span className="text-gray-400 text-sm">Mode appliqué:</span>
                      <span className={`font-medium ${
                        currentMode === 'economy' ? 'text-indigo-400' :
                        currentMode === 'performance' ? 'text-orange-400' : 'text-blue-400'
                      }`}>
                        {MODE_LABELS[currentMode]}
                      </span>
                    </div>
                  )}
                </>
              )}
            </div>

            {autoSelect.enabled && (
              <div className="bg-gray-900 rounded-lg p-4 space-y-4">
                <div className="grid grid-cols-3 gap-3">
                  <div>
                    <label className="block text-xs text-gray-400 mb-1">Seuil bas (req/s)</label>
                    <input
                      type="number"
                      value={autoSelect.thresholds.low}
                      onChange={e => setAutoSelect(prev => ({
                        ...prev,
                        thresholds: { ...prev.thresholds, low: parseInt(e.target.value) || 0 }
                      }))}
                      className="w-full bg-gray-800 border border-gray-700 rounded-lg px-3 py-2 text-white text-sm"
                    />
                    <span className="text-xs text-indigo-400">→ Économie</span>
                  </div>
                  <div>
                    <label className="block text-xs text-gray-400 mb-1">Seuil haut (req/s)</label>
                    <input
                      type="number"
                      value={autoSelect.thresholds.high}
                      onChange={e => setAutoSelect(prev => ({
                        ...prev,
                        thresholds: { ...prev.thresholds, high: parseInt(e.target.value) || 0 }
                      }))}
                      className="w-full bg-gray-800 border border-gray-700 rounded-lg px-3 py-2 text-white text-sm"
                    />
                    <span className="text-xs text-orange-400">→ Performance</span>
                  </div>
                  <div>
                    <label className="block text-xs text-gray-400 mb-1">Temps moyenne (s)</label>
                    <input
                      type="number"
                      min="1"
                      max="30"
                      value={autoSelect.averagingTime}
                      onChange={e => setAutoSelect(prev => ({
                        ...prev,
                        averagingTime: Math.max(1, Math.min(30, parseInt(e.target.value) || 3))
                      }))}
                      className="w-full bg-gray-800 border border-gray-700 rounded-lg px-3 py-2 text-white text-sm"
                    />
                  </div>
                </div>

                <div className="flex items-center gap-3">
                  <Button onClick={handleSaveAutoSelect} loading={savingAutoSelect}>
                    Enregistrer
                  </Button>
                  {autoSelectMessage && (
                    <span className={autoSelectMessage.type === 'success' ? 'text-green-400' : 'text-red-400'}>
                      {autoSelectMessage.text}
                    </span>
                  )}
                </div>
              </div>
            )}
          </div>
        </div>
      </Card>
      </div>
    </div>
  );
}

export default Energy;
