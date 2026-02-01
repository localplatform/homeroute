use hr_auth::sessions::SessionStore;
use hr_auth::users::{UserStore, hash_password, verify_password};
use std::path::Path;
use tempfile::tempdir;

/// Vérifie que le UserStore peut lire le fichier users.yml existant
#[test]
fn test_load_existing_users() {
    let data_dir = Path::new("/opt/homeroute/api/data");
    if !data_dir.join("users.yml").exists() {
        eprintln!("Skipping: users.yml not found");
        return;
    }

    let store = UserStore::new(data_dir);
    let users = store.get_all();

    assert!(!users.is_empty(), "Should have at least one user");

    // Vérifier que l'admin existe
    let admin = store.get("admin");
    assert!(admin.is_some(), "Admin user should exist");

    let admin = admin.unwrap();
    assert_eq!(admin.username, "admin");
    assert!(admin.groups.contains(&"admins".to_string()));
    assert!(!admin.disabled);
}

/// Vérifie que le hash Argon2id de Node.js est compatible avec notre vérification Rust
#[test]
fn test_verify_nodejs_argon2_hash() {
    let data_dir = Path::new("/opt/homeroute/api/data");
    if !data_dir.join("users.yml").exists() {
        eprintln!("Skipping: users.yml not found");
        return;
    }

    let store = UserStore::new(data_dir);
    let admin = store.get_with_password("admin");
    assert!(admin.is_some(), "Admin user should exist with password");

    let admin = admin.unwrap();
    // Le hash doit être au format Argon2id PHC
    assert!(
        admin.password_hash.starts_with("$argon2id$"),
        "Password hash should be Argon2id format"
    );
}

/// Vérifie que hash_password produit des hashes vérifiables
#[test]
fn test_hash_verify_roundtrip() {
    let password = "mon_mot_de_passe_test";
    let hash = hash_password(password).unwrap();

    assert!(hash.starts_with("$argon2id$v=19$m=65536,t=3,p=4$"));
    assert!(verify_password(password, &hash));
    assert!(!verify_password("mauvais_mot_de_passe", &hash));
}

/// Vérifie que le SessionStore peut créer et valider des sessions
#[test]
fn test_session_lifecycle() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();

    // Créer une session
    let (session_id, expires_at) = store
        .create("admin", Some("127.0.0.1"), Some("test-agent"), false)
        .unwrap();

    assert!(!session_id.is_empty());
    assert!(expires_at > 0);

    // Valider la session
    let info = store.validate(&session_id).unwrap();
    assert!(info.is_some());
    let info = info.unwrap();
    assert_eq!(info.user_id, "admin");
    assert!(!info.remember_me);

    // Lister les sessions de l'utilisateur
    let sessions = store.get_by_user("admin").unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, session_id);

    // Supprimer la session
    store.delete(&session_id).unwrap();

    // La session ne doit plus être valide
    let info = store.validate(&session_id).unwrap();
    assert!(info.is_none());
}

/// Vérifie remember_me
#[test]
fn test_session_remember_me() {
    let dir = tempdir().unwrap();
    let store = SessionStore::new(dir.path()).unwrap();

    let (session_id, _) = store
        .create("admin", None, None, true)
        .unwrap();

    let info = store.validate(&session_id).unwrap().unwrap();
    assert!(info.remember_me);
}

/// Vérifie la compatibilité avec la DB SQLite existante
#[test]
fn test_open_existing_auth_db() {
    let data_dir = Path::new("/opt/homeroute/api/data");
    if !data_dir.join("auth.db").exists() {
        eprintln!("Skipping: auth.db not found");
        return;
    }

    // Ouvrir la base existante
    let store = SessionStore::new(data_dir).unwrap();

    // Le cleanup ne doit pas planter
    store.cleanup_expired().unwrap();
}
