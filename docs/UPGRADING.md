# Betrieb, Kompatibilität und Upgrades

Dieses Dokument beschreibt den stabilen v1-Vertrag sowie sichere Upgrade- und
Rollback-Schritte für produktive Installationen.

## Stabile v1-Verträge

### Konfiguration

- Die globale TOML-Datei bleibt unter `/etc/webserver/webserver.toml`; einzelne
  virtuelle Hosts liegen unter `/etc/webserver/sites/*.conf`.
- Bereits dokumentierte Felder und ihre Bedeutung bleiben in v1 erhalten.
  Erweiterungen sind optional und nutzen sichere Standardwerte.
- Vor dem Anwenden wird jede Konfiguration mit
  `webserver check --config /etc/webserver/webserver.toml` validiert.
- Ein Reload übernimmt gültige Änderungen an Routen und Servereinstellungen.
  Änderungen an TLS-Listenern oder Zertifikaten erfordern einen Neustart.

### CLI

Die stabilen Verwaltungsbefehle sind `init`, `check`, `run`, `site-add`,
`site-remove`, `route-add`, `route-remove`, `admin web-init` und `completion`.
Die in `webserver --help` dokumentierten Optionen sind Teil des v1-Vertrags.
Skripte sollen stets `--config` mit einem absoluten Pfad verwenden.

### Verwaltungs-API

- Der stabile API-Präfix ist `/api/v1`.
- Die aktuelle Beschreibung steht unter `/api/v1/openapi.json`.
- Jede Antwort trägt `X-Webserver-Api-Version: 1`.
- Nicht-brechende Ergänzungen sind innerhalb von v1 möglich. Entfernte oder
  umbenannte Felder/Endpunkte sowie geänderte Semantik erscheinen nur unter
  einer neuen API-Hauptversion, etwa `/api/v2`.
- Schreibende Endpunkte verlangen `admin` oder `operator`; Benutzerverwaltung
  verlangt `admin`. Audit-Logs erfassen jede erfolgreiche Schreiboperation.

## Upgrade unter Debian/Ubuntu

1. Wartungsfenster planen und Version notieren:

   ```sh
   webserver --version
   sudo systemctl status webserver
   ```

2. Konfiguration, Site-Dateien, Zertifikate und Verwaltungsdatenbank sichern:

   ```sh
   sudo install -d -m 0700 /root/webserver-backup
   sudo cp -a /etc/webserver /root/webserver-backup/etc-webserver
   sudo cp -a /var/lib/webserver /root/webserver-backup/var-lib-webserver 2>/dev/null || true
   sudo cp -a /var/www/webserver /root/webserver-backup/var-www-webserver
   ```

   Der Datenbankpfad kann in `[admin].database` abweichen; diesen Pfad ebenfalls
   sichern. SQLite-Dateien dürfen nur bei gestopptem Dienst kopiert werden oder
   mit `sqlite3 DATABASE '.backup BACKUP.sqlite'` konsistent gesichert werden.

3. Paket aktualisieren und die Konfiguration vor dem Neustart prüfen:

   ```sh
   sudo apt update
   sudo apt install webserver
   sudo webserver check --config /etc/webserver/webserver.toml
   ```

4. Dienst neu starten, Health und Management-API prüfen:

   ```sh
   sudo systemctl restart webserver
   sudo systemctl --no-pager --full status webserver
   curl --fail --silent --show-error https://ADMIN_HOST:9080/api/v1/health
   ```

5. Dashboard- und Audit-Anmeldung testen. Nach dem ersten `admin web-init`
   muss das generierte Passwort unmittelbar geändert werden; der Setup-Code ist
   nur 15 Minuten gültig.

## Upgrade einer lokalen Binary

Zuerst den Release-Tarball prüfen, dann eine neue Binary atomar installieren:

```sh
tar -xzf webserver-linux-x86_64.tar.gz
./webserver/webserver check --config /etc/webserver/webserver.toml
sudo install -m 0755 ./webserver/webserver /usr/local/bin/webserver.new
sudo mv -f /usr/local/bin/webserver.new /usr/local/bin/webserver
sudo systemctl restart webserver
```

Die Release-Artefakte enthalten außerdem `dashboard/` mit dem vorgebauten
Frontend. Es wird ausschließlich aus dem versionierten `pnpm-lock.yaml`
erzeugt.

## Migration und Rollback

Verwaltungsdatenbank-Migrationen werden vor Nutzung atomar in SQLite erfasst.
Eine ältere Binary darf keine Datenbank mit neuerem Schema verändern.

Bei einem fehlgeschlagenen Upgrade:

1. Dienst stoppen: `sudo systemctl stop webserver`.
2. Vorherige, geprüfte Binary bzw. Paketversion wiederherstellen.
3. Konfiguration und die gesicherte Verwaltungsdatenbank zurückspielen.
4. Konfiguration prüfen und Dienst starten.
5. Health, eine produktive Route und die Admin-Anmeldung testen.

Ein Paket-Downgrade ohne Wiederherstellung eines dazu passenden
Datenbank-Backups ist nicht unterstützt.
