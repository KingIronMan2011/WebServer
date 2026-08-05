//! TLS configuration for local PEM certificates and ACME-managed certificates.

use std::{collections::HashMap, fs::File, io::BufReader, process::Command, sync::Arc};

use futures_util::StreamExt;
use rustls::{
    ServerConfig,
    server::{ClientHello, ResolvesServerCert, ResolvesServerCertUsingSni},
    sign::CertifiedKey,
};
use rustls_acme::{AcmeConfig, ResolvesServerCertAcme, UseChallenge, caches::DirCache};
use tokio_rustls::TlsAcceptor;

use crate::{
    config::{
        Config, DnsChallengeConfig, DnsProviderConfig, LocalCertificateConfig, normalise_host,
    },
    error::{Error, Result},
};

pub struct TlsManager {
    acme_resolver: Option<Arc<ResolvesServerCertAcme>>,
    acceptor: TlsAcceptor,
}

impl TlsManager {
    pub async fn start(config: &Config) -> Result<Arc<Self>> {
        let mut certificates = config.tls.certificates.clone();
        if let Some(dns) = &config.tls.dns_challenge {
            let email = config.tls.email.as_deref().ok_or_else(|| {
                Error::Config("tls.email is required for DNS-01 certificates".into())
            })?;
            certificates.extend(
                provision_dns_certificates(dns, &config.tls.certificate_cache, email).await?,
            );
        }
        let (local_resolver, local_hosts) = load_local_certificates(&certificates)?;
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

/// Obtains DNS-01 certificates with lego. lego provides maintained integrations for
/// the major DNS APIs and follows CNAME targets and NS-delegated challenge zones.
/// Keeping credentials in a mode-600 environment file means that no API token is
/// ever persisted in the webserver configuration or process arguments.
async fn provision_dns_certificates(
    dns: &DnsChallengeConfig,
    certificate_cache: &std::path::Path,
    email: &str,
) -> Result<Vec<LocalCertificateConfig>> {
    let cache = certificate_cache.join("dns-01");
    tokio::fs::create_dir_all(&cache).await?;
    let mut certificates = Vec::with_capacity(dns.providers.len());
    for provider in &dns.providers {
        let dns = dns.clone();
        let cache = cache.clone();
        let email = email.to_owned();
        let provider = provider.clone();
        let task_provider = provider.clone();
        let task_cache = cache.clone();
        tokio::task::spawn_blocking(move || run_lego(&dns, &task_provider, &task_cache, &email))
            .await
            .map_err(|error| Error::Config(format!("DNS-01 task failed: {error}")))??;
        let primary = provider
            .domains
            .first()
            .expect("validated DNS provider domains");
        let certificate = cache.join("certificates").join(format!("{primary}.crt"));
        let private_key = cache.join("certificates").join(format!("{primary}.key"));
        certificates.push(LocalCertificateConfig {
            hosts: provider.domains,
            certificate,
            private_key,
        });
    }
    Ok(certificates)
}

fn run_lego(
    dns: &DnsChallengeConfig,
    provider: &DnsProviderConfig,
    cache: &std::path::Path,
    email: &str,
) -> Result<()> {
    let credentials = read_credentials(&provider.credentials_file)?;
    let primary = provider
        .domains
        .first()
        .expect("validated DNS provider domains");
    let certificate = cache.join("certificates").join(format!("{primary}.crt"));
    let mut command = Command::new(&dns.command);
    command
        .arg("--accept-tos")
        .arg("--email")
        .arg(email)
        .arg("--path")
        .arg(cache)
        .arg("--dns")
        .arg(&provider.provider);
    for resolver in &provider.resolvers {
        command.arg("--dns.resolvers").arg(resolver);
    }
    for domain in &provider.domains {
        command.arg("--domains").arg(domain);
    }
    for (key, value) in credentials {
        command.env(key, value);
    }
    // `renew` avoids needless reissues on every service restart. lego performs a
    // fresh DNS-01 order only when no usable certificate is cached yet.
    command.arg(if certificate.is_file() {
        "renew"
    } else {
        "run"
    });
    let output = command.output().map_err(|error| {
        Error::Config(format!(
            "could not start DNS provider client {}: {error}",
            dns.command.display()
        ))
    })?;
    if !output.status.success() {
        return Err(Error::Config(format!(
            "DNS-01 certificate request using {} failed: {}",
            provider.provider,
            String::from_utf8_lossy(&output.stderr).trim(),
        )));
    }
    Ok(())
}

fn read_credentials(path: &std::path::Path) -> Result<HashMap<String, String>> {
    let contents = std::fs::read_to_string(path)?;
    let mut credentials = HashMap::new();
    for (line_number, line) in contents.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            Error::Config(format!(
                "invalid DNS credential at {}:{}",
                path.display(),
                line_number + 1
            ))
        })?;
        if key.is_empty()
            || key
                .chars()
                .any(|character| !character.is_ascii_alphanumeric() && character != '_')
        {
            return Err(Error::Config(format!(
                "invalid DNS credential name at {}:{}",
                path.display(),
                line_number + 1
            )));
        }
        credentials.insert(key.to_owned(), value.to_owned());
    }
    if credentials.is_empty() {
        return Err(Error::Config(format!(
            "DNS credentials file is empty: {}",
            path.display()
        )));
    }
    Ok(credentials)
}

#[derive(Debug)]
struct CertificateResolver {
    local: ResolvesServerCertUsingSni,
    local_hosts: std::collections::HashSet<String>,
    acme: Option<Arc<ResolvesServerCertAcme>>,
}

impl ResolvesServerCert for CertificateResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if hello.server_name().is_some_and(|host| {
            let host = host.to_ascii_lowercase();
            self.local_hosts.iter().any(|configured| {
                configured == &host
                    || configured
                        .strip_prefix("*.")
                        .is_some_and(|suffix| host.ends_with(suffix) && host.len() > suffix.len())
            })
        }) {
            return self.local.resolve(hello);
        }
        self.acme
            .as_ref()
            .and_then(|resolver| resolver.resolve(hello))
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
