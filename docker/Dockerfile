# SPDX-License-Identifier: GPL-3.0-only

FROM rust:1.85-bookworm AS builder

WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

RUN cargo build --release -p ikaros-cli

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --home-dir /home/ikaros --shell /usr/sbin/nologin ikaros \
    && mkdir -p /data/ikaros /workspace \
    && chown -R ikaros:ikaros /data/ikaros /workspace

COPY --from=builder /src/target/release/ikaros /usr/local/bin/ikaros

USER ikaros
WORKDIR /workspace
ENV IKAROS_HOME=/data/ikaros

ENTRYPOINT ["ikaros"]
CMD ["--help"]
