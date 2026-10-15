# anon v0.4.10.2: the Anyone Protocol daemon the connector dials hidden
# services through (connector ADR 0070). The relay's connector only DIALS, so
# this daemon is a SOCKS5 client and nothing else: it publishes no service and
# relays nobody's traffic (see ../anonrc).
#
# ghcr.io/anyone-protocol/ator-protocol publishes no image past v0.4.9.7, and
# v0.4.9.7 routes `.onion`, not `.anyone` (connector#1284). So this overlays
# the official v0.4.10.2 release binary on that image, checked against the
# same sha256 toon-client and the connector's local/anon-image pin, BEFORE it
# is ever executed. The same recipe as infra's sandbox/anon/Dockerfile, less
# socat and iptables, which only a serving daemon needs.
FROM ghcr.io/anyone-protocol/ator-protocol:v0.4.9.7

ARG ANON_VERSION=v0.4.10.2
ARG ANON_SHA256=9c6498b8d27de54d78842a1b854979a605f9c140ccc34f2b4c267bf094eaeb17

USER root
RUN set -eux; \
    apt-get update; \
    apt-get install -y --no-install-recommends curl unzip ca-certificates; \
    curl -fsSL -o /tmp/anon.zip \
      "https://github.com/anyone-protocol/ator-protocol/releases/download/${ANON_VERSION}/anon-live-linux-amd64.zip"; \
    echo "${ANON_SHA256}  /tmp/anon.zip" | sha256sum -c -; \
    unzip -o -j /tmp/anon.zip -d /tmp/anon; \
    install -m 0755 /tmp/anon/anon /usr/local/bin/anon; \
    install -m 0644 /tmp/anon/geoip  /usr/local/share/anon/geoip; \
    install -m 0644 /tmp/anon/geoip6 /usr/local/share/anon/geoip6; \
    apt-get purge -y curl unzip; apt-get autoremove -y; \
    rm -rf /tmp/anon /tmp/anon.zip /var/lib/apt/lists/*; \
    anon --version | grep -F "${ANON_VERSION#v}"
# No USER line: the image's docker-entrypoint runs as root to set up
# /var/lib/anon, and the daemon drops to `anond` itself (`User anond` in
# ../anonrc). Ending on `USER anond` stops the entrypoint at its first write.
