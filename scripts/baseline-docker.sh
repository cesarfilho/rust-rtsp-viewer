#!/usr/bin/env bash
# Baseline do rrv-daemon no Docker: CPU, memória, banda e o que o daemon relata por câmera.
# Uso: scripts/baseline-docker.sh <rótulo> <config.toml> [aquecimento=25] [amostra=30] [env-file]
#   - roda a imagem rust-rtsp-viewer/rrv-daemon:local com esse config (gravação em /data, sem gravar);
#   - espera o aquecimento, amostra `docker stats` 1×/s durante a amostra e lê `rrvctl status --json`;
#   - imprime uma linha de tabela markdown e acrescenta uma linha em docs/baseline.csv.
# Segredos: use `${NOME}` nas URLs do config e passe o valor num env-file (chmod 600); ele nunca vai
# para a linha de comando nem para a saída.
# Requer docker e python3. Só Linux. `cpu` = % de UM núcleo (100 = 1 núcleo cheio).
set -euo pipefail

LABEL="${1:?uso: $0 <rótulo> <config.toml> [aquecimento] [amostra] [env-file]}"
CONFIG="$(readlink -f "${2:?falta o config}")"
WARM="${3:-25}"; SAMPLE="${4:-30}"; ENVFILE="${5:-}"
IMG="${RRV_IMAGE:-rust-rtsp-viewer/rrv-daemon:local}"
NAME="rrvbase-$$"
WORK="$(mktemp -d)"; mkdir -p "$WORK/state" "$WORK/data" "$WORK/run"
cleanup() { docker rm -f "$NAME" >/dev/null 2>&1 || true; rm -rf "$WORK"; }
trap cleanup EXIT

args=(--name "$NAME" --network host --user "$(id -u):$(id -g)"
  -v "$CONFIG:/config/config.toml:ro" -v "$WORK/data:/data" -v "$WORK/state:/state" -v "$WORK/run:/run/rrv")
[ -n "$ENVFILE" ] && args+=(--env-file "$ENVFILE")
[ -n "${RRV_DOCKER_ARGS:-}" ] && read -r -a extra <<<"$RRV_DOCKER_ARGS" && args+=("${extra[@]}")
docker run -d "${args[@]}" "$IMG" >/dev/null

# banda: bytes recebidos pela interface da rota padrão durante a amostra
IFACE="$(ip route show default | awk '/default/ {print $5; exit}')"
rx() { awk -v i="$IFACE:" '$1==i {print $2}' /proc/net/dev; }

sleep "$WARM"
rx0="$(rx)"; t0="$(date +%s)"
cpus=(); mems=()
for _ in $(seq 1 "$SAMPLE"); do
  read -r c m < <(docker stats --no-stream --format '{{.CPUPerc}} {{.MemUsage}}' "$NAME" | awk '{gsub("%","",$1); print $1, $2}')
  cpus+=("$c"); mems+=("$m"); sleep 1
done
rx1="$(rx)"; t1="$(date +%s)"
status="$(docker exec "$NAME" rrvctl status --json 2>/dev/null || echo '[]')"
docker stop "$NAME" >/dev/null 2>&1 || true

python3 - "$LABEL" "$IFACE" "$rx0" "$rx1" "$t0" "$t1" "${cpus[*]}" "${mems[*]}" "$status" <<'PY'
import json, sys, statistics as st
label, iface, rx0, rx1, t0, t1, cpus, mems, status = sys.argv[1:]
cpus = [float(x) for x in cpus.split()]
def mib(s):
    s = s.strip()
    for unit, k in (("GiB", 1024), ("MiB", 1), ("KiB", 1/1024), ("B", 1/1048576)):
        if s.endswith(unit):
            return float(s[: -len(unit)]) * k
    return 0.0
mem = [mib(x) for x in mems.split()]
cams = json.loads(status or "[]")
live = [c for c in cams if c["status"] in ("live", "recording")]
secs = max(1, int(t1) - int(t0))
mbps = (int(rx1) - int(rx0)) * 8 / secs / 1e6
dec = sorted({f'{c["decoder"]}{" GPU" if c.get("decoder_hw") else ""}' for c in live if c.get("decoder")})
res = sorted({f'{c["width"]}x{c["height"]}' for c in live if c.get("width")})
fps = [c["fps"] for c in live if c.get("fps")]
kbps = sum(c.get("bitrate_kbps", 0) for c in live)
row = dict(label=label, cams=len(cams), live=len(live), cpu_avg=st.mean(cpus), cpu_max=max(cpus),
           rss_mib=st.mean(mem) if mem else 0, net_mbps=mbps, stream_kbps=kbps,
           fps_avg=st.mean(fps) if fps else 0, res="/".join(res) or "-", decoder="/".join(dec) or "-")
print(f'| {label} | {row["live"]}/{row["cams"]} | {row["cpu_avg"]:.0f}% | {row["cpu_max"]:.0f}% | '
      f'{row["rss_mib"]:.0f} MiB | {row["net_mbps"]:.1f} Mb/s | {row["stream_kbps"]/1000:.1f} Mb/s | '
      f'{row["fps_avg"]:.0f} | {row["res"]} | {row["decoder"]} |')
import os, csv
path = os.path.join(os.environ.get("RRV_BASELINE_CSV", "docs/baseline.csv"))
new = not os.path.exists(path)
with open(path, "a", newline="") as f:
    w = csv.writer(f)
    if new:
        w.writerow(["label", "cams", "live", "cpu_avg_pct", "cpu_max_pct", "rss_mib", "net_mbps",
                    "stream_mbps", "fps_avg", "resolution", "decoder"])
    w.writerow([label, row["cams"], row["live"], f'{row["cpu_avg"]:.1f}', f'{row["cpu_max"]:.1f}',
                f'{row["rss_mib"]:.0f}', f'{row["net_mbps"]:.2f}', f'{row["stream_kbps"]/1000:.2f}',
                f'{row["fps_avg"]:.1f}', row["res"], row["decoder"]])
PY
