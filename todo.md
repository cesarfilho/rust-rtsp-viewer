# TODO — ideias inspiradas no Frigate (https://docs.frigate.video/)

Lacunas atuais: sem gravação por evento, sem retenção/limpeza de disco, sem histórico
persistente. Módulos de domínio existentes mas **não ligados**: `motion`, `zones`
(+ `ui/zone_editor.rs`), `ptz`, `multi_stream`, `timelapse`, `streaming`, `hw_encoder`,
`bidirectional_audio`.

## Fase 1 — ligar o que já existe
- [ ] Detecção de movimento (`domain/motion.rs`): seção `[motion]` (threshold, contour_area,
      lightning_threshold), emitir `EventType::Motion`, máscaras de movimento
- [ ] Zonas: ligar `ui/zone_editor.rs` a uma view; inertia (frames consecutivos) e loitering (tempo mínimo)
- [ ] Sub-stream no grid / main-stream no spotlight (`domain/multi_stream.rs`): `sub_url` em `[[cameras]]`
- [ ] "Smart streaming": snapshot estático quando ocioso, live ao detectar movimento

## Fase 2 — gravação inteligente
- [ ] Retenção por modo em `[recording]`: `all` / `motion` / `active_objects`
- [ ] Pré/pós-captura (ring buffer no `tee`; hoje o ramo de gravação só existe durante a gravação)
- [ ] Limpeza por espaço em disco (apagar segmentos mais antigos quando faltar espaço)
- [ ] Export de clipes a partir do timeline

## Fase 3 — revisão e histórico
- [ ] Review items: agrupar detecções sobrepostas; separar Alerts de Detections
- [ ] Timeline persistente (`~/.local/state/rust-rtsp-viewer/`) ligada aos segmentos gravados, com playback/seek
- [ ] Modo Birdseye: só câmeras com movimento/objetos nos últimos ~30 s (reusar `pause_hidden` + `sync_active_streams`)

## Fase 4 — avançado
- [ ] Detecção de objetos (ONNX/YOLO 320x320 sobre região recortada do movimento)
- [ ] PTZ ONVIF + autotracking (`domain/ptz.rs`)
- [ ] Áudio bidirecional (two-way talk) e detecção de áudio
- [ ] Re-streaming (RTSP/HLS de saída, estilo go2rtc), MQTT/notificações, API HTTP
