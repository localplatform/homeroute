//! HomeRoute ACME certificate management
//!
//! This crate provides Let's Encrypt certificate management using DNS-01 challenges
//! via Cloudflare API. It manages wildcard certificates for HomeRoute applications.

mod acme;
mod cloudflare;
mod storage;
pub mod types;

pub use acme::AcmeManager;
pub use types::{AcmeConfig, AcmeError, AcmeResult, CertificateInfo, WildcardType};
