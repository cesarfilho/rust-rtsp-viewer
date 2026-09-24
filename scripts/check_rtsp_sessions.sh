#!/usr/bin/env bash
# Descobre quantas sessões RTSP simultâneas uma câmera aceita.
# Uso: scripts/check_rtsp_sessions.sh <rtsp-url> [max_sessoes=4] [segundos=8]
# Abre 1..N conexões (gst-launch-1.0 rtspsrc → fakesink) e informa quantas receberam vídeo.
# A senha da URL é mascarada na saída. Requer gst-launch-1.0 e timeout.
set -uo pipefail

URL="${1:?uso: $0 <rtsp-url> [max_sessoes] [segundos]}"; MAX="${2:-4}"; SECS="${3:-8}"
mask() { sed -E 's#(://[^:/@]+):[^@]*@#\1:****@#' <<<"$1"; }
echo "câmera: $(mask "$URL")  (até $MAX sessões, ${SECS}s cada teste)"

TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
best=0
for n in $(seq 1 "$MAX"); do
  pids=()
  for i in $(seq 1 "$n"); do
    ( timeout "$SECS" gst-launch-1.0 -m rtspsrc location="$URL" latency=200 protocols=tcp ! fakesink sync=false silent=true \
        >"$TMP/$n.$i.log" 2>&1; echo $? >"$TMP/$n.$i.rc" ) &
    pids+=($!)
  done
  wait "${pids[@]}" 2>/dev/null
  ok=0
  for i in $(seq 1 "$n"); do
    # timeout devolve 124 quando a sessão ficou viva até o fim = recebeu stream sem erro.
    [ "$(cat "$TMP/$n.$i.rc" 2>/dev/null)" = "124" ] && ok=$((ok + 1))
  done
  echo "  $n sessão(ões) simultânea(s): $ok ativa(s) até o fim"
  [ "$ok" -eq "$n" ] && best="$n" || break
done
echo "resultado: a câmera manteve até $best sessão(ões) simultânea(s) neste teste."
