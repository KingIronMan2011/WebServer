# Webserver

Ein in Rust geschriebener Webserver und Reverse Proxy mit einer schlanken
TOML-Konfiguration. Das Projekt bündelt Static-File-Serving, TLS,
Load-Balancing, Observability und moderne HTTP-Protokolle in einem Binary.

![Lizenz: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)

**Aktueller Stand: v0.6.0.** Der vollständige nächste Ausbauplan steht in
[TODO.md](TODO.md).

## Inhalt

- [Schnellstart](#schnellstart)
- [Installation auf Debian und Ubuntu](#installation-auf-debian-und-ubuntu)
- [Betrieb](#betrieb)
- [Konfiguration](#konfiguration)
- [TLS und HTTP/3](#tls-und-http3)
- [Upstreams und Discovery](#upstreams-und-discovery)
- [Sicherheit und Observability](#sicherheit-und-observability)
- [Entwicklung](#entwicklung)

## Schnellstart

Für die lokale Entwicklung genügt eine aktuelle Rust-Toolchain:

```sh
cargo run -- init
cargo run -- check
cargo run -- run
```

`init` legt eine globale Konfiguration, eine erste Site-Datei und ein
Startdokument an. `check` validiert die Konfiguration ohne einen Port zu
öffnen. `run` startet den Server.

Die Standardstruktur ist:

```text
webserver.toml              # globale Server- und TLS-Einstellungen
sites/
  example.com.conf          # eine virtuelle Site je Datei
public/                     # statische Inhalte
```

## Installation auf Debian und Ubuntu

Das signierte APT-Repository stellt derzeit `amd64`-Pakete bereit:

```sh
sudo apt update
sudo apt install -y ca-certificates curl

curl -fsSL https://repo.kingironman.dev/webserver-archive-keyring.gpg \
  | sudo tee /usr/share/keyrings/webserver-archive-keyring.gpg >/dev/null

echo 'deb [signed-by=/usr/share/keyrings/webserver-archive-keyring.gpg] https://repo.kingironman.dev stable main' \
  | sudo tee /etc/apt/sources.list.d/webserver.list >/dev/null

sudo apt update
sudo apt install webserver
sudo systemctl enable --now webserver
```

Danach liegen Konfiguration und Inhalte unter `/etc/webserver/` beziehungsweise
`/var/www/webserver/`. Der Dienst läuft als `www-data` und nutzt systemd
Hardening sowie `CAP_NET_BIND_SERVICE` für Ports unter 1024.

## Betrieb

| Aufgabe | Befehl |
| --- | --- |
| Konfiguration prüfen | `webserver check --config /etc/webserver/webserver.toml` |
| Dienst starten | `sudo systemctl start webserver` |
| Routen neu laden | `sudo systemctl reload webserver` |
| Logs verfolgen | `sudo journalctl -u webserver -f` |

Ein Reload (`SIGHUP`) übernimmt Routen- und Serveränderungen, sofern die neue
Konfiguration gültig ist. Änderungen an TLS-Listenern oder Zertifikaten
benötigen einen Neustart.

### Zero-Downtime-Binary-Upgrade unter Linux

Der Server verwendet auf Linux `SO_REUSEPORT`. Eine Ersatz-Binary bindet die
HTTP-, HTTPS- und QUIC-Ports zuerst; die bisherige Generation nimmt danach
keine neuen Verbindungen mehr an und beendet laufende Requests sauber.

```sh
sudo install -m 755 ./target/release/webserver /usr/local/bin/webserver.new
sudo mv -f /usr/local/bin/webserver.new /usr/local/bin/webserver
sudo systemctl kill -s USR2 webserver.service
```

Falls die Ersatz-Binary unmittelbar fehlschlägt, läuft die bisherige Generation
weiter.

## Konfiguration

Die globale Datei enthält Listener, Limits, Metriken und TLS. Jede Datei im
Verzeichnis `sites/` beschreibt genau einen virtuellen Host. Bei mehreren
passenden Routen gewinnt das längste `path_prefix`.

```toml
# /etc/webserver/webserver.toml
[server]
bind = "0.0.0.0:80"
upstream_timeout_secs = 30
max_header_bytes = 32768
max_body_bytes = 10485760
max_connections = 1024
# rate_limit_per_minute = 120
# allow_ips = ["192.0.2.0/24"]
# deny_ips = ["198.51.100.10/32"]
# trusted_proxies = ["127.0.0.1/32", "::1/128"]
# metrics_path = "/metrics"

[tls]
enabled = true
bind = "0.0.0.0:443"
http3 = true
email = "admin@example.com"
certificate_cache = "/etc/webserver/certificates/acme"
```

```toml
# /etc/webserver/sites/example.com.conf
host = "example.com"

[[routes]]
path_prefix = "/"
kind = "static"
root = "/var/www/webserver/example.com"
index_file = "index.html"

[[routes]]
path_prefix = "/api"
kind = "proxy"
upstream = "http://127.0.0.1:3000"
```

### Statische Dateien und Redirects

Static Files werden gestreamt und unterstützen Range Requests, `ETag`,
`Last-Modified`, Conditional Requests, gzip und Brotli. Response-Header,
Fehlerseiten und Redirects sind pro Route konfigurierbar:

```toml
[[routes]]
path_prefix = "/assets"
kind = "static"
root = "/var/www/webserver/assets"
response_headers = { cache-control = "public, max-age=3600", x-content-type-options = "nosniff" }
error_pages = { "404" = "/var/www/webserver/errors/not-found.html" }

[[routes]]
path_prefix = "/old"
kind = "redirect"
location = "https://example.com/new"
status = 308
```

## TLS und HTTP/3

Mit aktiviertem TLS leitet Port 80 reguläre Anfragen auf HTTPS um und bedient
nur ACME-HTTP-01-Challenges direkt. HTTP/2 wird auf dem TLS-Listener angeboten.
`http3 = true` startet zusätzlich QUIC auf UDP/443 und fügt HTTPS-Antworten
automatisch `Alt-Svc` hinzu, damit Browser auf HTTP/3 wechseln können.

Für einen abweichenden UDP-Port:

```toml
[tls]
enabled = true
bind = "0.0.0.0:443"
http3 = true
quic_bind = "0.0.0.0:8443"
```

Neben automatisch verwalteten Let's-Encrypt-Zertifikaten werden lokale
PEM-Zertifikate und interne CAs unterstützt:

```toml
[[tls.certificates]]
hosts = ["internal.example.com"]
certificate = "/etc/webserver/certificates/local/internal.fullchain.pem"
private_key = "/etc/webserver/certificates/local/internal.key.pem"
```

Für Wildcard-Zertifikate kann DNS-01 über den separat installierten
[lego](https://go-acme.github.io/lego/dns/)-Client verwendet werden:

```toml
[tls.dns_challenge]
command = "/usr/bin/lego"

[[tls.dns_challenge.providers]]
provider = "cloudflare"
domains = ["example.com", "*.example.com"]
credentials_file = "/etc/webserver/dns/cloudflare.env"
```

Private Schlüssel und DNS-Zugangsdaten müssen für andere Benutzer unlesbar
sein; auf Linux ist für die Credentials-Datei üblicherweise `0600` passend.

## Upstreams und Discovery

Proxy-Routen unterstützen mehrere Upstreams, Round Robin, gewichtetes Round
Robin, Least Connections, aktive und passive Health Checks, Retries mit Backoff
und Limits je Upstream.

```toml
[[routes]]
path_prefix = "/api"
kind = "proxy"
load_balancing = "least_connections"
upstreams = [
  { url = "http://127.0.0.1:3000", weight = 3 },
  { url = "http://127.0.0.1:3001", weight = 1 },
]
retries = 2
retry_backoff_ms = 100
max_connections_per_upstream = 100

[routes.health_check]
path = "/health"
interval_secs = 10
timeout_secs = 3
```

Discovery kann statische Upstreams ergänzen oder vollständig ersetzen.

```toml
# DNS
[routes.dns_discovery]
host = "api.internal.example"
port = 3000

# Docker: benötigt nur lesenden Zugriff auf den Docker-Socket.
[routes.docker_discovery]
labels = { "webserver.discovery" = "api" }
port = 3000
socket = "/var/run/docker.sock"
refresh_secs = 30

# Kubernetes-Service-DNS, auch für Headless Services.
[routes.kubernetes_discovery]
service = "api"
namespace = "production"
port = 3000
cluster_domain = "cluster.local"
```

## Sicherheit und Observability

- Limits für Header, Body, Requests pro IP und gleichzeitige Verbindungen
- Allow-/Deny-Netze und vertrauenswürdige Proxy-Netze für `X-Forwarded-For`
- CORS- sowie beliebige Response-Header-Regeln
- JSON-Logs mit `WEBSERVER_LOG_FORMAT=json`
- Prometheus-Endpunkt über `server.metrics_path`
- OpenTelemetry über die üblichen `OTEL_EXPORTER_OTLP_*`-Umgebungsvariablen
- Embedded Standardfehlerseiten für gängige 4xx- und 5xx-Statuscodes

Metriken sollten ausschließlich über ein privates Netzwerk oder einen
vertrauenswürdigen vorgeschalteten Proxy verfügbar sein.

## CLI

Neben `init`, `check` und `run` kann die CLI Sites und Routen verwalten:

```sh
webserver site-add --host example.test
webserver route-add --host example.test --path / --static ./public
webserver route-add --host example.test --path /api --upstream http://127.0.0.1:3000
webserver route-remove --host example.test --path /api
webserver completion bash
```

## Entwicklung

### Verwaltungs-API-Kompatibilität

Die Verwaltungs-API ist unter `/api/v1` versioniert. Nicht-brechende Felder
und Endpunkte können innerhalb von `v1` ergänzt werden; Umbenennungen,
Entfernungen oder Änderungen der Bedeutung erfordern eine neue Hauptversion
unter `/api/v2`. Jede Antwort enthält `X-Webserver-Api-Version: 1`.

SQLite-Migrationen der Verwaltungsdatenbank werden beim Start atomar erfasst.
Vor einem Downgrade muss die Datenbank aus einem Backup wiederhergestellt
werden; ein Binary mit niedrigerer API-/Schema-Version darf keine neuere
Verwaltungsdatenbank verändern.

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked -- -D warnings
cargo build --locked --release
```

Für eine lokale Paketinstallation auf systemd-basierten Linux-Systemen:

```sh
sudo packaging/install.sh ./target/release/webserver
```

## Roadmap

V0.6 ist abgeschlossen. Als Nächstes folgen eine lokale Verwaltungs-API
(V0.7), eine einheitliche Verwaltungs- und Zugriffsschicht (V0.8), ein
Web-Dashboard (V0.9) und danach die stabile V1.0-Oberfläche. Details stehen in
[TODO.md](TODO.md).

## Lizenz

Dieses Projekt steht unter der [MIT-Lizenz](LICENSE).
