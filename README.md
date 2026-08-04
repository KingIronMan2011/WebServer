# Webserver

Ein in Rust gebauter, Caddy-inspirierter Webserver und Reverse Proxy. Das Ziel ist ein einzelnes, produktionsnahes Binary mit einfachen Defaults und einer unkomplizierten CLI, ohne die Flexibilitaet klassischer Webserver aufzugeben.

Das Projekt befindet sich noch ganz am Anfang. Der aktuelle Stand enthaelt bewusst nur das Rust-Grundgeruest; die nachfolgende Roadmap beschreibt den geplanten Zielumfang.

## Zielbild

```text
Internet / Clients
        |
        | HTTP, spaeter HTTPS
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
cargo run
cargo test
cargo clippy -- -D warnings
```

Der Server implementiert noch keine Funktionalitaet. Der naechste Entwicklungsschritt ist die Projektstruktur und anschliessend ein minimaler HTTP/1.1-Listener.

