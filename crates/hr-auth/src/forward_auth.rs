use crate::users::UserInfo;
use crate::AuthService;
use std::sync::Arc;

/// Résultat d'une vérification forward-auth
pub enum ForwardAuthResult {
    /// Authentification réussie
    Success {
        user: UserInfo,
    },
    /// Non authentifié — rediriger vers login
    Unauthorized {
        login_url: String,
    },
    /// Authentifié mais accès refusé (groupes insuffisants)
    Forbidden {
        message: String,
    },
}

/// Headers à injecter dans la réponse en cas de succès
pub struct ForwardAuthHeaders {
    pub remote_user: String,
    pub remote_email: String,
    pub remote_name: String,
    pub remote_groups: String,
}

impl From<&UserInfo> for ForwardAuthHeaders {
    fn from(user: &UserInfo) -> Self {
        Self {
            remote_user: user.username.clone(),
            remote_email: user.email.clone(),
            remote_name: user.displayname.clone(),
            remote_groups: user.groups.join(","),
        }
    }
}

/// Vérifie l'authentification pour le reverse proxy
///
/// Appelé directement (sans HTTP) depuis le proxy pour chaque requête authentifiée.
pub fn check_forward_auth(
    auth: &Arc<AuthService>,
    session_cookie: Option<&str>,
    forwarded_host: &str,
    forwarded_uri: &str,
    forwarded_proto: &str,
    allowed_groups: &[String],
) -> ForwardAuthResult {
    let original_url = format!("{}://{}{}", forwarded_proto, forwarded_host, forwarded_uri);
    let login_url = format!(
        "https://auth.{}/login?rd={}",
        auth.base_domain,
        urlencoded(&original_url)
    );

    // Pas de cookie de session
    let Some(session_id) = session_cookie else {
        return ForwardAuthResult::Unauthorized { login_url };
    };

    // Valider la session
    let session = match auth.sessions.validate(session_id) {
        Ok(Some(s)) => s,
        _ => return ForwardAuthResult::Unauthorized { login_url },
    };

    // Récupérer l'utilisateur
    let Some(user) = auth.users.get(&session.user_id) else {
        return ForwardAuthResult::Unauthorized { login_url };
    };

    // Vérifier que le compte n'est pas désactivé
    if user.disabled {
        return ForwardAuthResult::Forbidden {
            message: "Account disabled".to_string(),
        };
    }

    // Vérifier les groupes d'accès
    let is_admin = user.groups.contains(&"admins".to_string());

    if !is_admin && !allowed_groups.is_empty() {
        let has_access = allowed_groups.iter().any(|g| user.groups.contains(g));
        if !has_access {
            return ForwardAuthResult::Forbidden {
                message: "Access denied: insufficient group permissions".to_string(),
            };
        }
    }

    ForwardAuthResult::Success { user }
}

/// URL-encode basique
fn urlencoded(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => result.push(c),
            _ => {
                for byte in c.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencoded() {
        assert_eq!(urlencoded("hello"), "hello");
        assert_eq!(
            urlencoded("https://example.com/path?a=1"),
            "https%3A%2F%2Fexample.com%2Fpath%3Fa%3D1"
        );
    }
}
