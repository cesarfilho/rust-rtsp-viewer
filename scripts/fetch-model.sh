#!/usr/bin/env bash
# Baixa o peso YOLO11n da Ultralytics (AGPL-3.0), confere o SHA-256 e exporta para ONNX em models/.
# O peso NÃO entra no repositório. Uso: scripts/fetch-model.sh [320|640 ...]   (padrão: 320 640)
set -euo pipefail
cd "$(dirname "$0")/.."

URL="https://github.com/ultralytics/assets/releases/download/v8.3.0/yolo11n.pt"
SHA256="0ebbc80d4a7680d14987a577cd21342b65ecfd94632bd9a8da63ae6417644ee1"
SIZES=("$@"); [ ${#SIZES[@]} -gt 0 ] || SIZES=(320 640)

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
