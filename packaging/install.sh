#!/usr/bin/env sh
set -eu

binary="${1:-./target/release/webserver}"
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)

if [ "$(id -u)" -ne 0 ]; then
  echo "Run this installer as root." >&2
  exit 1
fi

if [ ! -x "$binary" ]; then
  echo "Executable not found: $binary" >&2
  echo "Build it first with: cargo build --release" >&2
  exit 1
fi

if ! command -v systemctl >/dev/null 2>&1; then
  echo "This installer currently supports systemd-based Linux systems only." >&2
  exit 1
fi

if ! getent group www-data >/dev/null 2>&1; then
  groupadd --system www-data
fi
if ! getent passwd www-data >/dev/null 2>&1; then
  useradd --system --gid www-data --home-dir /var/www --shell /usr/sbin/nologin www-data
fi

install -Dm755 "$binary" /usr/local/bin/webserver
install -d -m750 -o root -g www-data /etc/webserver/sites
install -d -m750 -o www-data -g www-data /var/www/webserver /var/www/webserver/public /var/log/webserver

if [ ! -f /etc/webserver/webserver.toml ]; then
  install -Dm640 -o root -g www-data "$script_dir/webserver.toml" /etc/webserver/webserver.toml
fi
if [ ! -f /etc/webserver/sites/localhost.conf ]; then
  install -Dm640 -o root -g www-data "$script_dir/sites/localhost.conf" /etc/webserver/sites/localhost.conf
fi
if [ ! -f /var/www/webserver/public/index.html ]; then
  printf '%s\n' '<!doctype html><title>Webserver</title><h1>It works!</h1>' > /var/www/webserver/public/index.html
  chown www-data:www-data /var/www/webserver/public/index.html
fi
install -Dm644 "$script_dir/systemd/webserver.service" /etc/systemd/system/webserver.service

if [ -d /usr/share/bash-completion/completions ]; then
  /usr/local/bin/webserver completion bash > /usr/share/bash-completion/completions/webserver
fi

systemctl daemon-reload
systemctl enable --now webserver.service
systemctl --no-pager status webserver.service
