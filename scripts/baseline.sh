#!/usr/bin/env bash
# Amostra CPU/memória do rust-rtsp-viewer durante N segundos e imprime CSV.
# Uso: scripts/baseline.sh [segundos=60] [intervalo=2] [nome_do_processo=rust-rtsp-viewer]
# Rode com o app já aberto usando o config.toml a medir (1, 4, 16 câmeras) e anote o resultado
# em docs/baseline.md. cpu_pct = % de UM núcleo (200 = 2 núcleos cheios), calculado por delta
# de /proc (o `ps %cpu` é média desde o início do processo e não serve).
# Só Linux (/proc). Windows/macOS: usar Get-Counter / `top -pid` e manter as mesmas colunas.
# GPU NVIDIA: se `nvidia-smi` existir, inclui utilização e memória.
set -euo pipefail

DUR="${1:-60}"; STEP="${2:-2}"; NAME="${3:-rust-rtsp-viewer}"
[ -r /proc/self/stat ] || { echo "requer /proc (Linux)" >&2; exit 1; }
# O kernel guarda só os 15 primeiros caracteres do nome do processo (/proc/PID/comm), então
# `pgrep -x rust-rtsp-viewer` (16 caracteres) nunca casava e o script saía sem medir nada.
PID="$(pgrep -xn "${NAME:0:15}" || true)"
[ -n "$PID" ] || { echo "processo '$NAME' não encontrado" >&2; exit 1; }

HZ="$(getconf CLK_TCK)"
GPU=0; command -v nvidia-smi >/dev/null 2>&1 && GPU=1
echo "t_s,cpu_pct,rss_mb,threads$([ $GPU = 1 ] && echo ',gpu_util_pct,gpu_mem_mb')"

# utime+stime (campos 14 e 15). Remove "pid (comm)" antes de dividir, pois comm pode ter espaços.
ticks() { sed 's/^.*) //' "/proc/$PID/stat" | awk '{print $12 + $13}'; }

prev="$(ticks)"; t=0
while [ "$t" -lt "$DUR" ] && kill -0 "$PID" 2>/dev/null; do
  sleep "$STEP"; t=$((t + STEP))
  cur="$(ticks)"
  cpu="$(awk -v a="$prev" -v b="$cur" -v hz="$HZ" -v s="$STEP" 'BEGIN{printf "%.1f", (b-a)/hz/s*100}')"
  prev="$cur"
  rss="$(awk '/VmRSS/{printf "%.1f",$2/1024}' "/proc/$PID/status")"
  thr="$(awk '/Threads/{print $2}' "/proc/$PID/status")"
  line="$t,$cpu,$rss,$thr"
  if [ $GPU = 1 ]; then
    g="$(nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader,nounits | head -1 | tr -d ' ')"
    line="$line,$g"
  fi
  echo "$line"
done
