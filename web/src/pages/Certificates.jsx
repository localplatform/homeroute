import { useState, useEffect } from 'react';
import {
  Lock,
  RefreshCw,
  CheckCircle,
  AlertTriangle,
  Calendar,
  Globe,
  ExternalLink,
  Shield
} from 'lucide-react';
import Card from '../components/Card';
import Button from '../components/Button';
import PageHeader from '../components/PageHeader';
import Section from '../components/Section';

const Certificates = () => {
  const [status, setStatus] = useState(null);
  const [certificates, setCertificates] = useState([]);
  const [loading, setLoading] = useState(true);
  const [renewing, setRenewing] = useState(false);
  const [message, setMessage] = useState(null);

  useEffect(() => {
    fetchData();
  }, []);

  async function fetchData() {
    try {
      const [statusRes, certsRes] = await Promise.all([
        fetch('/api/acme/status'),
        fetch('/api/acme/certificates'),
      ]);

      const statusData = await statusRes.json();
      const certsData = await certsRes.json();

      setStatus(statusData);
      if (certsData.success) {
        setCertificates(certsData.certificates || []);
      }
    } catch (error) {
      console.error('Error fetching ACME data:', error);
      setMessage({ type: 'error', text: 'Erreur de chargement' });
    } finally {
      setLoading(false);
    }
  }

  async function handleRenewAll() {
    setRenewing(true);
    setMessage(null);

    try {
      const response = await fetch('/api/acme/renew', {
        method: 'POST',
      });

      const data = await response.json();

      if (data.success) {
        setMessage({ type: 'success', text: 'Renouvellement effectue avec succes' });
        fetchData();
      } else {
        setMessage({ type: 'error', text: data.error || 'Erreur lors du renouvellement' });
      }
    } catch (error) {
      setMessage({ type: 'error', text: error.message });
    } finally {
      setRenewing(false);
    }
  }

  function formatDate(dateString) {
    return new Date(dateString).toLocaleDateString('fr-FR', {
      year: 'numeric',
      month: 'long',
      day: 'numeric',
    });
  }

  function getDaysUntilExpiry(expiresAt) {
    const now = new Date();
    const expiry = new Date(expiresAt);
    const diffTime = expiry - now;
    const diffDays = Math.ceil(diffTime / (1000 * 60 * 60 * 24));
    return diffDays;
  }

  function getTypeLabel(wildcardType) {
    switch (wildcardType) {
      case 'Main':
        return 'Applications';
      case 'Code':
        return 'Code Server';
      default:
        return wildcardType;
    }
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
      <PageHeader title="Certificats TLS" icon={Lock} />

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

      <Section title="Fournisseur">
        <Card title="Let's Encrypt" icon={Shield}>
          <div className="space-y-4">
            <div className="flex items-center gap-3">
              <div className="flex items-center gap-2">
                <CheckCircle className="w-5 h-5 text-green-400" />
                <span className="font-medium text-green-400">Actif</span>
              </div>
              <span className="px-2 py-1 bg-blue-900/30 text-blue-300 text-sm rounded">
                Certificats wildcards
              </span>
            </div>

            <p className="text-sm text-gray-400">
              Les certificats sont emis par Let's Encrypt et renouveles automatiquement 30 jours avant expiration.
              Ils sont reconnus par tous les navigateurs sans configuration.
            </p>

            <div className="flex items-center gap-4 pt-2">
              <Button
                onClick={handleRenewAll}
                disabled={renewing}
                variant="outline"
              >
                <RefreshCw className={`w-4 h-4 mr-2 ${renewing ? 'animate-spin' : ''}`} />
                {renewing ? 'Renouvellement...' : 'Forcer le renouvellement'}
              </Button>

              <a
                href="https://letsencrypt.org/"
                target="_blank"
                rel="noopener noreferrer"
                className="flex items-center gap-1 text-sm text-blue-400 hover:text-blue-300"
              >
                <ExternalLink className="w-4 h-4" />
                letsencrypt.org
              </a>
            </div>
          </div>
        </Card>
      </Section>

      <Section title={`Certificats (${certificates.length})`}>
        <div className="flex items-center justify-end mb-4">
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
            Aucun certificat disponible. Les certificats seront emis automatiquement.
          </div>
        ) : (
          <div className="space-y-px">
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
                    <div className="flex-1 space-y-3">
                      <div className="flex items-center gap-3">
                        <div className="flex items-center gap-2">
                          <Globe className="w-5 h-5 text-blue-400" />
                          <span className="font-medium text-lg">
                            {cert.domains && cert.domains.length > 0
                              ? cert.domains[0]
                              : `*.${cert.wildcard_type === 'Code' ? 'code.' : ''}mynetwk.biz`}
                          </span>
                        </div>
                        <span className="px-2 py-0.5 bg-gray-700 text-gray-300 text-xs rounded">
                          {getTypeLabel(cert.wildcard_type)}
                        </span>
                      </div>

                      <div className="flex items-center gap-6 text-sm text-gray-400">
                        <span className="flex items-center gap-1">
                          <Calendar className="w-4 h-4" />
                          Emis : {formatDate(cert.issued_at)}
                        </span>
                        <span className="flex items-center gap-1">
                          <Calendar className="w-4 h-4" />
                          Expire : {formatDate(cert.expires_at)}
                        </span>
                      </div>

                      {expired ? (
                        <div className="flex items-center gap-2 text-red-400 text-sm font-medium">
                          <AlertTriangle className="w-4 h-4" />
                          Expire depuis {Math.abs(daysUntilExpiry)} jour(s)
                        </div>
                      ) : needsRenewal ? (
                        <div className="flex items-center gap-2 text-orange-400 text-sm font-medium">
                          <AlertTriangle className="w-4 h-4" />
                          Renouvellement prevu dans {daysUntilExpiry} jour(s)
                        </div>
                      ) : (
                        <div className="flex items-center gap-2 text-green-400 text-sm">
                          <CheckCircle className="w-4 h-4" />
                          Valide ({daysUntilExpiry} jours restants)
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </Section>
    </div>
  );
};

export default Certificates;
