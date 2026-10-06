#!/usr/bin/env bash
# Prepara o ambiente do roteiro A8 (docs/roteiro-a8.md): sobe o daemon no Docker com a SUA câmera RTSP
# do config.toml aparecendo como duas câmeras (CamA e CamB), grava dois trechos com uma lacuna entre eles
# e imprime o comando da janela. Nada fica no seu config.toml; tudo vai para /tmp/rrv-a8 (0700).
#   scripts/a8-demo.sh [start] [config.toml]     # prepara (padrão)
#   scripts/a8-demo.sh stop                      # derruba e apaga
# A senha da câmera sai do config.toml só para um arquivo 0600 em /tmp/rrv-a8; nunca é impressa.
set -euo pipefail
CMD="${1:-start}"
ROOT=/tmp/rrv-a8
NAME=rrv-a8
IMG="${RRV_IMAGE:-rust-rtsp-viewer/rrv-daemon:0.9.0}"

if [ "$CMD" = stop ]; then
  docker rm -f "$NAME" >/dev/null 2>&1 || true
  rm -rf "$ROOT"
  echo "parado e apagado ($ROOT)"
  exit 0
fi

CONFIG="$(readlink -f "${2:-config.toml}")"
[ -f "$CONFIG" ] || { echo "não achei $CONFIG (passe o caminho do config.toml)"; exit 1; }
docker image inspect "$IMG" >/dev/null 2>&1 || { echo "imagem $IMG não existe: docker build -t $IMG ."; exit 1; }
docker rm -f "$NAME" >/dev/null 2>&1 || true
rm -rf "$ROOT"
umask 077
mkdir -p "$ROOT"/data "$ROOT"/state "$ROOT"/run

python3 - "$CONFIG" "$ROOT" <<'PY'
import os, re, sys, tomllib
cfg = tomllib.load(open(sys.argv[1], 'rb'))
root = sys.argv[2]
cam = next((c for c in cfg.get('cameras', []) if str(c.get('url', '')).startswith('rtsp://')), None)
if cam is None:
    sys.exit("o config.toml não tem nenhuma câmera rtsp://")
m = re.match(r'rtsp://([^:@/]+):([^@]+)@(.*)', cam['url'])
if not m:
    sys.exit("a câmera rtsp:// do config.toml não tem usuário e senha na URL")
user, pw, rest = m.groups()
open(f'{root}/env', 'w').write(f'DEMO_PASS={pw}\n')
os.chmod(f'{root}/env', 0o600)
cams = ''.join(f'[[cameras]]\nurl = "rtsp://{user}:${{DEMO_PASS}}@{rest}"\nname = "{n}"\n' for n in ('CamA', 'CamB'))
open(f'{root}/daemon.toml', 'w').write(
    '[recording]\ndir = "/data"\n[logs]\ndir = "/state/logs"\n[motion]\nenabled = false\n' + cams)
open(f'{root}/window.toml', 'w').write(
    f'[recording]\ndir = "{root}/data"\n[logs]\ndir = "{root}/winlogs"\n[motion]\nenabled = false\n' + cams)
PY

docker run -d --name "$NAME" --network host --user "$(id -u):$(id -g)" --env-file "$ROOT/env" \
  -v "$ROOT/daemon.toml:/config/config.toml:ro" -v "$ROOT/data:/data" -v "$ROOT/state:/state" -v "$ROOT/run:/run/rrv" \
  "$IMG" >/dev/null

echo -n "esperando as duas câmeras ficarem ao vivo"
live=0
for _ in $(seq 1 60); do
  live="$(docker exec "$NAME" rrvctl status 2>/dev/null | grep -c ' live ' || true)"
  [ "$live" -ge 2 ] && break
  echo -n "."; sleep 1
done
echo
[ "$live" -ge 2 ] || { echo "só $live câmera(s) ao vivo; veja: docker logs $NAME"; exit 1; }

for round in 1 2; do
  echo "gravando o trecho $round (12 s, as duas câmeras juntas)..."
  docker exec "$NAME" rrvctl record CamA >/dev/null
  docker exec "$NAME" rrvctl record CamB >/dev/null
  sleep 12
  docker exec "$NAME" rrvctl record CamA >/dev/null
  docker exec "$NAME" rrvctl record CamB >/dev/null
  [ "$round" = 1 ] && { echo "lacuna de 10 s sem gravar..."; sleep 10; }
done
sleep 3
echo
docker exec "$NAME" rrvctl history --hours 1 | head -8
cat <<MSG

Pronto. Abra a janela ligada ao daemon (em outro terminal, na raiz do repositório):

  set -a; . $ROOT/env; set +a
  RRV_SOCKET=$ROOT/run/rrv.sock ./target/release/rust-rtsp-viewer $ROOT/window.toml

Para o passo do Ctrl+Q com gravação local (sem o daemon):

  set -a; . $ROOT/env; set +a
  ./target/release/rust-rtsp-viewer --embedded $ROOT/window.toml

Quando terminar:  scripts/a8-demo.sh stop
MSG
