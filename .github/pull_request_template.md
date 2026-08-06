# Pull Request

## Zusammenfassung

<!-- Was ändert sich und warum? -->

## Prüfung

- [ ] Relevante Tests ergänzt oder aktualisiert
- [ ] `cargo fmt --check` ausgeführt
- [ ] `cargo clippy --locked --all-targets -- -D warnings` ausgeführt
- [ ] `cargo test --locked --all-targets` ausgeführt
- [ ] Dashboard-Checks ausgeführt, falls betroffen
- [ ] Dokumentation aktualisiert, falls CLI, Konfiguration oder API betroffen

## Sicherheit

- [ ] Keine Secrets, Tokens, privaten Schlüssel oder produktiven Daten enthalten
- [ ] Auswirkungen auf Authentifizierung, TLS, Eingabevalidierung und
      Abhängigkeiten geprüft
