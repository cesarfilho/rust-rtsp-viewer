# 0003 — Detecção de objetos é meta principal
**Status:** Aceita

## Contexto
Dono definiu ML como objetivo principal, com GPU NVIDIA (ADR 0002) e três SOs (ADR 0001).

## Decisão
- Runtime: `ort` (ONNX Runtime). Versão consultada: 2.0.0-rc.13 (release candidate) → fixar versão exata e validar em spike.
  Alternativa de contingência sem dependência nativa: `tract-onnx` (mais lento).
- Modelo inicial: YOLO em 320x320 (padrão do Frigate), exportado para ONNX. Licença do modelo a verificar antes de distribuir.
- Detecção só roda quando há movimento (`domain/motion.rs`) e sobre a região recortada, como no Frigate.
- Módulo isolado (`src/detect/` ou crate própria) atrás de `feature = "detect"`; `domain/` só define tipos (`Detection { label, score, bbox }`).
- Inferência em thread(s) dedicada(s) com fila limitada e descarte do mais antigo; nunca na thread da UI nem no callback do appsink.

## Consequências
- Nova dependência nativa (libonnxruntime) → afeta empacotamento nos três SOs.
- Precisa de fixtures: imagens/vídeos com pessoas/carros e teste de regressão por IoU/score.
- Depende de: ADR 0005 (detect stream) e roadmap M1 (movimento + zonas).
- Spike obrigatório antes de M4: medir ms/inferência em 320x320 no hardware alvo.

## Revisão (2026-09-24)
- **Licença do modelo:** modelos YOLO da Ultralytics costumam ser AGPL-3.0 (verificar). O projeto não tem LICENSE definida; escolher a
  licença do projeto **antes** de embarcar modelo. Considerar modelos com licença permissiva (ex.: YOLOX, Apache-2.0 — verificar).
- **Feature flag vs meta principal:** se ML é a meta, a flag `detect` deve ser padrão em builds de release; a flag serve para dev/CI sem ONNX Runtime.
- **Providers por SO:** Linux/Windows NVIDIA → CUDA/TensorRT; Windows sem NVIDIA → DirectML; macOS → CoreML. Confirmar no spike; cada um
  muda o pacote nativo a distribuir.
- **VRAM compartilhada:** decodificação de 16+ streams + inferência competem pela mesma GPU; orçar no baseline.
- **Fila de inferência:** definir política quando a fila enche (descartar o mais antigo) e métrica de "detecções descartadas".
- Só rodar ML em câmeras onde há movimento **e** onde o pipeline está ativo (ADR 0008).

## Atualização (2026-09-24)
Com a licença AGPL-3.0 (ADR 0009), modelos YOLO da Ultralytics (AGPL) deixam de ser bloqueio de licença; continua necessário conferir a licença dos *pesos* do modelo escolhido.
