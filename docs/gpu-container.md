# Decodificação por GPU no contêiner (plano 2.5.10)

Medido em 2026-10-05. **Conclusão: funciona, mas hoje não economiza CPU.** O ganho depende do
caminho NV12 + shader (tarefa 2.3).

## Como ligar
```bash
docker compose -f compose.yaml -f compose.vaapi.yaml up -d
docker exec rrv rrvctl status   # coluna "decodificador": vah264dec (GPU) ou avdec_h264 (CPU)
docker exec rrv vainfo          # diagnóstico do driver
```
- `RRV_RENDER_GID` = o GID do grupo `render` **do host** (`getent group render | cut -d: -f3`).
- Só iGPU Intel/AMD por VA-API. A NVIDIA exige o `nvidia-container-toolkit` no host (D6) e não é
  coberta aqui.

## O que descobri no caminho
1. **O nó de renderização não é fixo.** Neste host a iGPU Intel é a `renderD129` e a NVIDIA é a
   `renderD128` (o `vainfo` falha nela). Por isso o compose passa `/dev/dri` inteiro.
2. **No GStreamer 1.22 (Debian bookworm) o `vah264dec` tem rank `none (0)`**, contra 256 do
   `avdec_h264`: o `decodebin` nunca o escolhe sozinho. O compose define
   `GST_PLUGIN_FEATURE_RANK=vah264dec:259,vah265dec:259`. Sem o dispositivo o plugin `va` não
   registra os decodificadores, e o `decodebin` cai para a CPU sozinho (sem erro).
3. **O hardware só decodifica H.264 4:2:0 de 8 bits.** Uma fonte 4:4:4 ou de 10 bits cai para a CPU
   (corretamente), e custa muito mais. Câmeras reais enviam 4:2:0; uma fonte de teste precisa forçar
   `video/x-raw,format=I420` antes do `x264enc`.

## Medição (4 câmeras 1080p30 H.264 4:2:0 ao vivo, contêiner do daemon, movimento desligado)
| | Decodificador | CPU do contêiner |
|---|---|---|
| CPU | `avdec_h264` | 71% de um núcleo (~18% por câmera) |
| iGPU Intel (VA-API) | `vah264dec` | 74% de um núcleo |

Método: fonte sintética (`videotestsrc` → `x264enc` ultrafast 4 Mb/s → `hlssink2`), as 4 câmeras
apontando para o mesmo HLS, 6 amostras de `docker stats` depois de 10 s de aquecimento.

### Por que a GPU não reduz a CPU
A decodificação H.264 4:2:0 com `avdec_h264` já é barata. O que domina é o que vem depois: com a
GPU o quadro ainda é **baixado para a memória do sistema**, convertido para **RGBA** na CPU
(`videoconvert`) e **copiado** (~8 MiB por quadro a 1080p) para o `appsink` e o `Handle` da janela.
Mesma conclusão de `docs/status.md`: o gargalo é a conversão RGBA. Só o caminho NV12 + shader
(plano 2.3) tira esse custo; a decodificação por GPU é pré-requisito dele, não o ganho.

### Estimativa (a confirmar com câmeras reais, tarefa 0.4)
~18% de um núcleo por câmera 1080p30 em CPU ⇒ 16 câmeras ≈ 3 núcleos. Viável sem GPU.

## Armadilha da medição (para não repetir)
A primeira rodada usou uma fonte sem `format=I420` e deu **285% de um núcleo**: o `x264enc` gerou
H.264 4:4:4 de 10 bits, que é muito mais pesado e que a GPU não decodifica. O número parecia um
resultado e era um artefato da fonte. Sempre conferir o formato/perfil da fonte de teste e a coluna
"decodificador" do `rrvctl status`.
