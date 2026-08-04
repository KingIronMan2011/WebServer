# Webserver roadmap

Dieses Dokument ist der Arbeitsplan fuer den Server. Die erste HTTP/1.1-Version wird unmittelbar umgesetzt; die folgenden Punkte bleiben bewusst als spaetere Meilensteine sichtbar.

## v0.1 — HTTP-Server und Proxy

- [x] HTTP/1.1-Listener, Keep-Alive und virtuelle Hosts
- [x] Pfad-Routing mit spezifischster passender Route
- [x] Static Files, MIME-Types, Index-Dateien sowie Schutz gegen Traversal und Symlink-Ausbrueche
- [x] HTTP-Reverse-Proxy, Connection Reuse, Proxy-Header, Timeouts sowie `502`/`504`
- [x] TOML-Konfiguration mit `init`, `check`, `run` sowie Site-/Route-Verwaltung ueber die CLI
- [x] Konfigurationsreload per `SIGHUP` auf Linux; der systemd-Service nutzt diesen Weg bei `systemctl reload`
- [x] Header- und Request-Body-Limits; Chunked Request Bodies sind bis zu einer spaeteren Streaming-Limitierung bewusst abgelehnt
- [x] systemd-Unit, Release-Build und Installationsskript
- [x] End-to-End-Tests fuer CLI-Verwaltung, Static Files und Proxying
- [ ] Weitere Regressionstests fuer Limits, Fehlerfaelle und Reload
- [ ] Konfigurationsschreiben mit kommentar-erhaltendem, atomarem Editor
- [ ] Vollstaendig graceful Shutdown: neue Verbindungen stoppen, bestehende Requests fertig bedienen

## v0.2 — HTTPS und Zertifikate

- [ ] TLS-Listener auf Port 443 mit sicheren Standard-Cipher-Suites und Protokollversionen
- [ ] HTTP-zu-HTTPS-Redirects
- [ ] SNI und mehrere TLS-Sites auf einem Listener
- [ ] ACME-Client fuer Let's Encrypt
- [ ] HTTP-01-Challenge und automatisierte Zertifikatserneuerung
- [ ] Lokale Zertifikate und eigene Certificate Authorities unterstuetzen
- [ ] Sichere Ablage und Rechte fuer private Schluessel

## v0.3 — Erweiterter Proxy

- [ ] Mehrere Upstreams je Route
- [ ] Round Robin, gewichtetes Round Robin und Least Connections
- [ ] Passive und aktive Health Checks
- [ ] Retry-Regeln, Backoff und Circuit Breaking
- [ ] WebSocket-Upgrade und bidirektionales Streaming
- [ ] Upstream-Basis-Pfade und Rewrite-Regeln
- [ ] Verbindungslimits pro Upstream

## v0.4 — HTTP-Features und Static Serving

- [ ] Streaming von grossen statischen Dateien statt vollständigem Einlesen
- [ ] Range Requests
- [ ] ETag, Last-Modified und Conditional Requests
- [ ] Konfigurierbare Fehlerseiten
- [ ] Header-Manipulation, Redirects und URL-Rewrites
- [ ] gzip und Brotli
- [ ] Caching-Regeln und optionaler Response-Cache

## v0.5 — Betrieb und Sicherheit

- [ ] Strukturierte JSON-Logs und Log-Rotation
- [ ] Prometheus-Metriken
- [ ] OpenTelemetry-Tracing
- [ ] Rate Limiting und Concurrent-Connection-Limits
- [ ] IP-Allow-/Deny-Listen und vertrauenswuerdige Proxy-Netze
- [ ] CORS- und Security-Header-Regeln
- [ ] Privilege Dropping, Linux-Capabilities und weitere systemd-Hardening-Optionen
- [ ] Fuzzing des HTTP- und Konfigurationspfads

## v0.6 — Moderne Protokolle und Infrastruktur

- [ ] HTTP/2
- [ ] HTTP/3 / QUIC
- [ ] DNS-basierte Upstream-Discovery
- [ ] Docker-Discovery
- [ ] Kubernetes-/Service-Discovery als optionales Modul
- [ ] Zero-Downtime-Binary-Upgrades

## Langfristig — Management-Oberflaeche

- [ ] Lokale, authentifizierte Admin-API
- [ ] Dieselbe Management-Schicht fuer CLI und API
- [ ] Web-Dashboard fuer Sites, Routen, Upstreams, Zertifikate, Logs und Metriken
- [ ] Rollen, Authentifizierung und Audit-Log vor jeder nicht-lokalen Freigabe
