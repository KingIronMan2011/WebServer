# Build from the repository root: docker build -f apt-repo/Dockerfile .
FROM rust:bookworm AS build

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY assets ./assets
COPY packaging ./packaging
COPY apt-repo ./apt-repo
COPY README.md ./README.md

RUN cargo build --locked --release

RUN set -eux; \
    version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)"; \
    install -d /package/DEBIAN /package/usr/local/bin /package/lib/systemd/system /package/usr/share/doc/webserver /package/usr/share/webserver; \
    install -m 0755 target/release/webserver /package/usr/local/bin/webserver; \
    install -m 0644 packaging/systemd/webserver.service /package/lib/systemd/system/webserver.service; \
    install -m 0755 apt-repo/debian/postinst /package/DEBIAN/postinst; \
    install -m 0644 packaging/webserver.toml /package/usr/share/webserver/webserver.toml; \
    install -m 0644 packaging/sites/localhost.conf /package/usr/share/webserver/localhost.conf; \
    printf 'Package: webserver\nVersion: %s\nSection: web\nPriority: optional\nArchitecture: amd64\nMaintainer: Webserver maintainers\nDescription: Caddy-inspired reverse proxy and web server\n' "$version" > /package/DEBIAN/control; \
    cp README.md /package/usr/share/doc/webserver/README.md; \
    dpkg-deb --build --root-owner-group /package "/webserver_${version}_amd64.deb"

FROM debian:bookworm AS repository
RUN apt-get update && apt-get install --no-install-recommends -y dpkg-dev && rm -rf /var/lib/apt/lists/*
COPY --from=build /webserver_*_amd64.deb /repo/pool/main/w/webserver/
RUN set -eux; \
    install -d /repo/dists/stable/main/binary-amd64; cd /repo; \
    dpkg-scanpackages --arch amd64 pool /dev/null > dists/stable/main/binary-amd64/Packages; \
    gzip -9c dists/stable/main/binary-amd64/Packages > dists/stable/main/binary-amd64/Packages.gz; \
    { printf 'Origin: Webserver\nLabel: Webserver\nSuite: stable\nCodename: stable\nArchitectures: amd64\nComponents: main\n'; printf 'Date: '; date -Ru; printf 'SHA256:\n'; for file in dists/stable/main/binary-amd64/Packages dists/stable/main/binary-amd64/Packages.gz; do printf ' %s %s %s\n' "$(sha256sum "$file" | cut -d' ' -f1)" "$(stat -c%s "$file")" "$file"; done; } > dists/stable/Release

# Runtime image: only nginx and the generated APT repository are retained.
# Rust, source code, dpkg-dev and all intermediate build files stay in prior stages.
FROM nginx:alpine
COPY --from=repository /repo /usr/share/nginx/html
COPY apt-repo/nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 8080
