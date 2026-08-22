#!/bin/sh
set -eu

if ! getent group webserver >/dev/null 2>&1; then groupadd --system webserver; fi
if ! getent passwd webserver >/dev/null 2>&1; then useradd --system --gid webserver --home-dir /var/lib/webserver --shell /usr/sbin/nologin webserver; fi

install -d -m 0750 -o webserver -g webserver /etc/webserver/sites /etc/webserver/certificates/acme /var/www/webserver/public /var/log/webserver /var/lib/webserver
install -d -m 0750 -o root -g webserver /etc/webserver/certificates/local

if [ ! -f /etc/webserver/webserver.toml ]; then install -m 0640 -o root -g webserver /usr/share/webserver/webserver.toml /etc/webserver/webserver.toml; fi
if [ ! -f /etc/webserver/sites/localhost.conf ]; then install -m 0640 -o webserver -g webserver /usr/share/webserver/localhost.conf /etc/webserver/sites/localhost.conf; fi
if [ ! -f /var/lib/webserver/admin.db ]; then install -m 0600 -o webserver -g webserver /dev/null /var/lib/webserver/admin.db; fi
if [ ! -f /var/www/webserver/public/index.html ]; then printf '%s\n' '<!doctype html><title>Webserver</title><h1>It works!</h1>' > /var/www/webserver/public/index.html; chown webserver:webserver /var/www/webserver/public/index.html; fi

if command -v systemctl >/dev/null 2>&1; then
  systemctl daemon-reload || true
  systemctl enable --now webserver.service || true
fi
