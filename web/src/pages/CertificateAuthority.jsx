import { useState, useEffect } from 'react';
import {
  Shield,
  Download,
  Key,
  AlertTriangle,
  CheckCircle,
  RefreshCw,
  Trash2,
  Calendar,
  Globe,
  Info,
  ChevronDown,
  ChevronUp,
  Copy,
  Check
} from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import PageHeader from '../components/PageHeader';
import Section from '../components/Section';

const CertificateAuthority = () => {
  const [status, setStatus] = useState(null);
  const [certificates, setCertificates] = useState([]);
  const [loading, setLoading] = useState(true);
  const [message, setMessage] = useState(null);
  const [initializing, setInitializing] = useState(false);
  const [showInstructions, setShowInstructions] = useState(false);
  const [copiedFormat, setCopiedFormat] = useState(null);

  useEffect(() => {
    fetchData();
  }, []);

  async function fetchData() {
    try {
      const [statusRes, certsRes] = await Promise.all([
        fetch('/api/ca/status'),
        fetch('/api/ca/certificates'),
      ]);

      const statusData = await statusRes.json();
      const certsData = await certsRes.json();

      setStatus(statusData);
      if (certsData.success) {
        setCertificates(certsData.certificates || []);
      }
    } catch (error) {
      console.error('Error fetching CA data:', error);
      setMessage({ type: 'error', text: 'Erreur de chargement' });
    } finally {
      setLoading(false);
    }
  }

  async function handleInitialize() {
    setInitializing(true);
    setMessage(null);

    try {
      const response = await fetch('/api/ca/init', {
        method: 'POST',
      });

      const data = await response.json();

      if (data.success) {
        setMessage({ type: 'success', text: 'CA initialisée avec succès' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: data.error || 'Erreur lors de l\'initialisation' });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.message });
    } finally {
      setInitializing(false);
    }
  }

  async function handleDownloadRootCert(format = 'pem') {
    try {
      const response = await fetch(`/api/ca/root-cert?format=${format}`);

      if (!response.ok) {
        throw new Error('Erreur lors du téléchargement');
      }

      const blob = await response.blob();
      const url = window.URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `homeroute-root-ca.${format}`;
      document.body.appendChild(a);
      a.click();
      window.URL.revokeObjectURL(url);
      document.body.removeChild(a);

      setCopiedFormat(format);
      setTimeout(() => setCopiedFormat(null), 2000);
    } catch (error) {
      setMessage({ type: 'error', text: error.message });
    }
  }

  async function handleRenewCertificate(certId) {
    setMessage(null);

    try {
      const response = await fetch(`/api/ca/renew/${certId}`, {
        method: 'POST',
      });

      const data = await response.json();

      if (data.success) {
        setMessage({ type: 'success', text: 'Certificat renouvelé avec succès' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.message });
    }
  }

  async function handleRevokeCertificate(certId) {
    if (!confirm('Êtes-vous sûr de vouloir révoquer ce certificat ?')) {
      return;
    }

    setMessage(null);

    try {
      const response = await fetch(`/api/ca/revoke/${certId}`, {
        method: 'DELETE',
      });

      const data = await response.json();

      if (data.success) {
        setMessage({ type: 'success', text: 'Certificat révoqué avec succès' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: data.error });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.message });
    }
  }

  function formatDate(dateString) {
    return new Date(dateString).toLocaleDateString('fr-FR', {
      year: 'numeric',
      month: 'long',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  }

  function getDaysUntilExpiry(expiresAt) {
    const now = new Date();
    const expiry = new Date(expiresAt);
    const diffTime = expiry - now;
    const diffDays = Math.ceil(diffTime / (1000 * 60 * 60 * 24));
    return diffDays;
  }

  if (loading) {
    return (
      <div className="p-6">
        <div className="text-center">Chargement...</div>
      </div>
    );
  }

  return (
    <div>
      <PageHeader title="Autorité de Certification" icon={Shield} />

      {message && (
        <div
          className={`p-3 ${
            message.type === 'error'
              ? 'bg-red-500/20 text-red-400'
              : 'bg-green-500/20 text-green-400'
          }`}
        >
          {message.text}
        </div>
      )}

      <Section title="Statut de la CA">
      <Card title="Statut" icon={Key}>
          {!status?.initialized ? (
            <div className="space-y-4">
              <div className="flex items-center gap-3 text-orange-400">
                <AlertTriangle className="w-5 h-5" />
                <span>L'autorité de certification n'est pas initialisée</span>
              </div>
              <Button
                onClick={handleInitialize}
                disabled={initializing}
                className="bg-blue-600 hover:bg-blue-700 text-white"
              >
                {initializing ? 'Initialisation...' : 'Initialiser la CA'}
              </Button>
            </div>
          ) : (
            <div className="space-y-4">
              <div className="flex items-center gap-3 text-green-400">
                <CheckCircle className="w-5 h-5" />
                <span className="font-medium">CA initialisée et opérationnelle</span>
              </div>

              <div className="border-t border-gray-700 pt-4">
                <h3 className="font-medium mb-3 flex items-center gap-2">
                  <Download className="w-4 h-4" />
                  Télécharger le certificat root
                </h3>
                <p className="text-sm text-gray-400 mb-3">
                  Installez ce certificat sur vos appareils pour faire confiance aux certificats émis par cette CA.
                </p>
                <div className="flex gap-2 flex-wrap">
                  <Button
                    onClick={() => handleDownloadRootCert('pem')}
                    variant="outline"
                    className="text-sm"
                  >
                    {copiedFormat === 'pem' ? (
                      <>
                        <Check className="w-4 h-4 mr-1" />
                        Téléchargé
                      </>
                    ) : (
                      <>
                        <Download className="w-4 h-4 mr-1" />
                        .pem (Linux/macOS)
                      </>
                    )}
                  </Button>
                  <Button
                    onClick={() => handleDownloadRootCert('crt')}
                    variant="outline"
                    className="text-sm"
                  >
                    {copiedFormat === 'crt' ? (
                      <>
                        <Check className="w-4 h-4 mr-1" />
                        Téléchargé
                      </>
                    ) : (
                      <>
                        <Download className="w-4 h-4 mr-1" />
                        .crt (Linux)
                      </>
                    )}
                  </Button>
                  <Button
                    onClick={() => handleDownloadRootCert('der')}
                    variant="outline"
                    className="text-sm"
                  >
                    {copiedFormat === 'der' ? (
                      <>
                        <Check className="w-4 h-4 mr-1" />
                        Téléchargé
                      </>
                    ) : (
                      <>
                        <Download className="w-4 h-4 mr-1" />
                        .der (Windows)
                      </>
                    )}
                  </Button>
                </div>
              </div>
            </div>
          )}
      </Card>
      </Section>

      {/* Installation Instructions */}
      {status?.initialized && (
        <Section title="Instructions" contrast>
            <button
              onClick={() => setShowInstructions(!showInstructions)}
              className="w-full flex items-center justify-between text-left p-4 bg-gray-800"
            >
              <span className="text-sm text-gray-300">Instructions d'installation</span>
              {showInstructions ? (
                <ChevronUp className="w-5 h-5" />
              ) : (
                <ChevronDown className="w-5 h-5" />
              )}
            </button>

            {showInstructions && (
              <div className="space-y-4 text-sm p-4">
                {/* Windows */}
                <div className="border border-gray-700 p-4">
                  <h3 className="font-semibold mb-2">Windows</h3>
                  <ol className="list-decimal list-inside space-y-1 text-gray-400">
                    <li>Téléchargez le certificat au format .der</li>
                    <li>Double-cliquez sur le fichier téléchargé</li>
                    <li>Cliquez sur "Installer le certificat..."</li>
                    <li>Sélectionnez "Ordinateur local" puis "Suivant"</li>
                    <li>Sélectionnez "Placer tous les certificats dans le magasin suivant"</li>
                    <li>Cliquez sur "Parcourir" et sélectionnez "Autorités de certification racines de confiance"</li>
                    <li>Cliquez sur "Suivant" puis "Terminer"</li>
                  </ol>
                </div>

                {/* macOS */}
                <div className="border border-gray-700 p-4">
                  <h3 className="font-semibold mb-2">macOS</h3>
                  <ol className="list-decimal list-inside space-y-1 text-gray-400">
                    <li>Téléchargez le certificat au format .pem</li>
                    <li>Double-cliquez sur le fichier pour l'ouvrir dans Trousseau d'accès</li>
                    <li>Double-cliquez sur le certificat importé</li>
                    <li>Dépliez "Se fier" et sélectionnez "Toujours faire confiance"</li>
                    <li>Fermez la fenêtre et entrez votre mot de passe</li>
                  </ol>
                </div>

                {/* Linux */}
                <div className="border border-gray-700 p-4">
                  <h3 className="font-semibold mb-2">Linux (Ubuntu/Debian)</h3>
                  <ol className="list-decimal list-inside space-y-1 text-gray-400">
                    <li>Téléchargez le certificat au format .crt</li>
                    <li>Copiez le fichier : <code className="bg-gray-800 px-1">sudo cp homeroute-root-ca.crt /usr/local/share/ca-certificates/</code></li>
                    <li>Mettez à jour les certificats : <code className="bg-gray-800 px-1">sudo update-ca-certificates</code></li>
                  </ol>
                </div>

                {/* Firefox */}
                <div className="border border-gray-700 p-4">
                  <h3 className="font-semibold mb-2">Firefox (tous OS)</h3>
                  <ol className="list-decimal list-inside space-y-1 text-gray-400">
                    <li>Ouvrez Firefox et allez dans Paramètres → Vie privée et sécurité</li>
                    <li>Descendez jusqu'à "Certificats" et cliquez sur "Afficher les certificats"</li>
                    <li>Onglet "Autorités" → "Importer"</li>
                    <li>Sélectionnez le certificat .pem téléchargé</li>
                    <li>Cochez "Confirmer cette AC pour identifier des sites web"</li>
                  </ol>
                </div>
              </div>
            )}
        </Section>
      )}

      {/* Certificates List */}
      {status?.initialized && (
        <Section title={`Certificats émis (${certificates.length})`}>
          <div className="flex items-center justify-between mb-4">
              <Button
                onClick={fetchData}
                variant="outline"
                size="sm"
              >
                <RefreshCw className="w-4 h-4" />
              </Button>
            </div>

            {certificates.length === 0 ? (
              <div className="text-center py-8 text-gray-400">
                Aucun certificat émis pour le moment
              </div>
            ) : (
              <div className="space-y-3">
                {certificates.map((cert) => {
                  const daysUntilExpiry = getDaysUntilExpiry(cert.expires_at);
                  const needsRenewal = daysUntilExpiry < 30;
                  const expired = daysUntilExpiry < 0;

                  return (
                    <div
                      key={cert.id}
                      className={`border border-gray-700 p-4 ${
                        expired
                          ? 'bg-red-900/10 border-red-800'
                          : needsRenewal
                          ? 'bg-orange-900/10 border-orange-800'
                          : ''
                      }`}
                    >
                      <div className="flex items-start justify-between gap-4">
                        <div className="flex-1 space-y-2">
                          <div className="flex items-center gap-2">
                            <Key className="w-4 h-4 text-gray-400" />
                            <span className="font-mono text-sm text-gray-400">
                              {cert.id.substring(0, 8)}...
                            </span>
                          </div>

                          <div className="flex flex-wrap gap-2">
                            {cert.domains.map((domain, idx) => (
                              <span
                                key={idx}
                                className="inline-flex items-center gap-1 px-2 py-1 bg-blue-900/30 text-blue-300 text-sm"
                              >
                                <Globe className="w-3 h-3" />
                                {domain}
                              </span>
                            ))}
                          </div>

                          <div className="flex items-center gap-4 text-sm text-gray-400">
                            <span className="flex items-center gap-1">
                              <Calendar className="w-4 h-4" />
                              Émis : {formatDate(cert.issued_at)}
                            </span>
                            <span className="flex items-center gap-1">
                              <Calendar className="w-4 h-4" />
                              Expire : {formatDate(cert.expires_at)}
                            </span>
                          </div>

                          {expired ? (
                            <div className="flex items-center gap-2 text-red-400 text-sm font-medium">
                              <AlertTriangle className="w-4 h-4" />
                              Expiré depuis {Math.abs(daysUntilExpiry)} jour(s)
                            </div>
                          ) : needsRenewal ? (
                            <div className="flex items-center gap-2 text-orange-400 text-sm font-medium">
                              <AlertTriangle className="w-4 h-4" />
                              Expire dans {daysUntilExpiry} jour(s)
                            </div>
                          ) : (
                            <div className="flex items-center gap-2 text-green-400 text-sm">
                              <CheckCircle className="w-4 h-4" />
                              Valide ({daysUntilExpiry} jours restants)
                            </div>
                          )}
                        </div>

                        <div className="flex gap-2">
                          <Button
                            onClick={() => handleRenewCertificate(cert.id)}
                            variant="outline"
                            size="sm"
                            title="Renouveler"
                          >
                            <RefreshCw className="w-4 h-4" />
                          </Button>
                          <Button
                            onClick={() => handleRevokeCertificate(cert.id)}
                            variant="outline"
                            size="sm"
                            className="text-red-400 hover:text-red-300 hover:bg-red-900/20"
                            title="Révoquer"
                          >
                            <Trash2 className="w-4 h-4" />
                          </Button>
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
        </Section>
      )}
    </div>
  );
};

export default CertificateAuthority;
