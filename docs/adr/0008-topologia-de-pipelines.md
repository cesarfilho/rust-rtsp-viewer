# 0008 — Topologia de pipelines: detecção vs `pause_hidden`
**Status:** Proposta — 16 câmeras confirmadas; falta confirmar quantas sessões RTSP cada câmera aceita

## Problema
Hoje `update::sync_active_streams` (com `[view] pause_hidden`) faz `bridge.stop()` nas câmeras fora da página visível.
Câmera parada não decodifica → não há movimento, nem ML, nem gravação por evento. Com "muitas câmeras" em grid paginado,
a maioria estaria cega justamente quando o usuário não está olhando — o caso de uso principal de um NVR.

## Opções
A. **Um pipeline por câmera (atual)**, detecção no tee. Simples; exige decodificar tudo o tempo todo → anula `pause_hidden`
   e mantém 16+ decodificações completas.
B. **Dois pipelines por câmera**: (1) *detect/record* sempre ligado sobre o **sub-stream** (baixa resolução, decodifica barato,
   alimenta movimento/ML); (2) *display* só para câmeras visíveis, no main ou sub. Duas conexões RTSP por câmera (algumas
   câmeras limitam sessões). Gravação por evento pode sair de (1) sem decodificar (ADR 0007).
C. **Um pipeline por câmera com `tee` no stream codificado**: antes do decoder, `tee` alimenta (i) decoder de display
   (ligado só quando visível) e (ii) decoder de detecção em baixa taxa (`I-frames only` via `skip-frame`/`drop-only`,
   se o decoder suportar) e (iii) remux para gravação. Uma conexão por câmera; mais complexo, muda a estrutura do `pipeline.rs`.

## Recomendação (a validar no spike do M0)
Começar por **A** para entregar M1 (movimento/zonas) rápido, com `pause_hidden` desligado quando `[motion]` estiver ativo na câmera.
Medir no baseline. Migrar para **B** em M2 se o custo de A com 16+ câmeras estourar o orçamento.
Registrar o limite de sessões RTSP por câmera antes de escolher B.

## Consequências
- Define o desenho de `sync_active_streams`, `drain_start_queue` e do `GStreamerBridge`.
- Afeta ADR 0005 (onde fica o ramo de detecção), 0007 (de onde sai a gravação) e 0002 (custo de decodificação).

## Atualização (2026-09-24): 16 câmeras
- No preset `4x4` todas as 16 células estão visíveis: todas decodificam de qualquer forma, então `pause_hidden` pouco economiza.
  Para o caso de uso principal (16 câmeras, gravação/detecção contínuas), o custo de decodificar 16 main streams é o risco central.
- Recomendação mantida: **A em M1**, medir com `scripts/baseline.sh` em 16 câmeras (main vs sub). Se >orçamento, **B (ou C)** em M2.
- Pergunta restante ao dono: as câmeras aceitam 2 sessões RTSP simultâneas (necessário para B)? Tem sub-stream configurável?

## Atualização (2026-09-24): sessões RTSP
As câmeras têm sub-stream, mas o limite de sessões é desconhecido. Rodar `scripts/check_rtsp_sessions.sh <url> [n]` para medir
antes de escolher a opção B. Até lá, valem A em M1.
