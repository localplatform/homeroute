//! HTML templates for loading and down pages.

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Response, StatusCode};

/// The body type used by the proxy handler.
pub type BoxBody = http_body_util::combinators::BoxBody<Bytes, std::convert::Infallible>;

/// HTML template for the loading page (service starting).
const LOADING_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Starting...</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
            color: #e5e7eb;
            display: flex;
            align-items: center;
            justify-content: center;
            height: 100vh;
            text-align: center;
        }
        .container {
            padding: 2rem;
            max-width: 400px;
        }
        .spinner {
            width: 48px;
            height: 48px;
            border: 4px solid #374151;
            border-top-color: #3b82f6;
            border-radius: 50%;
            animation: spin 1s linear infinite;
            margin: 0 auto 1.5rem;
        }
        @keyframes spin {
            to { transform: rotate(360deg); }
        }
        h1 {
            font-size: 1.5rem;
            font-weight: 600;
            margin-bottom: 0.5rem;
            color: #f3f4f6;
        }
        p {
            color: #9ca3af;
            font-size: 0.95rem;
            line-height: 1.5;
        }
        .service-name {
            color: #60a5fa;
            font-weight: 500;
        }
        .footer {
            margin-top: 2rem;
            font-size: 0.75rem;
            color: #6b7280;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="spinner"></div>
        <h1>Starting Service</h1>
        <p>The <span class="service-name">{{SERVICE_NAME}}</span> is waking up from power-save mode.</p>
        <p class="footer" id="status">Connecting...</p>
    </div>
    <script>
        (function() {
            const statusEl = document.getElementById('status');
            let polling = true;

            async function checkStatus() {
                if (!polling) return;
                try {
                    const resp = await fetch('/_hr/status');
                    if (!resp.ok) {
                        statusEl.textContent = 'Checking service status...';
                        setTimeout(checkStatus, 500);
                        return;
                    }
                    const data = await resp.json();
                    console.log('[loading] status:', data);
                    if (data.codeServerStatus === 'running' || data.appStatus === 'running') {
                        polling = false;
                        statusEl.textContent = 'Service ready! Redirecting...';
                        location.reload();
                    } else {
                        statusEl.textContent = 'Waiting for service to start...';
                        setTimeout(checkStatus, 500);
                    }
                } catch (err) {
                    console.log('[loading] fetch error:', err);
                    statusEl.textContent = 'Checking...';
                    setTimeout(checkStatus, 1000);
                }
            }

            checkStatus();
        })();
    </script>
</body>
</html>"#;

/// HTML template for the down page (service manually stopped).
const DOWN_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>Service Stopped</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
            color: #e5e7eb;
            display: flex;
            align-items: center;
            justify-content: center;
            height: 100vh;
            text-align: center;
        }
        .container {
            padding: 2rem;
            max-width: 400px;
        }
        .icon {
            width: 64px;
            height: 64px;
            background: #7f1d1d;
            border-radius: 50%;
            display: flex;
            align-items: center;
            justify-content: center;
            margin: 0 auto 1.5rem;
        }
        .icon svg {
            width: 32px;
            height: 32px;
            fill: #fca5a5;
        }
        h1 {
            font-size: 1.5rem;
            font-weight: 600;
            margin-bottom: 0.5rem;
            color: #f3f4f6;
        }
        p {
            color: #9ca3af;
            font-size: 0.95rem;
            line-height: 1.5;
            margin-bottom: 0.5rem;
        }
        .service-name {
            color: #f87171;
            font-weight: 500;
        }
        a {
            color: #60a5fa;
            text-decoration: none;
        }
        a:hover {
            text-decoration: underline;
        }
        .btn {
            display: inline-block;
            margin-top: 1.5rem;
            padding: 0.75rem 1.5rem;
            background: #3b82f6;
            color: white;
            border-radius: 0.5rem;
            font-weight: 500;
            transition: background 0.2s;
        }
        .btn:hover {
            background: #2563eb;
            text-decoration: none;
        }
    </style>
</head>
<body>
    <div class="container">
        <div class="icon">
            <svg viewBox="0 0 24 24" xmlns="http://www.w3.org/2000/svg">
                <rect x="6" y="4" width="12" height="16" rx="2" fill="none" stroke="currentColor" stroke-width="2"/>
                <line x1="9" y1="9" x2="15" y2="9" stroke="currentColor" stroke-width="2"/>
            </svg>
        </div>
        <h1>Service Stopped</h1>
        <p>The <span class="service-name">{{SERVICE_NAME}}</span> has been manually stopped.</p>
        <p>To start it, go to the HomeRoute dashboard.</p>
        <a href="{{DASHBOARD_URL}}" class="btn">Open Dashboard</a>
    </div>
</body>
</html>"#;

/// Generate a loading page response.
pub fn loading_response(service_name: &str, dashboard_url: &str, app_id: &str) -> Response<BoxBody> {
    let html = LOADING_HTML
        .replace("{{SERVICE_NAME}}", service_name)
        .replace("{{DASHBOARD_URL}}", dashboard_url)
        .replace("{{APP_ID}}", app_id);

    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Retry-After", "5")
        .header("Cache-Control", "no-cache, no-store, must-revalidate")
        .body(
            Full::new(Bytes::from(html))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed(),
        )
        .unwrap()
}

/// Generate a down page response.
pub fn down_response(service_name: &str, dashboard_url: &str) -> Response<BoxBody> {
    let html = DOWN_HTML
        .replace("{{SERVICE_NAME}}", service_name)
        .replace("{{DASHBOARD_URL}}", dashboard_url);

    Response::builder()
        .status(StatusCode::SERVICE_UNAVAILABLE)
        .header("Content-Type", "text/html; charset=utf-8")
        .header("Cache-Control", "no-cache, no-store, must-revalidate")
        .body(
            Full::new(Bytes::from(html))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed(),
        )
        .unwrap()
}
