FROM debian:bookworm@sha256:813017f3d62be4b5891a7acca6a01bdcd4b8513daa81b1ab99d3a50385b26931 AS source

ARG SOURCE_REPOSITORY=https://github.com/KingIronMan2011/WebServer.git
ARG SOURCE_REF=main

RUN apt-get update && apt-get install --no-install-recommends -y ca-certificates git && rm -rf /var/lib/apt/lists/*
WORKDIR /src
RUN --mount=type=secret,id=github_token,required=false \
    set -eu; \
    if [ -s /run/secrets/github_token ]; then \
      token="$(cat /run/secrets/github_token)"; \
      auth="$(printf 'x-access-token:%s' "$token" | base64 -w 0)"; \
      git -c "http.extraheader=Authorization: Basic $auth" clone --depth 1 --branch "$SOURCE_REF" "$SOURCE_REPOSITORY" .; \
    else \
      git clone --depth 1 --branch "$SOURCE_REF" "$SOURCE_REPOSITORY" .; \
    fi

FROM node:26-bookworm@sha256:93f972701f557d320918895e41eeda4821e4beb495f45f53282a22ca4c3f0813 AS dashboard

COPY --from=source /src/dashboard /dashboard
WORKDIR /dashboard
RUN npm install --global npm@12.0.2 pnpm@11.20.0 \
    && npm cache clean --force \
    && rm -rf /usr/local/lib/node_modules/npm /usr/local/bin/npm /usr/local/bin/npx \
    && pnpm install --frozen-lockfile \
    && pnpm build

FROM rust:bookworm@sha256:14bc9c5966e7b3a385794b3d5389a8765668342025fbcc7b2e3d2866ac4bd8c3 AS build

COPY --from=source /src /src
COPY debian/postinst /tmp/postinst
WORKDIR /src
RUN cargo build --locked --release
COPY --from=dashboard /dashboard/dist /dashboard/dist

RUN set -eux; \
    version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"; \
    install -d /package/DEBIAN /package/usr/local/bin /package/lib/systemd/system /package/usr/share/doc/webserver /package/usr/share/webserver/dashboard; \
    install -m 0755 target/release/webserver /package/usr/local/bin/webserver; \
    install -m 0644 packaging/systemd/webserver.service /package/lib/systemd/system/webserver.service; \
    install -m 0755 /tmp/postinst /package/DEBIAN/postinst; \
    install -m 0644 packaging/webserver.toml /package/usr/share/webserver/webserver.toml; \
    install -m 0644 packaging/sites/localhost.conf /package/usr/share/webserver/localhost.conf; \
    printf 'Package: webserver\nVersion: %s\nSection: web\nPriority: optional\nArchitecture: amd64\nMaintainer: Webserver maintainers\nDescription: Caddy-inspired reverse proxy and web server\n' "$version" > /package/DEBIAN/control; \
    cp README.md /package/usr/share/doc/webserver/README.md; \
    cp LICENSE /package/usr/share/doc/webserver/LICENSE; \
    cp LICENSE /package/usr/share/doc/webserver/copyright; \
    cp -a /dashboard/dist/. /package/usr/share/webserver/dashboard/; \
    dpkg-deb --build --root-owner-group /package "/webserver_${version}_amd64.deb"

FROM debian:bookworm@sha256:813017f3d62be4b5891a7acca6a01bdcd4b8513daa81b1ab99d3a50385b26931 AS repository

ARG APT_GPG_FINGERPRINT

RUN apt-get update && apt-get install --no-install-recommends -y reprepro gnupg ca-certificates && rm -rf /var/lib/apt/lists/*
COPY reprepro/distributions /tmp/distributions
COPY --from=build /webserver_*_amd64.deb /tmp/
COPY --from=build /src/LICENSE /repo/LICENSE

RUN --mount=type=secret,id=apt_gpg_private_key_b64,required=true \
    set -eu; \
    test -n "$APT_GPG_FINGERPRINT"; \
    export GNUPGHOME=/tmp/gnupg; \
    install -d -m 0700 "$GNUPGHOME" /repo/conf; \
    base64 -d /run/secrets/apt_gpg_private_key_b64 | gpg --batch --import; \
    fingerprint="$(gpg --batch --with-colons --list-secret-keys "$APT_GPG_FINGERPRINT" | awk -F: '/^fpr:/{print $10; exit}')"; \
    test "$fingerprint" = "$APT_GPG_FINGERPRINT"; \
    sed "s/__GPG_KEY_FINGERPRINT__/$fingerprint/" /tmp/distributions > /repo/conf/distributions; \
    reprepro -b /repo includedeb stable /tmp/webserver_*_amd64.deb; \
    gpg --batch --armor --export "$fingerprint" > /repo/webserver-archive-keyring.asc; \
    gpg --batch --export "$fingerprint" > /repo/webserver-archive-keyring.gpg; \
    rm -rf "$GNUPGHOME" /tmp/webserver_*_amd64.deb

FROM nginx:alpine@sha256:4a73073bd557c65b759505da037898b61f1be6cbcc3c2c3aeac22d2a470c1752

ENV TZ=Europe/Berlin

RUN apk add --no-cache tzdata

COPY --from=repository /repo /seed
COPY nginx.conf /etc/nginx/conf.d/default.conf
COPY entrypoint.sh /docker-entrypoint.d/30-seed-repository.sh
COPY README.md /seed/README.md

RUN rm -f /usr/share/nginx/html/index.html /usr/share/nginx/html/50x.html \
    && chmod 0755 /docker-entrypoint.d/30-seed-repository.sh \
    && mkdir -p /seed/debian /seed/rpm /seed/arch /seed/keys

VOLUME ["/repo"]

EXPOSE 8080
