# rrv-daemon: o motor do rust-rtsp-viewer sem janela (ADR 0010).
#
#   docker compose up -d --build
#
# Só o daemon vai para o contêiner; a janela continua nativa no host.

# ---- compilação ------------------------------------------------------------
# O imagem `rust:1` acompanha o stable; o MSRV do projeto é 1.92 (gstreamer 0.25).
FROM rust:1-bookworm AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev pkg-config \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
# O manifesto raiz lista o crate da janela como membro, então seus fontes precisam
# existir para o cargo ler o workspace; ele NÃO é compilado (só `-p rrv-daemon`),
# logo nenhuma biblioteca gráfica é necessária.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p rrv-daemon \
 && cp target/release/rrv-daemon target/release/rrvctl /

# ---- execução --------------------------------------------------------------
FROM debian:bookworm-slim
# good/bad/ugly/libav: decodebin, hlsdemux, x264enc (gravação) e os decodificadores.
# curl: o webhook de aviso (`[webhook]`) o usa como subprocesso.
# intel-media-va-driver + vainfo: decodificação por GPU Intel (VA-API) quando o contêiner recebe
# /dev/dri (compose.vaapi.yaml). Sem o dispositivo, o GStreamer cai para a CPU sozinho.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libgstreamer1.0-0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
      gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav \
      ca-certificates tzdata curl \
      intel-media-va-driver vainfo \
 && rm -rf /var/lib/apt/lists/*

COPY --from=build /rrv-daemon /rrvctl /usr/local/bin/

# Não-root. O compose troca o UID/GID para os do dono dos volumes; por isso HOME e
# o estado ficam em /state (gravável por qualquer UID quando o volume é do dono).
RUN useradd --uid 1000 --no-create-home --shell /usr/sbin/nologin rrv \
 && mkdir -p /config /data /state /run/rrv && chown 1000:1000 /data /state /run/rrv
ENV HOME=/state \
    XDG_STATE_HOME=/state \
    RRV_CONFIG=/config/config.toml \
    RRV_SOCKET=/run/rrv/rrv.sock \
    RUST_LOG=info
USER rrv
VOLUME ["/data", "/state", "/run/rrv"]

# O contêiner está saudável enquanto o laço do daemon bate o coração. Uma câmera
# fora do ar não o torna doente: reiniciar não a consertaria.
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
  CMD ["rrv-daemon", "--health"]

# `docker stop` envia SIGTERM; o daemon finaliza as gravações antes de sair.
STOPSIGNAL SIGTERM
ENTRYPOINT ["rrv-daemon"]
