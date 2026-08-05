FROM rust:bookworm AS build

ARG SOURCE_REPOSITORY=https://github.com/KingIronMan2011/WebServer.git
ARG SOURCE_REF=main

RUN apt-get update && apt-get install --no-install-recommends -y ca-certificates git && rm -rf /var/lib/apt/lists/*
COPY debian/postinst /tmp/postinst
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

RUN cargo build --locked --release

RUN set -eux; \
    version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"; \
    install -d /package/DEBIAN /package/usr/local/bin /package/lib/systemd/system /package/usr/share/doc/webserver /package/usr/share/webserver; \
    install -m 0755 target/release/webserver /package/usr/local/bin/webserver; \
    install -m 0644 packaging/systemd/webserver.service /package/lib/systemd/system/webserver.service; \
    install -m 0755 /tmp/postinst /package/DEBIAN/postinst; \
    install -m 0644 packaging/webserver.toml /package/usr/share/webserver/webserver.toml; \
    install -m 0644 packaging/sites/localhost.conf /package/usr/share/webserver/localhost.conf; \
    printf 'Package: webserver\nVersion: %s\nSection: web\nPriority: optional\nArchitecture: amd64\nMaintainer: Webserver maintainers\nDescription: Caddy-inspired reverse proxy and web server\n' "$version" > /package/DEBIAN/control; \
    cp README.md /package/usr/share/doc/webserver/README.md; \
    cp LICENSE /package/usr/share/doc/webserver/LICENSE; \
    cp LICENSE /package/usr/share/doc/webserver/copyright; \
    dpkg-deb --build --root-owner-group /package "/webserver_${version}_amd64.deb"

FROM debian:bookworm AS repository

ARG APT_GPG_FINGERPRINT

RUN apt-get update && apt-get install --no-install-recommends -y reprepro gnupg ca-certificates && rm -rf /var/lib/apt/lists/*
COPY reprepro/distributions /tmp/distributions
COPY --from=build /webserver_*_amd64.deb /tmp/
COPY --from=build /src/LICENSE /repo/LICENSE

# The secret is base64 encoded so Dokploy can store it as a single build-secret
# value. The key only exists in this intermediate image layer and is removed
# before the nginx runtime image is assembled.
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

FROM nginx:alpine
COPY --from=repository /repo/dists /usr/share/nginx/html/dists
COPY --from=repository /repo/pool /usr/share/nginx/html/pool
COPY --from=repository /repo/webserver-archive-keyring.asc /usr/share/nginx/html/webserver-archive-keyring.asc
COPY --from=repository /repo/webserver-archive-keyring.gpg /usr/share/nginx/html/webserver-archive-keyring.gpg
COPY --from=repository /repo/LICENSE /usr/share/nginx/html/LICENSE
COPY nginx.conf /etc/nginx/conf.d/default.conf
# The base image's welcome/error pages would otherwise be exposed or take
# precedence over the repository directory listing.
RUN rm -f /usr/share/nginx/html/index.html /usr/share/nginx/html/50x.html
EXPOSE 8080
