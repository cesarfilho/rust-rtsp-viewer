# 0004 — Uso: app desktop pessoal
**Status:** Substituída em parte pelo ADR 0010 (2026-10-05): o NVR passa a ter um daemon sem janela. O resto (Iced como interface, núcleo sem iced) segue valendo.

## Decisão
Iced continua sendo a interface única. Sem serviço headless, sem API HTTP/MQTT no escopo atual.
Manter o núcleo (`domain/`, `infrastructure/`) sem dependência de `iced` para deixar a porta aberta.

## Consequências
- `axum`, `rumqttc` e `gstreamer-rtsp-server` saem do roadmap principal (ficam como ideias em `todo.md`).
- Notificações: locais (`notify-rust` ou equivalente por SO), não de rede.
- Credenciais: guardar no keyring do SO (a definir) em vez de texto puro no `config.toml`.

## Revisão (2026-09-24)
- Keyring: nenhuma crate foi avaliada ainda (`keyring` é candidata, não verificada). Manter o `config.toml` funcionando com URL completa para não quebrar quem já usa.
- Sem API/MQTT não significa sem "contrato de eventos": definir o formato de evento interno (ADR 0006) para não reescrever se a decisão mudar.
- "Desktop pessoal" com muitas câmeras + ML implica app rodando 24/7 (é um NVR): definir se fechar a janela mantém gravação/detecção (bandeja do sistema) — hoje não existe.
