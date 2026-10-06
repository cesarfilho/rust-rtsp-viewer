# rrv-daemon: o motor do rust-rtsp-viewer sem janela (ADR 0010).
#
#   docker compose up -d --build
#
# Só o daemon vai para o contêiner; a janela continua nativa no host.
#
# Três imagens saem deste arquivo (`--target`); a última etapa (`runtime`) é a padrão:
#   runtime       só o NVR (sem detecção de objetos)
#   detect-cpu    + detecção de objetos (YOLO) na CPU: libonnxruntime 1.30 dentro da imagem
#   detect-cuda   + detecção na GPU NVIDIA (CUDA 13 + cuDNN 9); precisa do compose.nvidia.yaml
# O modelo NÃO vai na imagem: `scripts/fetch-model.sh` o baixa para ./models, montado em /models.
# Veja compose.detect.yaml e compose.detect-cuda.yaml.

# ---- compilação ------------------------------------------------------------
# O imagem `rust:1` acompanha o stable; o MSRV do projeto é 1.92 (gstreamer 0.25).
FROM rust:1-bookworm AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev pkg-config \
 && rm -rf /var/lib/apt/lists/*
# `--build-arg FEATURES=detect` compila a detecção de objetos (carrega a libonnxruntime em execução).
ARG FEATURES=""
WORKDIR /src
# O manifesto raiz lista o crate da janela como membro, então seus fontes precisam
# existir para o cargo ler o workspace; ele NÃO é compilado (só `-p rrv-daemon`),
# logo nenhuma biblioteca gráfica é necessária.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p rrv-daemon ${FEATURES:+--features $FEATURES} \
 && cp target/release/rrv-daemon target/release/rrvctl /

# ---- ONNX Runtime (só as imagens com detecção) -----------------------------
# Binários oficiais da Microsoft, conferidos por SHA-256.
FROM debian:bookworm-slim AS ort-cpu
ARG ORT_VERSION=1.30.0
ARG ORT_CPU_SHA256=a5ed5a3cac51fbb2e90da632ae43d19212faaa20e76484e62bcb7c23ddb3b3fd
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && curl -fsSL -o /ort.tgz https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-${ORT_VERSION}.tgz \
 && echo "${ORT_CPU_SHA256}  /ort.tgz" | sha256sum -c - \
 && mkdir /ort && tar xzf /ort.tgz -C /ort --strip-components=1

FROM debian:bookworm-slim AS ort-cuda
ARG ORT_VERSION=1.30.0
ARG ORT_CUDA_SHA256=382d79133112388cf94ce5855789b7c9bef12bef76a08b6b277e5a317213adcd
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && curl -fsSL -o /ort.tgz https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-gpu_cuda13-${ORT_VERSION}.tgz \
 && echo "${ORT_CUDA_SHA256}  /ort.tgz" | sha256sum -c - \
 && mkdir /ort && tar xzf /ort.tgz -C /ort --strip-components=1

# ---- execução --------------------------------------------------------------
FROM debian:bookworm-slim AS runtime-base
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


# ---- variantes -------------------------------------------------------------
# Detecção de objetos na CPU. Build: `--build-arg FEATURES=detect --target detect-cpu`.
FROM runtime-base AS detect-cpu
COPY --from=ort-cpu /ort/lib/libonnxruntime.so* /opt/onnxruntime/
ENV ORT_DYLIB_PATH=/opt/onnxruntime/libonnxruntime.so

# Detecção na GPU NVIDIA: base com CUDA 13 + cuDNN 9 (o CDI do `compose.nvidia.yaml` injeta só o driver).
# Ubuntu 24.04 (glibc 2.39) executa o binário feito no bookworm (glibc 2.36).
FROM nvidia/cuda:13.0.3-cudnn-runtime-ubuntu24.04 AS detect-cuda
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libgstreamer1.0-0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
      gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav \
      ca-certificates tzdata curl \
 && rm -rf /var/lib/apt/lists/* \
 && (userdel -r ubuntu 2>/dev/null || true)
COPY --from=ort-cuda /ort/lib/libonnxruntime*.so* /opt/onnxruntime/
ENV ORT_DYLIB_PATH=/opt/onnxruntime/libonnxruntime.so \
    LD_LIBRARY_PATH=/opt/onnxruntime
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


# A imagem padrão (a última etapa): só o NVR.
FROM runtime-base AS runtime
