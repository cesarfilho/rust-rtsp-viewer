# 0007 — Pré-captura sem quebrar a regra do tee
**Status:** Proposta

## Contexto
AGENTS.md: o ramo de gravação (x264enc) só existe enquanto grava, senão gasta um núcleo por câmera. Pré-captura (gravar N s antes do evento) parece exigir o contrário.

## Opções
A. **Ring buffer de vídeo já codificado**: guardar os últimos N s do stream *original* (H.264/H.265, sem transcodificar) e escrever direto no mux ao iniciar o evento (sem x264enc). Custo: memória (≈ bitrate × N s) e cuidado com keyframes. Sem CPU de encode.
B. Ramo de encode sempre ligado: rejeitado (custo por câmera × 16+).
C. Sem pré-captura: gravar a partir do evento; perde os primeiros segundos.

## Decisão proposta
Opção A, com início do clipe no keyframe anterior ao evento. Spike necessário: gravar direto do stream original (`rtph264depay ! h264parse ! splitmuxsink`), o que também elimina o transcode na gravação em geral.

## Consequências
- Muda `start_recording`/`stop_recording`; manter a regra do EOS (arquivo tocável).
- Não funciona para HLS/arquivo do mesmo jeito: tratar por tipo de fonte.

## Revisão (2026-09-24)
- **Ponto de captura:** hoje o ramo de gravação sai do `tee` **depois da decodificação e conversão para RGBA**. Gravar o stream original exige um
  ponto de captura **antes do decoder** (payload RTP depayed/parsed). Com `rtspsrc → decodebin` o parser é interno ao `decodebin`; será
  preciso montar `rtspsrc → rtph264depay → h264parse → tee` manualmente (o config já aceita cadeia customizada em `decoder`, mas não é o padrão).
- Resolve também H.265 e evita o limite de sessões NVENC (ADR 0002).
- **Granularidade da pré-captura:** limitada ao intervalo de keyframes da câmera (GOP de 1–4 s é comum); pré-captura de 5 s pode virar 5–9 s.
- **Memória:** ≈ bitrate × N s por câmera (ex.: 4 Mbps × 10 s ≈ 5 MB; ×16 câmeras ≈ 80 MB) — aceitável, mas medir.
- Áudio: a captura direta precisa incluir a faixa de áudio se ela existir (hoje o áudio é pipeline separado).
- Depende de ADR 0008 (onde fica o pipeline sempre ligado).
