# 0009 — Licença do projeto
**Status:** Aceita — **AGPL-3.0** (decidido pelo dono em 2026-09-24)

## Contexto
O projeto não tem arquivo LICENSE. Dono: o projeto deve ser livre (software livre/open source).
A escolha condiciona modelos de ML (ADR 0003) e dependências.

## Opções (a confirmar com o dono)
- **MIT ou Apache-2.0** (permissivas; Apache-2.0 tem concessão de patentes; comum em Rust, muitas crates usam "MIT OR Apache-2.0").
- **GPL-3.0 / AGPL-3.0** (copyleft forte): garante que derivados permaneçam livres. AGPL-3.0 seria compatível com embarcar modelos YOLO
  da Ultralytics (que costumam ser AGPL — verificar); com licença permissiva, esses modelos passam a impor AGPL ao conjunto distribuído.

## Ação
Escolher, adicionar `LICENSE` na raiz e `license = "..."` no `Cargo.toml`; rodar `cargo-deny` para checar licenças das dependências
(GStreamer é LGPL; plugins `bad`/`ugly`/`libav` têm licenças/patentes próprias — relevante para distribuir binários).

## Decisão
AGPL-3.0-or-later (a confirmar o "or-later" se preferir apenas 3.0). Motivos: mantém derivados livres, inclusive uso via rede, e é
compatível com embarcar modelos YOLO da Ultralytics (AGPL — verificar a licença do modelo escolhido).

## Consequências / a fazer
- Adicionar `LICENSE` (texto oficial da AGPL-3.0, obtido de gnu.org) na raiz e `license = "AGPL-3.0-or-later"` no `Cargo.toml` — **feito** (v0.7.0).
- Dependências permissivas (MIT/Apache-2.0) e LGPL (GStreamer) são compatíveis; `cargo-deny` no CI deve barrar dependências incompatíveis (ex.: licenças só-proprietárias).
- Cabeçalho SPDX nos arquivos é opcional; definir se será adotado.
- Copyleft forte reduz contribuições de quem quer usar em produto fechado; é uma escolha consciente do dono.
