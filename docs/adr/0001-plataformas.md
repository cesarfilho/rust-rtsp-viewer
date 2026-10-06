# 0001 — Plataformas-alvo: somente Linux
**Status:** Aceita — **revisada em 2026-10-06** (antes: Linux completo, Windows/macOS melhor esforço)

## Contexto
O projeto foi usado e testado só em Linux (Ubuntu, Arch/Omarchy). A primeira versão desta decisão previa Windows e
macOS em "melhor esforço"; na prática nenhum dos dois chegou a ter build no CI, e o motor passou a rodar como um
daemon num contêiner Linux (ADR 0010).

## Decisão
**Só Linux.** Windows e macOS não são suportados e não entram no plano. Quem quiser usar o projeto em outro sistema
roda o daemon num contêiner Linux (Docker Desktop) e fica sem a janela nativa; isso não é um alvo do projeto.

## Consequências
- O código pode usar `cfg(unix)`, sockets Unix (o canal de controle já é um), `statvfs`, XDG e `notify-send` sem
  alternativas por SO.
- Empacotamento só para Linux: AUR, AppImage/Flatpak, a imagem Docker e releases (plano D6).
- Nada de `%APPDATA%`, MSVC, `d3d11h264dec`, `vtdec` ou CoreML. As decisões de GPU (ADR 0002, 0003) valem para
  Linux: NVIDIA (`nvh264dec`/CUDA) e VA-API (Intel/AMD).
- Menos matriz de testes de GStreamer.

## Histórico
Os trechos das versões anteriores sobre Windows/macOS ficam só como referência no histórico do git.
