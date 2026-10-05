# Baseline medido (plano 0.3 e 0.4)

Medido em 2026-10-05 com `scripts/baseline-docker.sh` (daemon no contêiner, sem GPU,
decodificação por CPU `avdec_h264`). Dados brutos em `docs/baseline.csv`.

## Método
Aquecimento de 25–40 s, amostra de 30 s. CPU e RSS vêm de `docker stats` (100% = 1 núcleo);
rede de `/proc/net/dev`; fps, resolução e decodificador de `rrvctl status --json`.
Câmeras: 1 Intelbras local (RTSP, 1080p principal / 480p sub) e 11 HLS públicas
(720p/1080p). Builds de produção (release); debug distorce o fps.

## Resultados
| Cenário | CPU méd | RSS | Rede |
|---|---|---|---|
| Intelbras principal, 1 sessão | 1,2% | 96 MiB | 3,5 Mb/s |
| Intelbras sub (480p) | 0,6% | 49 MiB | 1,9 Mb/s |
| Intelbras principal, 4 sessões | 2,2% | 331 MiB | 7,3 Mb/s |
| 11 HLS, sem limite de threads | 169% | 2180 MiB | 22 Mb/s |
| 11 HLS, `max-threads=1` | 153% | 805 MiB | 24 Mb/s |
| **11 HLS, `max-threads=2` (padrão)** | **143–150%** | **~940 MiB** | 24 Mb/s |
| 11 HLS, `max-threads=4` | 157% | 1113 MiB | 23 Mb/s |
| 12 câmeras, com detecção de movimento | 154% | 2369 MiB* | 24 Mb/s |

\* medido antes do limite de threads. O custo da detecção ficou dentro do ruído.

## Conclusões
- **0.3:** a Intelbras aceitou **4 sessões RTSP simultâneas** na principal (≥ 6 testadas
  pelo script). As HLS não têm limite prático (mas são de terceiros: usar com moderação).
- **Memória:** `avdec_*` abre 1 thread por núcleo; limitar a 2 corta ~57% do RSS
  (~190 → ~85 MiB por câmera) e ~12% de CPU. Ajuste: `RRV_DECODER_THREADS`.
- **GPU (VA-API):** funciona (`vah264dec`), mas não reduz CPU hoje; o custo está nas
  cópias RGBA. O ganho real é a tarefa 2.3 (NV12 + shader).
- **Ressalva:** o link da Intelbras perde ~25% dos pacotes, então o fps dela é baixo.
  É rede (cabo, Wi-Fi ou switch), não o motor.
