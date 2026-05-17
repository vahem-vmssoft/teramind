//! Load rustls server config from PEM-encoded cert + key files.

use crate::config::TlsConfig;
use anyhow::{Context, anyhow};
use rustls_pemfile::Item;
use std::fs::File;
use std::io::BufReader;
use std::sync::Arc;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;

pub fn rustls_config(tls: &TlsConfig) -> anyhow::Result<Arc<RustlsServerConfig>> {
    let mut cert_reader = BufReader::new(File::open(&tls.cert_file)
        .with_context(|| format!("open cert {}", tls.cert_file.display()))?);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<_, _>>().context("parse cert PEM")?;
    if certs.is_empty() {
        return Err(anyhow!("no certificates found in {}", tls.cert_file.display()));
    }

    let mut key_reader = BufReader::new(File::open(&tls.key_file)
        .with_context(|| format!("open key {}", tls.key_file.display()))?);
    let key = rustls_pemfile::read_one(&mut key_reader)?
        .ok_or_else(|| anyhow!("no key in {}", tls.key_file.display()))?;
    let key: PrivateKeyDer<'static> = match key {
        Item::Pkcs8Key(k) => PrivateKeyDer::Pkcs8(k),
        Item::Pkcs1Key(k) => PrivateKeyDer::Pkcs1(k),
        Item::Sec1Key(k)  => PrivateKeyDer::Sec1(k),
        other => return Err(anyhow!("unsupported key type: {other:?}")),
    };

    let cfg = RustlsServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    Ok(Arc::new(cfg))
}
