import { useState, useEffect, useRef } from 'react';
import { RefreshCw, Package, AlertTriangle, Server, Shield, CheckCircle, Play, Square, ChevronDown, ChevronUp, Download } from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import StatusBadge from '../components/StatusBadge';
import ConfirmModal from '../components/ConfirmModal';
import PageHeader from '../components/PageHeader';
import {
  getUpdatesStatus,
  getLastUpdatesCheck,
  checkForUpdates,
  cancelUpdatesCheck,
  getUpgradeStatus,
  runAptUpgrade,
  runAptFullUpgrade,
  runSnapRefresh,
  cancelUpgrade
} from '../api/client';
import useWebSocket from '../hooks/useWebSocket';

function Updates() {
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [message, setMessage] = useState(null);
  const [currentPhase, setCurrentPhase] = useState(null);
  const [liveOutput, setLiveOutput] = useState([]);
  const [showOutput, setShowOutput] = useState(false);
  const outputRef = useRef(null);
  const upgradeOutputRef = useRef(null);

  // Results state
  const [lastCheck, setLastCheck] = useState(null);
  const [aptPackages, setAptPackages] = useState([]);
  const [snapPackages, setSnapPackages] = useState([]);
  const [needrestart, setNeedrestart] = useState(null);
  const [summary, setSummary] = useState(null);

  // Upgrade state
  const [upgrading, setUpgrading] = useState(false);
  const [upgradeType, setUpgradeType] = useState(null);
  const [upgradeOutput, setUpgradeOutput] = useState([]);
  const [showUpgradeOutput, setShowUpgradeOutput] = useState(true);

  // Confirm modal
  const [confirmModal, setConfirmModal] = useState({ show: false, type: null });

  useWebSocket({
    'updates:started': () => {
      setRunning(true);
      setLiveOutput([]);
      setCurrentPhase(null);
      setMessage(null);
    },
    'updates:phase': (data) => {
      setCurrentPhase(data);
    },
    'updates:output': (data) => {
      setLiveOutput(prev => [...prev.slice(-100), data.line]);
    },
    'updates:apt-complete': (data) => {
      setAptPackages(data.packages || []);
    },
    'updates:snap-complete': (data) => {
      setSnapPackages(data.snaps || []);
    },
    'updates:needrestart-complete': (data) => {
      setNeedrestart(data);
    },
    'updates:complete': (data) => {
      setRunning(false);
      setCurrentPhase(null);
      setSummary(data.summary);
      setLastCheck(new Date().toISOString());
      if (data.success) {
        setMessage({ type: 'success', text: `Verification terminee en ${Math.round(data.duration / 1000)}s` });
      }
    },
    'updates:cancelled': () => {
      setRunning(false);
      setCurrentPhase(null);
      setCancelling(false);
      setMessage({ type: 'warning', text: 'Verification annulee' });
    },
    'updates:error': (data) => {
      setRunning(false);
      setCurrentPhase(null);
      setMessage({ type: 'error', text: data.error });
    },
    'updates:upgrade-started': (data) => {
      setUpgrading(true);
      setUpgradeType(data.type);
      setUpgradeOutput([]);
      setShowUpgradeOutput(true);
      setMessage(null);
    },
    'updates:upgrade-output': (data) => {
      setUpgradeOutput(prev => [...prev.slice(-200), data.line]);
    },
    'updates:upgrade-complete': (data) => {
      setUpgrading(false);
      if (data.success) {
        setMessage({ type: 'success', text: `Mise a jour ${data.type} terminee en ${Math.round(data.duration / 1000)}s` });
        fetchInitialData();
      } else {
        setMessage({ type: 'error', text: data.error || 'Erreur lors de la mise a jour' });
      }
    },
    'updates:upgrade-cancelled': () => {
      setUpgrading(false);
      setCancelling(false);
      setMessage({ type: 'warning', text: 'Mise a jour annulee' });
    },
  });

  // Auto-scroll live output
  useEffect(() => {
    if (outputRef.current) {
      outputRef.current.scrollTop = outputRef.current.scrollHeight;
    }
  }, [liveOutput]);

  // Auto-scroll upgrade output
  useEffect(() => {
    if (upgradeOutputRef.current) {
      upgradeOutputRef.current.scrollTop = upgradeOutputRef.current.scrollHeight;
    }
  }, [upgradeOutput]);

  // Initial data fetch
  useEffect(() => {
    fetchInitialData();
  }, []);

  async function fetchInitialData() {
    try {
      // Check if a check is already running
      const statusRes = await getUpdatesStatus();
      if (statusRes.data.running) {
        setRunning(true);
      }

      // Check if upgrade is running
      const upgradeRes = await getUpgradeStatus();
      if (upgradeRes.data.running) {
        setUpgrading(true);
      }

      // Get last check results
      const lastRes = await getLastUpdatesCheck();
      if (lastRes.data.success && lastRes.data.result) {
        const result = lastRes.data.result;
        setLastCheck(result.timestamp);
        setAptPackages(result.apt?.packages || []);
        setSnapPackages(result.snap?.packages || []);
        setNeedrestart(result.needrestart || null);
        setSummary(result.summary || null);
      }
    } catch (error) {
      console.error('Error fetching initial data:', error);
    } finally {
      setLoading(false);
    }
  }

  async function handleCheck() {
    setMessage(null);
    setLiveOutput([]);
    try {
      await checkForUpdates();
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || error.message });
      setRunning(false);
    }
  }

  async function handleCancel() {
    setCancelling(true);
    try {
      if (upgrading) {
        await cancelUpgrade();
      } else {
        await cancelUpdatesCheck();
      }
    } catch (error) {
      console.error('Error cancelling:', error);
      setCancelling(false);
    }
  }

  async function handleUpgrade(type) {
    setConfirmModal({ show: false, type: null });
    setMessage(null);
    setUpgradeOutput([]);

    try {
      switch (type) {
        case 'apt':
          await runAptUpgrade();
          break;
        case 'apt-full':
          await runAptFullUpgrade();
          break;
        case 'snap':
          await runSnapRefresh();
          break;
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.response?.data?.error || error.message });
      setUpgrading(false);
    }
  }

  function formatDate(isoString) {
    if (!isoString) return '-';
    return new Date(isoString).toLocaleString('fr-FR');
  }

  function getUpgradeDescription(type) {
    switch (type) {
      case 'apt':
        return 'Cette action va executer "apt upgrade" pour mettre a jour tous les paquets APT disponibles. Les paquets existants ne seront pas supprimes.';
      case 'apt-full':
        return 'Cette action va executer "apt full-upgrade". Contrairement a upgrade, cette commande peut supprimer des paquets si necessaire pour resoudre des conflits.';
      case 'snap':
        return 'Cette action va executer "snap refresh" pour mettre a jour tous les snaps installes.';
      default:
        return '';
    }
  }

  function getUpgradeTitle(type) {
    switch (type) {
      case 'apt':
        return 'APT Upgrade';
      case 'apt-full':
        return 'APT Full-Upgrade';
      case 'snap':
        return 'Snap Refresh';
      default:
        return 'Mise a jour';
    }
  }

  if (loading) {
    return (
      <div className="flex items-center justify-center h-full">
        <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-blue-400"></div>
      </div>
    );
  }

  const securityCount = aptPackages.filter(p => p.isSecurity).length;
  const isRunningAny = running || upgrading;

  return (
    <div>
      <PageHeader title="Mises a jour systeme" icon={Package}>
        {isRunningAny ? (
          <Button
            variant="danger"
            onClick={handleCancel}
            loading={cancelling}
            icon={Square}
          >
            Annuler
          </Button>
        ) : (
          <Button
            variant="primary"
            onClick={handleCheck}
            icon={Play}
          >
            Verifier les mises a jour
          </Button>
        )}
      </PageHeader>

      {message && (
        <div className={`p-3 ${
          message.type === 'success' ? 'bg-green-500/20 text-green-400' :
          message.type === 'error' ? 'bg-red-500/20 text-red-400' :
          'bg-yellow-500/20 text-yellow-400'
        }`}>
          {message.text}
        </div>
      )}

      {/* Check Progress Section */}
      {running && (
        <Card title="Verification en cours" icon={RefreshCw}>
          <div className="space-y-4">
            {currentPhase && (
              <div className="flex items-center gap-2">
                <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-blue-400"></div>
                <span className="text-blue-400">{currentPhase.message}</span>
              </div>
            )}

            <div className="flex items-center gap-2">
              <button
                onClick={() => setShowOutput(!showOutput)}
                className="flex items-center gap-1 text-sm text-gray-400 hover:text-white"
              >
                {showOutput ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
                {showOutput ? 'Masquer' : 'Afficher'} la sortie
              </button>
            </div>

            {showOutput && liveOutput.length > 0 && (
              <div
                ref={outputRef}
                className="bg-gray-900 p-3 font-mono text-xs h-48 overflow-y-auto"
              >
                {liveOutput.map((line, i) => (
                  <div key={i} className="text-gray-400">{line}</div>
                ))}
              </div>
            )}
          </div>
        </Card>
      )}

      {/* Upgrade Progress Section */}
      {upgrading && (
        <Card title={`Mise a jour en cours: ${getUpgradeTitle(upgradeType)}`} icon={Download}>
          <div className="space-y-4">
            <div className="flex items-center gap-2">
              <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-green-400"></div>
              <span className="text-green-400">Installation en cours...</span>
            </div>

            <div className="flex items-center gap-2">
              <button
                onClick={() => setShowUpgradeOutput(!showUpgradeOutput)}
                className="flex items-center gap-1 text-sm text-gray-400 hover:text-white"
              >
                {showUpgradeOutput ? <ChevronUp className="w-4 h-4" /> : <ChevronDown className="w-4 h-4" />}
                {showUpgradeOutput ? 'Masquer' : 'Afficher'} la sortie
              </button>
            </div>

            {showUpgradeOutput && upgradeOutput.length > 0 && (
              <div
                ref={upgradeOutputRef}
                className="bg-gray-900 p-3 font-mono text-xs h-64 overflow-y-auto"
              >
                {upgradeOutput.map((line, i) => (
                  <div key={i} className="text-gray-400">{line}</div>
                ))}
              </div>
            )}
          </div>
        </Card>
      )}

      {/* Summary Cards */}
      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
        {/* Total Updates */}
        <Card title="Total mises a jour" icon={Package}>
          <div className="text-3xl font-bold text-blue-400">
            {aptPackages.length + snapPackages.length}
          </div>
          <p className="text-sm text-gray-400">paquets disponibles</p>
          {lastCheck && (
            <p className="text-xs text-gray-500 mt-2">
              Derniere verification: {formatDate(lastCheck)}
            </p>
          )}
        </Card>

        {/* Security Updates */}
        <Card title="Securite" icon={Shield}>
          <div className={`text-3xl font-bold ${securityCount > 0 ? 'text-red-400' : 'text-green-400'}`}>
            {securityCount}
          </div>
          <p className="text-sm text-gray-400">
            {securityCount > 0 ? 'mises a jour critiques' : 'systeme a jour'}
          </p>
        </Card>

        {/* Services Restart */}
        <Card title="Services" icon={Server}>
          {needrestart?.kernelRebootNeeded ? (
            <>
              <div className="text-3xl font-bold text-red-400">
                <AlertTriangle className="w-8 h-8" />
              </div>
              <p className="text-sm text-red-400">Redemarrage requis</p>
            </>
          ) : needrestart?.services?.length > 0 ? (
            <>
              <div className="text-3xl font-bold text-yellow-400">
                {needrestart.services.length}
              </div>
              <p className="text-sm text-gray-400">services a redemarrer</p>
            </>
          ) : (
            <>
              <div className="text-3xl font-bold text-green-400">
                <CheckCircle className="w-8 h-8" />
              </div>
              <p className="text-sm text-gray-400">Aucun redemarrage requis</p>
            </>
          )}
        </Card>
      </div>

      {/* APT Packages */}
      {aptPackages.length > 0 && (
        <Card
          title={`Paquets APT (${aptPackages.length})`}
          icon={Package}
          actions={
            <div className="flex gap-2">
              <Button
                variant="primary"
                size="sm"
                onClick={() => setConfirmModal({ show: true, type: 'apt' })}
                disabled={isRunningAny}
                icon={Download}
              >
                Upgrade
              </Button>
              <Button
                variant="secondary"
                size="sm"
                onClick={() => setConfirmModal({ show: true, type: 'apt-full' })}
                disabled={isRunningAny}
              >
                Full-Upgrade
              </Button>
            </div>
          }
        >
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-400 border-b border-gray-700">
                  <th className="pb-2">Paquet</th>
                  <th className="pb-2">Version actuelle</th>
                  <th className="pb-2">Nouvelle version</th>
                  <th className="pb-2">Type</th>
                </tr>
              </thead>
              <tbody>
                {aptPackages.map((pkg, i) => (
                  <tr key={i} className="border-b border-gray-700/50">
                    <td className="py-2 font-mono">{pkg.name}</td>
                    <td className="py-2 font-mono text-gray-400 text-xs">{pkg.currentVersion}</td>
                    <td className="py-2 font-mono text-blue-400 text-xs">{pkg.newVersion}</td>
                    <td className="py-2">
                      {pkg.isSecurity ? (
                        <StatusBadge status="down">Securite</StatusBadge>
                      ) : (
                        <StatusBadge status="active">Normal</StatusBadge>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}

      {/* Snap Packages */}
      {snapPackages.length > 0 && (
        <Card
          title={`Snaps (${snapPackages.length})`}
          icon={Package}
          actions={
            <Button
              variant="primary"
              size="sm"
              onClick={() => setConfirmModal({ show: true, type: 'snap' })}
              disabled={isRunningAny}
              icon={Download}
            >
              Refresh
            </Button>
          }
        >
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead>
                <tr className="text-left text-gray-400 border-b border-gray-700">
                  <th className="pb-2">Snap</th>
                  <th className="pb-2">Nouvelle version</th>
                  <th className="pb-2">Revision</th>
                  <th className="pb-2">Editeur</th>
                </tr>
              </thead>
              <tbody>
                {snapPackages.map((snap, i) => (
                  <tr key={i} className="border-b border-gray-700/50">
                    <td className="py-2 font-mono">{snap.name}</td>
                    <td className="py-2 font-mono text-blue-400">{snap.newVersion}</td>
                    <td className="py-2 text-gray-400">{snap.revision}</td>
                    <td className="py-2 text-gray-400">{snap.publisher}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}

      {/* Services needing restart */}
      {needrestart && (needrestart.kernelRebootNeeded || needrestart.services?.length > 0) && (
        <Card title="Services a redemarrer" icon={AlertTriangle}>
          {needrestart.kernelRebootNeeded && (
            <div className="bg-red-500/20 p-4 mb-4">
              <div className="flex items-center gap-2 text-red-400 font-semibold">
                <AlertTriangle className="w-5 h-5" />
                Redemarrage du systeme requis
              </div>
              {needrestart.currentKernel && needrestart.expectedKernel && (
                <div className="mt-2 text-sm text-gray-400">
                  <p>Kernel actuel: <span className="font-mono">{needrestart.currentKernel}</span></p>
                  <p>Kernel attendu: <span className="font-mono text-blue-400">{needrestart.expectedKernel}</span></p>
                </div>
              )}
            </div>
          )}

          {needrestart.services?.length > 0 && (
            <div className="space-y-2">
              <p className="text-sm text-gray-400 mb-3">
                Les services suivants doivent etre redemarres pour appliquer les mises a jour:
              </p>
              {needrestart.services.map((service, i) => (
                <div key={i} className="flex items-center gap-2 p-2 bg-gray-800">
                  <Server className="w-4 h-4 text-yellow-400" />
                  <span className="font-mono text-sm">{service}</span>
                </div>
              ))}
            </div>
          )}
        </Card>
      )}

      {/* Empty state */}
      {!running && !upgrading && aptPackages.length === 0 && snapPackages.length === 0 && !needrestart && (
        <Card>
          <div className="text-center py-8 text-gray-400">
            <RefreshCw className="w-12 h-12 mx-auto mb-4 opacity-50" />
            <p>Aucune donnee disponible</p>
            <p className="text-sm mt-2">Cliquez sur "Verifier les mises a jour" pour commencer</p>
          </div>
        </Card>
      )}

      {/* Confirmation Modal */}
      <ConfirmModal
        isOpen={confirmModal.show}
        onClose={() => setConfirmModal({ show: false, type: null })}
        onConfirm={() => handleUpgrade(confirmModal.type)}
        title={`Confirmer ${getUpgradeTitle(confirmModal.type)}`}
        message={getUpgradeDescription(confirmModal.type)}
        confirmText="Lancer la mise a jour"
        variant="warning"
      />
    </div>
  );
}

export default Updates;
