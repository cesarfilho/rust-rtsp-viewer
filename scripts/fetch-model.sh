#!/usr/bin/env bash
# Baixa o peso YOLO11n da Ultralytics (AGPL-3.0), confere o SHA-256 e exporta para ONNX em models/.
# Baixa também a libonnxruntime (CPU) para models/onnxruntime/, que o daemon carrega em execução.
# Nada disto entra no repositório. Uso: scripts/fetch-model.sh [320|640 ...]   (padrão: 640)
set -euo pipefail
cd "$(dirname "$0")/.."

URL="https://github.com/ultralytics/assets/releases/download/v8.3.0/yolo11n.pt"
SHA256="0ebbc80d4a7680d14987a577cd21342b65ecfd94632bd9a8da63ae6417644ee1"
SIZES=("$@"); [ ${#SIZES[@]} -gt 0 ] || SIZES=(640)

mkdir -p models
if [ ! -f models/yolo11n.pt ]; then
  echo "baixando yolo11n.pt"
  curl -fL --retry 3 -o models/yolo11n.pt.part "$URL"
  mv models/yolo11n.pt.part models/yolo11n.pt
fi
echo "$SHA256  models/yolo11n.pt" | sha256sum -c - || { echo "SHA-256 diferente: apague models/yolo11n.pt e confira a origem" >&2; exit 1; }

command -v uv >/dev/null || { echo "precisa do uv (https://docs.astral.sh/uv/) para exportar o ONNX" >&2; exit 1; }
for s in "${SIZES[@]}"; do
  out="models/yolo11n-$s.onnx"
  [ -f "$out" ] && { echo "$out já existe"; continue; }
  echo "exportando $out"
  (cd models && uv run --quiet --python 3.12 --with ultralytics --with onnx --with onnxscript \
      --extra-index-url https://download.pytorch.org/whl/cpu \
      python -c "from ultralytics import YOLO; import shutil; shutil.move(YOLO('yolo11n.pt').export(format='onnx', imgsz=$s, dynamic=False), 'yolo11n-$s.onnx')")
  sha256sum "$out"
done

# ---- ONNX Runtime (CPU) ------------------------------------------------------
ORT_VERSION="1.30.0"
ORT_SHA256="a5ed5a3cac51fbb2e90da632ae43d19212faaa20e76484e62bcb7c23ddb3b3fd"
ORT_LIB="models/onnxruntime/lib/libonnxruntime.so"
if [ ! -f "$ORT_LIB" ]; then
  echo "baixando a libonnxruntime $ORT_VERSION (CPU)"
  curl -fL --retry 3 -o models/ort.tgz "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VERSION}/onnxruntime-linux-x64-${ORT_VERSION}.tgz"
  echo "$ORT_SHA256  models/ort.tgz" | sha256sum -c - || { rm -f models/ort.tgz; echo "SHA-256 da libonnxruntime diferente" >&2; exit 1; }
  mkdir -p models/onnxruntime && tar xzf models/ort.tgz -C models/onnxruntime --strip-components=1 && rm models/ort.tgz
fi
echo
echo "pronto. Para usar a detecção:"
echo "  ORT_DYLIB_PATH=$PWD/$ORT_LIB ./bin/rrv-daemon config.toml     (daemon feito com: make bin FEATURES=detect)"
echo "  (GPU NVIDIA: veja docs/gpu-container.md e docs/spike-0.6-ort.md)"
