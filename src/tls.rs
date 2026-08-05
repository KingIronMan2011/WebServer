//! TLS configuration for local PEM certificates and ACME-managed certificates.

use std::{fs::File, io::BufReader, sync::Arc};

use futures_util::StreamExt;
use rustls::{
    ServerConfig,
    server::{ClientHello, ResolvesServerCert, ResolvesServerCertUsingSni},
    sign::CertifiedKey,
};
use rustls_acme::{AcmeConfig, ResolvesServerCertAcme, UseChallenge, caches::DirCache};
use tokio_rustls::TlsAcceptor;

use crate::{
    config::{Config, LocalCertificateConfig, normalise_host},
    error::{Error, Result},
};

pub struct TlsManager {
    acme_resolver: Option<Arc<ResolvesServerCertAcme>>,
    acceptor: TlsAcceptor,
}

impl TlsManager {
    pub async fn start(config: &Config) -> Result<Arc<Self>> {
        let (local_resolver, local_hosts) = load_local_certificates(&config.tls.certificates)?;
        let acme_domains = config
            .sites
            .iter()
            .map(|site| site.host.clone())
            .filter(|host| !local_hosts.contains(&normalise_host(host)))
            .collect::<Vec<_>>();
        let acme_resolver = if acme_domains.is_empty() {
            None
        } else {
            let email = config.tls.email.as_deref().ok_or_else(|| {
                Error::Config("tls.email is required for ACME-managed sites".into())
            })?;
            tokio::fs::create_dir_all(&config.tls.certificate_cache).await?;
            let mut state = AcmeConfig::new(acme_domains)
                .contact([format!("mailto:{email}")])
                .cache(DirCache::new(config.tls.certificate_cache.clone()))
                .directory_lets_encrypt(true)
                .challenge_type(UseChallenge::Http01)
                .state();
            let resolver = state.resolver();
            tokio::spawn(async move {
                while let Some(event) = state.next().await {
                    match event {
                        Ok(event) => tracing::info!(?event, "ACME certificate event"),
                        Err(error) => tracing::error!(?error, "ACME certificate error"),
                    }
                }
            });
            Some(resolver)
        };
        let resolver = CertificateResolver {
            local: local_resolver,
            local_hosts,
            acme: acme_resolver.clone(),
        };
        let server_config = ServerConfig::builder()
            .with_no_client_auth()
            .with_cert_resolver(Arc::new(resolver));
        Ok(Arc::new(Self {
            acme_resolver,
            acceptor: TlsAcceptor::from(Arc::new(server_config)),
        }))
    }

    pub fn acceptor(&self) -> TlsAcceptor {
        self.acceptor.clone()
    }

    pub fn challenge_response(&self, token: &str) -> Option<String> {
        self.acme_resolver
            .as_ref()
            .and_then(|resolver| resolver.get_http_01_key_auth(token))
    }
}

#[derive(Debug)]
struct CertificateResolver {
    local: ResolvesServerCertUsingSni,
    local_hosts: std::collections::HashSet<String>,
    acme: Option<Arc<ResolvesServerCertAcme>>,
}

impl ResolvesServerCert for CertificateResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if hello
            .server_name()
            .is_some_and(|host| self.local_hosts.contains(&host.to_ascii_lowercase()))
        {
            self.local.resolve(hello)
        } else {
            self.acme
                .as_ref()
                .and_then(|resolver| resolver.resolve(hello))
        }
    }
}

fn load_local_certificates(
    certificates: &[LocalCertificateConfig],
) -> Result<(
    ResolvesServerCertUsingSni,
    std::collections::HashSet<String>,
)> {
    let mut resolver = ResolvesServerCertUsingSni::new();
    let mut hosts = std::collections::HashSet::new();
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    for certificate in certificates {
        let chain =
            rustls_pemfile::certs(&mut BufReader::new(File::open(&certificate.certificate)?))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(|error| Error::Config(format!("invalid certificate PEM: {error}")))?;
        let key = rustls_pemfile::private_key(&mut BufReader::new(File::open(
            &certificate.private_key,
        )?))?
        .ok_or_else(|| {
            Error::Config(format!(
                "no private key found: {}",
                certificate.private_key.display()
            ))
        })?;
        let key = CertifiedKey::from_der(chain, key, &provider).map_err(|error| {
            Error::Config(format!("invalid certificate or private key: {error}"))
        })?;
        for host in &certificate.hosts {
            let host = normalise_host(host);
            resolver.add(&host, key.clone()).map_err(|error| {
                Error::Config(format!("local certificate for {host} is invalid: {error}"))
            })?;
            hosts.insert(host);
        }
    }
    Ok((resolver, hosts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[tokio::test]
    async fn starts_with_a_local_certificate_without_acme_email() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("webserver-local-tls-test-{unique}"));
        let sites = directory.join("sites");
        let public = directory.join("public");
        let certificates = directory.join("certificates");
        fs::create_dir_all(&sites).expect("create sites directory");
        fs::create_dir_all(&public).expect("create public directory");
        fs::create_dir_all(&certificates).expect("create certificate directory");
        let generated = rcgen::generate_simple_self_signed(vec!["example.test".into()])
            .expect("generate certificate");
        let certificate = certificates.join("certificate.pem");
        let private_key = certificates.join("private-key.pem");
        fs::write(&certificate, generated.cert.pem()).expect("write certificate");
        fs::write(&private_key, generated.key_pair.serialize_pem()).expect("write private key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&private_key, fs::Permissions::from_mode(0o640))
                .expect("restrict private key permissions");
        }

        let path = directory.join("webserver.toml");
        let toml_path = |path: &std::path::Path| path.display().to_string().replace('\\', "\\\\");
        fs::write(
            &path,
            format!(
                "[server]\nbind = \"127.0.0.1:0\"\n\n[tls]\nenabled = true\n\n[[tls.certificates]]\nhosts = [\"example.test\"]\ncertificate = \"{}\"\nprivate_key = \"{}\"\n",
                toml_path(&certificate),
                toml_path(&private_key),
            ),
        )
        .expect("write configuration");
        fs::write(
            sites.join("example.test.conf"),
            "host = \"example.test\"\n[[routes]]\npath_prefix = \"/\"\nkind = \"static\"\nroot = \"../public\"\n",
        )
        .expect("write site configuration");

        let config = Config::load(&path).expect("load configuration");
        config
            .validate()
            .expect("validate local certificate configuration");
        let _tls = TlsManager::start(&config)
            .await
            .expect("start local TLS manager");
        fs::remove_dir_all(directory).expect("remove test directory");
    }
}
