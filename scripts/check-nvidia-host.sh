#!/usr/bin/env bash
# Confere se o host está pronto para o contêiner usar a GPU NVIDIA (decisão D6) e diz o que falta.
# Só LÊ: não instala nada nem usa sudo. Cada passo que falta vem com o comando para você rodar.
#   scripts/check-nvidia-host.sh [imagem]      # imagem padrão: rust-rtsp-viewer/rrv-daemon:local
set -u
IMG="${1:-rust-rtsp-viewer/rrv-daemon:local}"
ok=1
say() { printf '%s\n' "$*"; }
bad() { ok=0; say "✗ $1"; shift; for l in "$@"; do say "    $l"; done; }

if command -v nvidia-smi >/dev/null 2>&1 && gpu="$(nvidia-smi --query-gpu=name,driver_version --format=csv,noheader 2>/dev/null)" && [ -n "$gpu" ]; then
  say "✓ driver NVIDIA no host: $gpu"
else
  bad "driver NVIDIA não encontrado (nvidia-smi falhou)" "instale o driver do seu sistema e reinicie"
fi

if command -v nvidia-ctk >/dev/null 2>&1; then
  say "✓ nvidia-container-toolkit instalado ($(nvidia-ctk --version 2>/dev/null | head -1))"
else
  bad "nvidia-container-toolkit não instalado" \
      "sudo pacman -S nvidia-container-toolkit        # Arch/Omarchy" \
      "(Debian/Ubuntu: https://docs.nvidia.com/datacenter/cloud-native/container-toolkit/latest/install-guide.html)"
fi

if docker info 2>/dev/null | grep -qi 'runtimes:.*nvidia'; then
  say "✓ o Docker conhece o runtime nvidia"
else
  bad "o Docker ainda não tem o runtime nvidia" \
      "sudo nvidia-ctk runtime configure --runtime=docker" \
      "sudo systemctl restart docker"
fi

if ! docker image inspect "$IMG" >/dev/null 2>&1; then
  bad "a imagem $IMG não existe" "docker compose build   (ou: docker build -t $IMG .)"
elif [ "$ok" = 1 ]; then
  if out="$(docker run --rm --gpus all -e NVIDIA_DRIVER_CAPABILITIES=compute,video,utility --entrypoint nvidia-smi "$IMG" --query-gpu=name --format=csv,noheader 2>&1)"; then
    say "✓ o contêiner enxerga a GPU: $out"
  else
    bad "o contêiner NÃO enxerga a GPU" "$out"
  fi
fi

echo
if [ "$ok" = 1 ]; then
  say "Pronto. Suba o daemon com a GPU:"
  say "  docker compose -f compose.yaml -f compose.nvidia.yaml up -d"
  say "  docker exec rrv rrvctl status        # a coluna 'decodificador' deve mostrar nvh264dec (GPU)"
  exit 0
fi
say "Falta(m) o(s) passo(s) acima. Rode de novo este script depois."
exit 1
