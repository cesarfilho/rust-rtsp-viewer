# 0005 — Ramo de detecção separado do display
**Status:** Proposta (revisada em 2026-09-24; depende de 0008)

## Contexto
O `appsink` de display recebe RGBA em resolução cheia e faz uma cópia por frame (`Bytes::copy_from_slice`,
`ui/pipeline.rs` `setup_appsink`); depois o buffer é compartilhado entre `Handle` e snapshot (sem cópias extras).
O `tee` já existe (`insert_tee`, depois do `capsfilter` RGBA): `tee → display_queue → appsink`.
O Frigate usa um "detect stream" de baixa resolução, separado do stream de gravação.

## Decisão
Movimento e ML consomem um frame reduzido, nunca o RGBA de tela cheia.
Ramo **a partir do tee atual** (opção mínima):
`tee → detect_queue(leaky=downstream, max-size-buffers=1) → videorate → videoscale → capsfilter(≈320x180, GRAY8|RGBA, ≤detect_fps) → detect_sink(appsink)`.
**Ordem importa:** `videorate` antes de `videoscale`, para descartar frames antes de pagar a escala.

## Limite desta opção (importante)
O tee está *depois* de `decoder → videoconvert → RGBA` em resolução cheia. A conversão/decodificação em 1080p
já foi paga para o display; o ramo só adiciona escala barata. Mas **só funciona enquanto a câmera está
decodificando para a UI** — ver ADR 0008 (`pause_hidden` desliga o pipeline de câmeras fora da página).

## Consequências
- Custo adicional por câmera pequeno e constante; display inalterado.
- O ramo de *gravação* continua existindo só durante a gravação (regra do AGENTS.md). O ramo de detecção é permanente
  e consome CPU mesmo ocioso — por isso é limitado por `detect_fps` e pode ser desligado por câmera.
- Validar: um terceiro pad no `tee` não quebra o `stop_recording` (bloqueio de pad + EOS no ramo de gravação).
- Alternativa (ADR 0008, opção B): pipeline de detecção próprio sobre o sub-stream.
