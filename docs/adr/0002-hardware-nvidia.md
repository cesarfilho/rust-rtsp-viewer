# 0002 — Hardware de referência: muitas câmeras + GPU NVIDIA

> **Nota (2026-10-06):** o projeto é só Linux (ADR 0001); as menções a Windows e macOS abaixo são histórico.

**Status:** Aceita (número exato de câmeras ainda a definir; ver "Em aberto")

## Contexto
Meta: 16+ câmeras simultâneas em desktop com GPU NVIDIA.

## Decisão
- Decodificação: preferir hardware quando disponível. Ordem de sondagem (verificar nomes no spike, dependem da versão do GStreamer):
  NVIDIA `nvh264dec`/`nvh265dec` (plugin `nvcodec`), Windows `d3d11h264dec`, macOS `vtdec`, Linux VA-API `vah264dec`, fallback `avdec_*`.
- Codificação de gravação: `nvh264enc` quando existir; fallback `x264enc`. Ligar `domain/hw_encoder.rs` (hoje sem uso; `start_recording` usa `x264enc` fixo).
- Sub-stream no grid é obrigatório (ver `docs/specs/motion-zones.md` e roadmap M2): decodificar 16 streams principais em 1080p é o principal risco de CPU/GPU.
- Inferência de ML: ONNX Runtime com execution provider CUDA/TensorRT em NVIDIA (ADR 0003).

## Consequências
- Diagnóstico deve mostrar qual decoder foi escolhido (hoje `pipeline.rs` só trata `avdec_h264/h265` por nome).
- O frame RGBA em CPU (`videoconvert` → appsink) continua sendo cópia obrigatória com iced; medir antes de propor DMABuf/GL.

## Em aberto
- ~~Número de câmeras~~ **Definido: 16** (2026-09-24). Ainda faltam resoluções/codecs/bitrates e o limite de sessões RTSP por câmera.
- macOS não tem CUDA: inferência lá usa outro provider (CoreML) — verificar no spike.

## Revisão (2026-09-24)
- GPUs NVIDIA de consumo historicamente **limitam sessões NVENC simultâneas** (o número depende do driver/modelo; verificar no hardware alvo).
  Gravar 16+ câmeras por evento com `nvh264enc` pode bater nesse limite → reforça gravar **sem reencodar** (ADR 0007).
- Hoje a gravação parte do RGBA já decodificado (`tee` após `capsfilter`) e reencoda com `x264enc`: perde qualidade e gasta CPU. O
  `hw_encoder` só ajuda se a gravação continuar reencodando.
- Decodificação por GPU ocupa memória de vídeo por stream; medir VRAM no baseline (`baseline.sh` já lista `gpu_mem_mb`).
- "Muitas câmeras" continua sem número: fixar (16? 32?) para dimensionar.
- macOS não tem NVIDIA/CUDA: a decisão vale para Linux/Windows; macOS usa VideoToolbox (`vtdec`).

## Dimensionamento com 16 câmeras (2026-09-24)
- Meta de referência do baseline: 16 streams. Cenários a medir: 16 × main 1080p decodificados vs 16 × sub-stream.
- Conta de sessões: com a opção B do ADR 0008, seriam até 32 sessões RTSP por 16 câmeras (16 main visíveis/gravação + 16 sub de detecção) —
  verificar se as câmeras aceitam. Com NVENC limitado, a gravação por evento deve ser remux (ADR 0007).
- O grid com 16 células (4x4 fixo) mostra todas de uma vez: `pause_hidden` deixa de ajudar nesse preset (ADR 0008).
