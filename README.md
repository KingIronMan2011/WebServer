# Webserver

Ein in Rust gebauter, Caddy-inspirierter Webserver und Reverse Proxy. Das Ziel ist ein einzelnes, produktionsnahes Binary mit einfachen Defaults und einer unkomplizierten CLI, ohne die Flexibilitaet klassischer Webserver aufzugeben.

Das Projekt befindet sich noch ganz am Anfang. Der aktuelle Stand enthaelt bewusst nur das Rust-Grundgeruest; die nachfolgende Roadmap beschreibt den geplanten Zielumfang.

## Zielbild

```text
Internet / Clients
        |
        | HTTP und HTTPS
        v
    Webserver
    |- Listener und Protokollschicht
    |- Host- und Pfad-Routing
    |- Static-File-Handler
    |- Reverse Proxy und Load Balancer
    |- TLS- und Zertifikatsverwaltung
    |- Sicherheits-, Logging- und Metrikschicht
    `- CLI und Konfigurationsverwaltung
        |
        v
    Upstreams: Anwendungen, Dienste, Container
```

## Geplanter Funktionsumfang

### HTTP und Routing

- HTTP/1.1 mit Keep-Alive und sauberem Request-/Response-Handling
- Virtuelle Hosts anhand des `Host`-Headers
- Pfadbasiertes Routing; die spezifischste passende Route gewinnt
- Methoden-, Header- und Query-Parameter-Verarbeitung
- Konfigurierbare Redirects und Header-Regeln
- WebSocket- und Streaming-Proxying

### Static Files

- Auslieferung statischer Dateien pro Site oder Route
- MIME-Type-Erkennung, Index-Dateien und konfigurierbare Fehlerseiten
- Sichere Pfadauflosung mit Schutz vor Path Traversal
- Conditional Requests und Cache-Header
- Kompression mit gzip und Brotli

### Reverse Proxy und Upstreams

- Proxying zu HTTP-Upstreams
- Korrekte `Host`- sowie `X-Forwarded-*`-Header
- Verbindungs-Pooling, Timeouts und konfigurierbare Retry-Regeln
- Aussagekraeftige Fehlerantworten, insbesondere `502 Bad Gateway` und `504 Gateway Timeout`
- Mehrere Upstreams pro Route
- Load-Balancing-Strategien: Round Robin, gewichtetes Round Robin und Least Connections
- Aktive und passive Health Checks sowie Circuit Breaking
- Spaeter: Discovery ueber DNS, Docker und weitere Infrastrukturquellen

### TLS und Protokolle

- HTTPS mit sicherer Standardkonfiguration
- ACME-Integration fuer automatische Let's-Encrypt-Zertifikate und Erneuerung
- SNI und mehrere TLS-Sites auf einem Listener
- HTTP/2
- Spaeter: HTTP/3 ueber QUIC

### Bedienung und Konfiguration

- Ein einzelnes Rust-Binary fuer Betrieb und Verwaltung
- CLI als primaerer Einstieg, etwa zum Anlegen von Sites und Routen
- Interaktiver CLI-Modus fuer einfache Einrichtung
- Menschenlesbare, manuell editierbare Konfiguration
- Eine gemeinsam genutzte Management-Schicht: Die CLI verwaltet Konfiguration und Runtime; sie bereitet damit eine spaetere Admin-API vor
- Konfigurationspruefung vor dem Start oder Reload
- Reload ohne unnötige Unterbrechung laufender Verbindungen

Ein konkretes Dateiformat wird erst festgelegt, wenn das Konfigurationsmodell steht. TOML ist aktuell der bevorzugte Kandidat.

### Betrieb, Sicherheit und Beobachtbarkeit

- Strukturierte Access- und Error-Logs
- Konfigurierbare Log-Level und Log-Ausgaben
- Graceful Shutdown
- Rate Limits, Request- und Body-Groessenlimits
- IP-Regeln, sichere Standard-Header und CORS-Regeln
- Prometheus-kompatible Metriken
- Spaeter: OpenTelemetry-Tracing, Caching und optionale WAF-Regeln
- Tests fuer Parser, Routing, Proxy-Verhalten, Konfiguration und Sicherheitsgrenzen

## Nicht Teil des ersten Ziels

Eine browserbasierte Verwaltungsoberflaeche ist als moegliche spaetere Erweiterung vorgemerkt, wird aber nicht im aktuellen Umfang umgesetzt. Wenn sie entsteht, soll sie dieselbe Management-Schicht wie die CLI verwenden und standardmaessig nur lokal sowie abgesichert erreichbar sein.

## Entwicklung

Voraussetzung ist eine aktuelle Rust-Toolchain.

```powershell
cargo run -- init
cargo run -- check
cargo run -- run
cargo test
cargo clippy -- -D warnings
```

`init` erzeugt eine globale `webserver.toml`, eine erste Site unter `sites/localhost.conf` sowie einen passenden `public/index.html`-Startpunkt. `check` validiert die Datei, ohne einen Port zu belegen. `run` startet den Server auf der in der Konfiguration gesetzten Adresse.

Die CLI kann Sites und Routen auch direkt verwalten:

```powershell
cargo run -- site-add --host example.test
cargo run -- route-add --host example.test --path / --static ./public
cargo run -- route-add --host example.test --path /api --upstream http://127.0.0.1:3000
cargo run -- route-remove --host example.test --path /api
```

## Konfiguration

Die v0.1-Konfiguration liegt im TOML-Format vor und ist absichtlich wie nginx in globale und Site-spezifische Dateien getrennt. Jede reguläre Datei unter `sites/` wird als eine Site geladen; die Dateiendung ist frei waehbar, `.conf` ist die empfohlene Konvention.

```toml
# /etc/webserver/webserver.toml
[server]
bind = "0.0.0.0:80"
upstream_timeout_secs = 30
max_header_bytes = 32768
max_body_bytes = 10485760

[tls]
enabled = true
bind = "0.0.0.0:443"
email = "admin@example.com"
certificate_cache = "/etc/webserver/certificates/acme"
```

```toml
# /etc/webserver/sites/example.conf
host = "example.com"

[[routes]]
path_prefix = "/"
kind = "static"
root = "/var/www/webserver/example"

[[routes]]
path_prefix = "/api"
kind = "proxy"
upstream = "http://127.0.0.1:3000"
```

Eine Site-Datei besitzt einen Host und ihre Routen. Eine Route besitzt entweder `kind = "static"` mit `root` (und optional `index_file`) oder `kind = "proxy"` mit einem vollständigen HTTP-Upstream. Bei mehreren Treffern gewinnt die Route mit dem längsten passenden Pfadpraefix. Relative Static-Roots werden relativ zur jeweiligen Site-Datei aufgeloest.

Eine Proxy-Route kann auch mehrere Ziele besitzen. Die Ziele werden global per Round Robin ausgewählt; das bisherige einzelne Feld `upstream` bleibt weiterhin gültig.

```toml
[[routes]]
path_prefix = "/api"
kind = "proxy"
upstreams = [
  "http://127.0.0.1:3000",
  "http://127.0.0.1:3001",
]
```

Für Produktionsrouten sind Gewichtungen und Resilienzregeln konfigurierbar:

```toml
[[routes]]
path_prefix = "/api"
kind = "proxy"
load_balancing = "least_connections" # round_robin | weighted_round_robin | least_connections
upstreams = [
  { url = "http://127.0.0.1:3000", weight = 3 },
  { url = "http://127.0.0.1:3001", weight = 1 },
]
retries = 2
retry_backoff_ms = 100
max_connections_per_upstream = 100
base_path = "/v1"
rewrite_prefix = "/internal"

[routes.health_check]
path = "/health"
interval_secs = 10
timeout_secs = 3
```

Aktive HTTP-Health-Checks erwarten einen 2xx-Status. Passive Fehler öffnen nach drei aufeinanderfolgenden Verbindungs- oder Timeout-Fehlern für 30 Sekunden einen Circuit Breaker. Normale Proxy-Requests werden bis zum bestehenden Request-Limit gepuffert und können deshalb mit exponentiellem Backoff wiederholt werden. WebSocket-Upgrades werden als bidirektionaler Stream durchgereicht.

Sobald `tls.enabled = true` gesetzt ist, startet der Server zusätzlich auf Port 443. Für jede konfigurierte Site ohne lokales Zertifikat fordert er automatisch ein Let's-Encrypt-Zertifikat per HTTP-01 an, speichert es im `certificate_cache` und erneuert es automatisch. Port 80 bleibt dabei erforderlich: Nur `/.well-known/acme-challenge/...` wird dort für Let's Encrypt ausgeliefert, alle übrigen HTTP-Anfragen werden mit einem permanenten Redirect auf HTTPS weitergeleitet. DNS der jeweiligen Hosts muss vorher auf den Server zeigen und beide Ports müssen von außen erreichbar sein. Änderungen an TLS-Einstellungen oder Hostnamen werden erst nach einem Dienstneustart übernommen; Routen können weiter per Reload geändert werden.

### DNS-01 und Wildcard-Zertifikate

Für Wildcards oder Hosts ohne öffentlich erreichbaren Port 80 kann DNS-01 verwendet werden. Der Server integriert dafür den DNS-Provider-Client [lego](https://go-acme.github.io/lego/dns/); dessen Provider (etwa Cloudflare, Route 53 und DigitalOcean) und die CNAME-/NS-Delegation von `_acme-challenge` werden direkt unterstützt. `lego` muss installiert sein; Zugangsdaten bleiben in einer geschützten `KEY=VALUE`-Datei und werden nur für den gestarteten Provider-Prozess als Umgebungsvariablen gesetzt.

```toml
[tls.dns_challenge]
command = "/usr/bin/lego" # optional; Standard ist "lego"

[[tls.dns_challenge.providers]]
provider = "cloudflare"
domains = ["example.com", "*.example.com"]
credentials_file = "/etc/webserver/dns/cloudflare.env"
# Optional: Resolver, die bei delegierten CNAME-/NS-Zonen abgefragt werden.
resolvers = ["1.1.1.1:53", "8.8.8.8:53"]
```

Für Cloudflare enthält die Credentials-Datei beispielsweise `CLOUDFLARE_DNS_API_TOKEN=...`. Sie muss dem Dienst lesbar sein und darf nicht für andere lesbar oder für Gruppe/andere schreibbar sein (unter Linux in der Regel `0600`). Beim ersten Start fordert der Server das Zertifikat an; bei weiteren Starts führt er sicher `lego renew` aus. Für eine vollständig automatische Erneuerung ohne Neustart sollte der Dienst regelmäßig neu gestartet werden, beispielsweise über einen systemd-Timer.

### Lokale Zertifikate und eigene CA

Lokale Zertifikate werden als PEM-Kette und PEM-Private-Key konfiguriert und per SNI dem jeweiligen Host zugeordnet. Das funktioniert genauso mit Zertifikaten einer eigenen internen CA; die vollständige Kette gehört in `certificate`.

```toml
[[tls.certificates]]
hosts = ["internal.example.com"]
certificate = "/etc/webserver/certificates/local/internal.example.com.fullchain.pem"
private_key = "/etc/webserver/certificates/local/internal.example.com.key.pem"
```

Lokale Zertifikate gewinnen gegenüber ACME. Bestehen alle TLS-Sites aus lokalen Zertifikaten, ist keine `tls.email` erforderlich. Der Linux-Installer trennt `/etc/webserver/certificates/acme` (schreibbar für automatische Erneuerungen) von `/etc/webserver/certificates/local` (nur lesbar für den Dienst). Private Keys müssen reguläre Dateien sein, dürfen weder für andere lesbar noch für Gruppe/andere schreibbar sein. Empfohlene Installation:

```sh
sudo install -m 640 -o root -g www-data fullchain.pem /etc/webserver/certificates/local/internal.example.com.fullchain.pem
sudo install -m 640 -o root -g www-data privkey.pem /etc/webserver/certificates/local/internal.example.com.key.pem
```

## Fehlerseiten

Die Standardfehlerseiten fuer `400`, `403`, `404`, `405`, `411`, `413`, `431`, `500`, `502`, `503` und `504` liegen unter [assets/error-pages](assets/error-pages) und werden beim Build direkt in das Binary eingebettet. Sie funktionieren daher auch ohne zusaetzliche Dateien auf dem Server. Eigene konfigurierbare Fehlerseiten folgen spaeter.

## Linux-Installation und systemd

Ein Release-Binary wird mit folgendem Befehl erstellt:

```sh
cargo build --locked --release
sudo packaging/install.sh ./target/release/webserver
```

Das Installationsskript verwendet den etablierten Systembenutzer und die Gruppe `www-data` (wie nginx und Apache auf Debian/Ubuntu), installiert das Binary nach `/usr/local/bin/webserver`, erzeugt die Standardverzeichnisse, installiert die systemd-Unit und aktiviert den Dienst. Falls `www-data` fehlt, wird das Systemkonto angelegt. Die produktive Konfiguration liegt unter `/etc/webserver/webserver.toml`; jede Site liegt unter `/etc/webserver/sites/*.conf`. Statische Dateien liegen standardmaessig in `/var/www/webserver/public`.

`/usr/local/bin` liegt auf gängigen Linux-Systemen bereits im globalen `PATH`; deshalb ist kein Eintrag in `.bashrc` oder eine Shell-Alias-Datei erforderlich. Falls das Paket `bash-completion` installiert ist, richtet der Installer ausserdem Tab-Vervollständigung für `webserver` ein. Sie kann auch manuell erzeugt werden:

```sh
webserver completion bash > /usr/share/bash-completion/completions/webserver
```

```sh
sudo systemctl status webserver
sudo systemctl reload webserver  # laedt die Konfiguration neu
sudo journalctl -u webserver -f
```

Auf Linux verarbeitet der Prozess `SIGHUP` als Konfigurationsreload. Bei ungültiger Konfiguration wird der Reload abgelehnt und die bisherige, funktionierende Konfiguration weiterverwendet. TLS-Einstellungen und die Liste der TLS-Hostnamen benötigen dagegen einen Neustart, damit Listener und ACME-Verwaltung konsistent neu aufgebaut werden. Der Installer reserviert Port 80 und 443 über `CAP_NET_BIND_SERVICE` für den Dienstbenutzer `www-data` und gibt diesem nur auf den Zertifikatsspeicher Schreibzugriff.

## Windows Server

Windows Server wird nativ unterstuetzt. Das Binary kann normal in PowerShell gestartet werden oder als echter Windows-Service laufen. Der Installer legt die folgenden Orte an:

```text
C:\Program Files\Webserver\webserver.exe
C:\ProgramData\Webserver\webserver.toml
C:\ProgramData\Webserver\sites\*.conf
C:\inetpub\wwwroot\Webserver\public
```

In einer **als Administrator** gestarteten PowerShell:

```powershell
cargo build --locked --release
.\packaging\windows\install.ps1 -BinaryPath .\target\release\webserver.exe
Get-Service Webserver
```

Der Dienst läuft als `NT AUTHORITY\LOCAL SERVICE`; der Installer setzt auf Konfiguration und statische Inhalte die erforderlichen Leserechte sowie Schreibrechte ausschließlich auf `C:\ProgramData\Webserver\certificates`. Der Dienst kann wie jeder Windows-Service verwaltet werden:

```powershell
Restart-Service Webserver
Stop-Service Webserver
Get-Service Webserver
```

## Docker-APT-Repository

Der APT-Repository-Container wird mit `apt-repo/` als eigenständigem Docker-Build-Kontext gebaut. Die Build-Stage klont den konfigurierten Quell-Branch und erstellt daraus Binary und Debian-Paket; das finale Image enthält ausschließlich nginx und das erzeugte APT-Repository für `amd64` auf Port `8080`.

```sh
cd apt-repo
docker build -t webserver-apt-repo .
docker run --rm -p 8080:8080 webserver-apt-repo
```

Auf einem Debian-/Ubuntu-Client wird es einmalig als vertrauenswürdige Quelle eingebunden (für einen öffentlichen Betrieb sollte die URL per TLS ausgeliefert und das Repository zusätzlich GPG-signiert werden):

```sh
echo 'deb [trusted=yes] http://REPOSITORY-HOST:8080 stable main' | sudo tee /etc/apt/sources.list.d/webserver.list
sudo apt update
sudo apt install webserver
```

## Releases

Der Workflow [release.yml](.github/workflows/release.yml) baut bei einem Git-Tag wie `v0.1.0` ein Linux-x86_64-Release-Archiv mit Binary, Installer und systemd-Unit. Ein eigentlicher Paket-Repository-Feed (APT/RPM) ist noch nicht eingerichtet; das Archiv ist der sichere, einfache erste Distributionsweg.
