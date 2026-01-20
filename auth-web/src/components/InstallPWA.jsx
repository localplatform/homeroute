import React, { useState, useEffect } from 'react';
import { Download, X, Share } from 'lucide-react';

function InstallPWA() {
  const [showPrompt, setShowPrompt] = useState(false);
  const [deferredPrompt, setDeferredPrompt] = useState(null);
  const [isIOS, setIsIOS] = useState(false);
  const [isStandalone, setIsStandalone] = useState(false);

  useEffect(() => {
    // Check if running as standalone PWA
    const standalone = window.matchMedia('(display-mode: standalone)').matches ||
                      window.navigator.standalone ||
                      document.referrer.includes('android-app://');
    setIsStandalone(standalone);

    // Detect iOS
    const ios = /iPad|iPhone|iPod/.test(navigator.userAgent) && !window.MSStream;
    setIsIOS(ios);

    // Check if already dismissed
    const dismissed = localStorage.getItem('pwa-install-dismissed');
    if (dismissed) return;

    // Show prompt if not standalone
    if (!standalone) {
      setShowPrompt(true);
    }

    // Listen for beforeinstallprompt (Android/Desktop)
    const handleBeforeInstall = (e) => {
      e.preventDefault();
      setDeferredPrompt(e);
      setShowPrompt(true);
    };

    window.addEventListener('beforeinstallprompt', handleBeforeInstall);

    return () => {
      window.removeEventListener('beforeinstallprompt', handleBeforeInstall);
    };
  }, []);

  const handleInstall = async () => {
    if (deferredPrompt) {
      deferredPrompt.prompt();
      const { outcome } = await deferredPrompt.userChoice;
      if (outcome === 'accepted') {
        setShowPrompt(false);
      }
      setDeferredPrompt(null);
    }
  };

  const handleDismiss = () => {
    setShowPrompt(false);
    localStorage.setItem('pwa-install-dismissed', 'true');
  };

  if (isStandalone || !showPrompt) return null;

  return (
    <div className="fixed bottom-4 left-4 right-4 z-50 animate-slideUp">
      <div className="max-w-md mx-auto bg-gray-800/95 backdrop-blur-sm rounded-xl p-4 shadow-2xl border border-gray-700">
        <div className="flex items-start gap-3">
          <div className="flex-shrink-0 w-10 h-10 bg-blue-600 rounded-lg flex items-center justify-center">
            <Download className="w-5 h-5 text-white" />
          </div>

          <div className="flex-1 min-w-0">
            <h3 className="text-white font-semibold mb-1">
              Installer l'Authenticator
            </h3>

            {isIOS ? (
              <div className="text-sm text-gray-300 space-y-2">
                <p>Pour installer sur iOS:</p>
                <ol className="list-decimal list-inside space-y-1 text-gray-400">
                  <li>Appuyez sur le bouton <Share className="inline w-4 h-4" /> Partager</li>
                  <li>Sélectionnez "Sur l'écran d'accueil"</li>
                  <li>Appuyez sur "Ajouter"</li>
                </ol>
              </div>
            ) : deferredPrompt ? (
              <p className="text-sm text-gray-300">
                Installez l'app pour un accès rapide et des notifications push
              </p>
            ) : (
              <p className="text-sm text-gray-300">
                Accédez rapidement à vos codes d'authentification depuis votre écran d'accueil
              </p>
            )}

            {deferredPrompt && !isIOS && (
              <button
                onClick={handleInstall}
                className="mt-3 w-full py-2 px-4 bg-blue-600 hover:bg-blue-700 text-white text-sm font-medium rounded-lg transition-colors"
              >
                Installer maintenant
              </button>
            )}
          </div>

          <button
            onClick={handleDismiss}
            className="flex-shrink-0 p-1 text-gray-400 hover:text-white transition-colors"
            aria-label="Fermer"
          >
            <X className="w-5 h-5" />
          </button>
        </div>
      </div>
    </div>
  );
}

export default InstallPWA;
