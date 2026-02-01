pub mod routes;
pub mod state;

use axum::Router;
use state::ApiState;
use tower_http::services::{ServeDir, ServeFile};

/// Build the complete API router with all `/api/*` routes.
///
/// Non-API requests are served from `web_dist_path` (React SPA).
/// Any path that doesn't match a static file falls back to `index.html` (200).
pub fn build_router(state: ApiState) -> Router {
    let web_dist = state.env.web_dist_path.clone();
    let index_html = web_dist.join("index.html");

    let spa_fallback = ServeDir::new(&web_dist)
        .fallback(ServeFile::new(&index_html));

    Router::new()
        .nest("/api", api_routes())
        .with_state(state)
        .fallback_service(spa_fallback)
}

fn api_routes() -> Router<ApiState> {
    Router::new()
        .nest("/auth", routes::auth::router())
        .nest("/users", routes::users::router())
        .nest("/dns-dhcp", routes::dns_dhcp::router())
        .nest("/dns", routes::dns::router())
        .nest("/adblock", routes::adblock::router())
        .nest("/network", routes::network::router())
        .nest("/nat", routes::nat::router())
        .nest("/ddns", routes::ddns::router())
        .nest("/reverseproxy", routes::reverseproxy::router())
        .nest("/rust-proxy", routes::rust_proxy::router())
        .nest("/ca", routes::ca::router())
        .nest("/energy", routes::energy::router())
        .nest("/updates", routes::updates::router())
        .nest("/traffic", routes::traffic::router())
        .nest("/servers", routes::servers::router())
        .nest("/wol", routes::wol::router())
        .merge(routes::ws::router())
        .merge(routes::health::router())
}
