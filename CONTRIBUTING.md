# Beitragen

Danke für dein Interesse an Webserver. Für Fehler, Verbesserungen,
Dokumentation und Tests sind Beiträge willkommen.

## Vor dem Pull Request

1. Prüfe, ob bereits ein Issue oder Pull Request existiert.
2. Diskutiere größere Änderungen zuerst in einem Issue.
3. Melde Sicherheitslücken ausschließlich nach der
   [Sicherheitsrichtlinie](SECURITY.md).

## Lokale Prüfung

Für Rust:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
```

Für das Dashboard:

```sh
cd dashboard
pnpm install --frozen-lockfile
pnpm lint
pnpm format:check
pnpm types
pnpm build
```

Wenn du Konfigurations-, Routing-, Authentifizierungs- oder Sicherheitslogik
änderst, ergänze bitte einen Regressionstest. Aktualisiere die Dokumentation,
wenn sich CLI, Konfiguration oder die Management-API verändert.

## Pull Requests

- Halte Änderungen fokussiert und erkläre das Warum.
- Verwende keine Secrets, produktiven Zugangsdaten oder privaten Zertifikate.
- Aktualisiere Lockfiles nur zusammen mit der zugehörigen
  Abhängigkeitsänderung.
- Lass die Checks aus der Pull-Request-Vorlage vor dem Review laufen.
