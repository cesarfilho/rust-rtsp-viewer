#!/usr/bin/env bash
# Mede CPU e memória da JANELA (o processo rust-rtsp-viewer), como o baseline-docker.sh faz com o daemon.
# Referência para a tarefa 2.3 (NV12 + shader): é a janela que ainda converte cada quadro para RGBA.
#   scripts/baseline-window.sh <rótulo> <config.toml> [aquecimento=25] [amostra=30] [env-file] [binário]
#   - abre a janela de verdade (em motor local, `--embedded`) na sua sessão por ~aquecimento+amostra segundos;
#   - amostra /proc/<pid>/stat 1×/s: `cpu` = % de UM núcleo (100 = 1 núcleo cheio), RSS em MiB, threads;
#   - imprime uma linha de tabela markdown e acrescenta uma linha em docs/baseline-window.csv.
# Segredos: use `${NOME}` nas URLs do config e passe o valor num env-file (chmod 600); ele nunca é impresso.
# Importante: o PID vem do `$!` (o nome do processo é truncado em 15 caracteres: `pgrep -x` com o nome inteiro não acha).
set -euo pipefail
LABEL="${1:?uso: $0 <rótulo> <config.toml> [aquecimento] [amostra] [env-file] [binário]}"
CONFIG="$(readlink -f "${2:?falta o config}")"
WARM="${3:-25}"; SAMPLE="${4:-30}"; ENVFILE="${5:-}"
BIN="${6:-target/release/rust-rtsp-viewer}"
[ -x "$BIN" ] || { echo "binário não encontrado: $BIN (make release)"; exit 1; }
CSV="docs/baseline-window.csv"
TICKS="$(getconf CLK_TCK)"

if [ -n "$ENVFILE" ]; then set -a; . "$ENVFILE"; set +a; fi
"$BIN" --embedded "$CONFIG" >/dev/null 2>&1 &
PID=$!
trap 'kill "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true' EXIT

cpu_ticks() { awk '{print $14+$15}' "/proc/$PID/stat"; }   # utime+stime
sleep "$WARM"
kill -0 "$PID" 2>/dev/null || { echo "a janela fechou durante o aquecimento"; exit 1; }

cpus=(); rss=(); thr=()
prev="$(cpu_ticks)"
for _ in $(seq 1 "$SAMPLE"); do
  sleep 1
  kill -0 "$PID" 2>/dev/null || { echo "a janela fechou durante a amostra"; exit 1; }
  now="$(cpu_ticks)"
  cpus+=("$(awk -v a="$prev" -v b="$now" -v t="$TICKS" 'BEGIN{printf "%.1f", (b-a)*100/t}')")
  prev="$now"
  rss+=("$(awk '/VmRSS/{printf "%.0f", $2/1024}' "/proc/$PID/status")")
  thr+=("$(awk '/Threads/{print $2}' "/proc/$PID/status")")
done

python3 - "$LABEL" "$CONFIG" "$CSV" "$BIN" "${cpus[*]}" "${rss[*]}" "${thr[*]}" <<'PY'
import sys, re, os, tomllib
label, cfg, csv, binp = sys.argv[1:5]
cpu = [float(x) for x in sys.argv[5].split()]
rss = [float(x) for x in sys.argv[6].split()]
thr = [int(x) for x in sys.argv[7].split()]
try:
    cams = len(tomllib.load(open(cfg, 'rb')).get('cameras', []))
except Exception:
    cams = 0
avg = lambda v: sum(v) / len(v)
row = f"| {label} | {cams} | {avg(cpu):.0f}% | {max(cpu):.0f}% | {avg(rss):.0f} MiB | {max(rss):.0f} MiB | {max(thr)} |"
print("| cenário | câmeras | CPU méd | CPU máx | RSS méd | RSS máx | threads |")
print("|---|---|---|---|---|---|---|")
print(row)
new = not os.path.exists(csv)
with open(csv, 'a') as f:
    if new:
        f.write("label,cams,cpu_avg_pct,cpu_max_pct,rss_avg_mib,rss_max_mib,threads_max,binary\n")
    f.write(f'"{label}",{cams},{avg(cpu):.1f},{max(cpu):.1f},{avg(rss):.0f},{max(rss):.0f},{max(thr)},{os.path.basename(binp)}\n')
PY
