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
| **11 HLS, daemon sem RGBA (headless)** | **62%** | **631 MiB** | 23 Mb/s |
| **12 câmeras com movimento, headless** | **52%** | **687 MiB** | 23 Mb/s |

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

## Pré-roll (plano 3.6), memória
Intelbras 1080p (≈2–4 Mb/s) no contêiner, `on_motion = true`, com `motion_pre_roll_secs` = 0 / 5 / 30:
RSS **~77 MiB (5 s) e ~80 MiB (30 s)**; o ring de 30 s custou cerca de 3 MiB sobre o de 5 s, uma ordem de
grandeza abaixo do decodificador (~70 MiB por câmera). O pré-roll não é problema de memória com 16 câmeras
(≈ bitrate × segundos; 16 × 4 Mb/s × 5 s ≈ 40 MB no total). A rodada de 0 s não subiu a câmera (0/1 ao vivo) e
as de CPU saíram confundidas por um build em paralelo: só o RSS vale.
Limites conhecidos: o tap codificado liga o primeiro pad do `rtspsrc` (como o `decodebin` já fazia): uma
câmera cujo SDP lista o áudio antes do vídeo não funcionaria; a faixa de áudio ainda não entra no arquivo.

## GPU depois do daemon sem RGBA (2026-10-05)
11 câmeras HLS no contêiner, 40 s de aquecimento e 40 s de amostra, em sequência: decodificação por CPU (`avdec_h264`)
**88%** de CPU média (máx 125%, 625 MiB) × iGPU Intel por VA-API (`vah264dec`) **47%** (máx 70%, 709 MiB). Com o RGBA fora do
caminho, a decodificação passou a dominar e a GPU a reduz quase pela metade. Detalhes em `docs/gpu-container.md`.

## Keyframes da Intelbras (spike A2, 2026-10-06)
Ferramenta: `RRV_PROBE_URL=... cargo test -p rrv-core --test headless probe_keyframes -- --ignored --nocapture`
(mede os quadros-I por `alignment=au`, sem pedir e pedindo um *force-key-unit* para cima a cada N s).
- A GOP da câmera é **dinâmica**: com a cena parada (gravação da noite) o ring chegou a **23 s** sem keyframe; com
  movimento na sala vieram **17 keyframes em 40 s** (intervalos de 0,6 a 5,7 s).
- Pedir keyframe a cada 4 s (RTCP PLI/FIR via rtspsrc) **não mudou visivelmente** a distribuição (15 em 40 s), mas com a
  cena em movimento a linha de base já é curta: o teste só separa o efeito numa cena parada. Não construí
  `[recording] keyframe_every_secs`; refazer o spike numa noite sem movimento antes de decidir.
- Consequência para o pré-roll: ele sempre começa no keyframe anterior ao pedido, então com cena parada pode passar de 5 s
  (até o teto de 30 s), e com movimento fica perto do pedido.
