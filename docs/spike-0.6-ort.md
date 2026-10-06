# Spike 0.6 / C0 — ONNX Runtime: qual backend para o YOLO

Medido em 2026-10-06 nesta máquina (i7-11800H, 16 threads; iGPU Intel UHD TGL GT1; GTX 1650 4 GiB, driver 610.57).
Modelo: **YOLO11n** da Ultralytics (`yolo11n.pt`, SHA-256 `0ebbc80d4a7680d14987a577cd21342b65ecfd94632bd9a8da63ae6417644ee1`),
exportado com `model.export(format="onnx", imgsz=N, dynamic=False)` (10 MiB, opset 18, saída `1×84×8400` a 640).
Entrada aleatória `1×3×N×N` float32; 5 execuções de aquecimento, 40 medidas; mediana e p95. Python + ONNX Runtime
(o `ort` do Rust usa a mesma biblioteca; o custo do pré/pós-processamento **não** está incluído).

| Backend | 320 mediana (p95) | 640 mediana (p95) | Observação |
|---|---|---|---|
| CPU (ORT 1.30) | 26,5 ms (30,3) | 68,3 ms (74,6) | máquina ociosa; numa rodada anterior, com carga, 43,6 / 173,5 ms |
| OpenVINO, CPU (ORT 1.24) | 48,1 ms (107) | 173,2 ms (336) | mais lento que o CPU puro; sem vantagem |
| OpenVINO, iGPU Intel | **indisponível** | **indisponível** | "Device GPU is not available": falta o runtime de compute da Intel no host |
| **CUDA, GTX 1650** (ORT 1.30, CUDA 13 + cuDNN 9) | **4,6 ms (5,6)** | **9,7 ms (10,6)** | ~6× (320) e ~7× (640) mais rápido que o CPU ocioso |

A VRAM não foi medida (o processo terminou antes da leitura); fica para a C1, com a inferência de verdade.

## Recomendação

1. **CUDA como backend principal** quando houver GPU NVIDIA: 640 px em ~10 ms deixa folga para muitas câmeras
   (≈100 inferências/s numa só GPU), e libera a CPU, que já carrega o decode.
2. **CPU como fallback obrigatório** (quem não tem GPU): usar **320 px** (~27–45 ms) e inferir só com movimento (C3).
3. **Descartar OpenVINO por ora**: no CPU não ganha do ORT puro, e a iGPU exigiria `intel-compute-runtime` (sudo)
   para sequer medir. Reavaliar só se alguém precisar de inferência em máquina sem NVIDIA.
4. A imagem Docker (C7) terá duas variantes: **CPU** e **CUDA** (CUDA 13 + cuDNN 9, ~GiB; o compose já tem o
   `compose.nvidia.yaml`).

## Para reproduzir

```
uv venv --python 3.14 .venv && . .venv/bin/activate
uv pip install ultralytics onnx onnxruntime --extra-index-url https://download.pytorch.org/whl/cpu
python -c "from ultralytics import YOLO; m=YOLO('yolo11n.pt'); [m.export(format='onnx', imgsz=s) for s in (320, 640)]"
# CUDA: onnxruntime-gpu + nvidia-cudnn-cu13 nvidia-cublas nvidia-cuda-runtime nvidia-cufft nvidia-curand,
# com LD_LIBRARY_PATH apontando para os lib/ de site-packages/nvidia/*
```

Licença: o peso e o código da Ultralytics são AGPL-3.0, compatível com a licença deste projeto; reconferir no download (C1).
