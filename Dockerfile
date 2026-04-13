# ── Build from source (local development) ──────────────────────
FROM rust:1-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release && strip target/release/mcc

# ── Shared runtime base ─────────────────────────────────────────
FROM linuxserver/ffmpeg AS runtime

COPY encoding.yaml /etc/mcc/encoding.yaml
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

ENV MCC_TEMP_DIR=""

RUN mkdir -p /tmp/encoding
VOLUME ["/tmp/encoding"]
VOLUME ["/media"]

ENTRYPOINT ["docker-entrypoint.sh"]
CMD ["/media"]

# ── CI: inject pre-built multi-arch binaries (--target release) ─
FROM runtime AS release
ARG TARGETARCH
COPY bin/${TARGETARCH}/mcc /usr/local/bin/mcc

# ── Default: build from source ──────────────────────────────────
FROM runtime AS local
COPY --from=builder /build/target/release/mcc /usr/local/bin/mcc
