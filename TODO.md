# Webserver roadmap

Dieses Dokument ist der Arbeitsplan für den Server. Die erste HTTP/1.1-Version wird unmittelbar umgesetzt; die folgenden Punkte bleiben bewusst als spätere Meilensteine sichtbar.

## v0.1 — HTTP-Server und Proxy

- [x] HTTP/1.1-Listener, Keep-Alive und virtuelle Hosts
- [x] Pfad-Routing mit spezifischster passender Route
- [x] Static Files, MIME-Types, Index-Dateien sowie Schutz gegen Traversal und Symlink-Ausbrüche
- [x] HTTP-Reverse-Proxy, Connection Reuse, Proxy-Header, Timeouts sowie `502`/`504`
- [x] TOML-Konfiguration mit `init`, `check`, `run` sowie Site-/Route-Verwaltung über die CLI
- [x] Konfigurationsreload per `SIGHUP` auf Linux; der systemd-Service nutzt diesen Weg bei `systemctl reload`
- [x] Header- und Request-Body-Limits; Chunked Request Bodies sind bis zu einer späteren Streaming-Limitierung bewusst abgelehnt
- [x] systemd-Unit, Release-Build und Installationsskript
- [x] End-to-End-Tests für CLI-Verwaltung, Static Files und Proxying
- [x] Weitere Regressionstests für Limits, Fehlerfälle und Reload
- [x] Konfigurationsschreiben mit kommentar-erhaltendem, atomarem Editor
- [x] Vollständig graceful Shutdown: neue Verbindungen stoppen, bestehende Requests fertig bedienen

## v0.2 — HTTPS und Zertifikate

- [x] TLS-Listener auf Port 443 mit sicheren Standard-Cipher-Suites und Protokollversionen
- [x] HTTP-zu-HTTPS-Redirects
- [x] SNI und mehrere TLS-Sites auf einem Listener
- [x] ACME-Client für Let's Encrypt
- [x] HTTP-01-Challenge und automatisierte Zertifikatserneuerung
- [x] DNS-01-Challenge mit DNS-Provider-Integrationen, CNAME/NS-Delegation und Wildcard-Zertifikaten
- [x] Lokale Zertifikate und eigene Certificate Authorities unterstützen
- [x] Sichere Ablage und Rechte für private Schlüssel

## v0.3 — Erweiterter Proxy

- [x] Mehrere Upstreams je Route
- [x] Round Robin, gewichtetes Round Robin und Least Connections
- [x] Passive und aktive Health Checks
- [x] Retry-Regeln, Backoff und Circuit Breaking
- [x] WebSocket-Upgrade und bidirektionales Streaming
- [x] Upstream-Basis-Pfade und Rewrite-Regeln
- [x] Verbindungslimits pro Upstream

## v0.4 — HTTP-Features und Static Serving

- [x] Streaming von grossen statischen Dateien statt vollständigem Einlesen
- [x] Range Requests
- [x] ETag, Last-Modified und Conditional Requests
- [x] Konfigurierbare Fehlerseiten
- [x] Header-Manipulation, Redirects und URL-Rewrites
- [x] gzip und Brotli
- [x] Caching-Regeln und optionaler Response-Cache

## v0.5 — Betrieb und Sicherheit

- [x] Strukturierte JSON-Logs und Log-Rotation
- [x] Prometheus-Metriken
- [x] OpenTelemetry-Tracing
- [x] Rate Limiting und Concurrent-Connection-Limits
- [x] IP-Allow-/Deny-Listen und vertrauenswürdige Proxy-Netze
- [x] CORS- und Security-Header-Regeln
- [x] Privilege Dropping, Linux-Capabilities und weitere systemd-Hardening-Optionen
- [x] Fuzzing des HTTP- und Konfigurationspfads

## v0.6 — Moderne Protokolle und Infrastruktur

- [x] HTTP/2
- [x] HTTP/3 / QUIC
- [x] DNS-basierte Upstream-Discovery
- [x] Docker-Discovery
- [x] Kubernetes-/Service-Discovery als optionales Modul
- [x] Zero-Downtime-Binary-Upgrades

## v0.7 — Lokale Verwaltungs-API

- [x] Lokale, authentifizierte Admin-API
- [x] API-Endpunkte für Sites, Routen, Upstreams, Zertifikate, Logs und Metriken
- [x] Sichere lokale Standardbindung; keine externe Freigabe ohne explizite Konfiguration
- [x] API-Tests und OpenAPI-Dokumentation

## v0.8 — Einheitliche Verwaltung und Zugriffsschutz

- [x] Dieselbe Management-Schicht für CLI und API
- [x] Rollen und Authentifizierung für nicht-lokale API-Freigaben
- [x] Audit-Log für jede schreibende Verwaltungsaktion
- [x] Migrations- und Kompatibilitätsregeln für die Verwaltungs-API

## v0.9 — Web-Dashboard

- [x] Web-Dashboard für Sites, Routen, Upstreams, Zertifikate, Logs und Metriken
- [x] Dashboard-Anmeldung über die Verwaltungs-API und Rollenmodell
- [x] Betriebsansichten für Health Checks, Tracing und Prometheus-Metriken

## v1.0 — Stabile Server- und Verwaltungsoberfläche

- [x] Stabile, dokumentierte Konfigurations-, CLI- und API-Verträge
- [x] Upgrade- und Migrationsleitfaden für produktive Installationen
- [x] Vollständige End-to-End-, Sicherheits- und Release-Regressionstests
