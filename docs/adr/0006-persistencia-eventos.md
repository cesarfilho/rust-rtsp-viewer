# 0006 — Persistência de eventos
**Status:** Proposta

## Contexto
`EventTimeline` é só memória. Precisamos de histórico consultável por câmera/intervalo, ligado aos segmentos gravados, para review, playback e retenção.

## Decisão
SQLite via `rusqlite` (0.40.x consultado; usar feature `bundled` para evitar dependência do sistema nos três SOs).
Tabelas mínimas: `events(id, camera, ts_start, ts_end, kind, label, score, zone, segment_id)` e `segments(id, camera, path, ts_start, ts_end, has_motion, bytes)`.
Migrações com `PRAGMA user_version`. Caminho do banco no mesmo diretório de estado do `view.toml`.

## Alternativa considerada
JSONL append-only: zero dependência e simples, mas consultas por intervalo e limpeza por retenção exigem varrer tudo.

## Consequências
- Retenção e limpeza de disco (M3) viram queries sobre `segments`.
- Escrever em thread dedicada (nunca na UI); banco em modo WAL.

## Revisão (2026-09-24)
- `rusqlite` com feature `bundled` compila SQLite em C: precisa de compilador C nos três SOs (MSVC no Windows). Alternativas sem C: JSONL ou `redb` (não avaliado).
- Definir consistência entre banco e arquivos: se o processo cair após criar o segmento e antes do `INSERT`, há arquivo sem registro; prever varredura de reconciliação na inicialização.
- Retenção deve apagar arquivo **e** linha de forma atômica (arquivo primeiro, depois linha; ou marcar `deleted`).
- Não colocar o banco em diretório sincronizado (o projeto vive no Dropbox; o estado fica em `~/.local/state`, ok — manter fora de pastas sincronizadas).
