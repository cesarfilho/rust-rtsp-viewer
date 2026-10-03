# Avisos de terceiros

O código deste projeto é licenciado sob **AGPL-3.0-or-later** (veja `LICENSE`).
Os componentes abaixo são distribuídos com ele ou usados por ele sob suas
próprias licenças.

## Distribuído neste repositório

### DejaVu Sans — `assets/fonts/DejaVuSans.ttf`

Embutida no binário (`src/ui/icons.rs`) para desenhar os ícones da interface
(`◉ ● ♪ ⬡ ▲ ▼ ⋯ …`), que o fallback de fontes do sistema nem sempre cobre.

Licença: Bitstream Vera Fonts Copyright + domínio público para as adições do
DejaVu — permite uso, cópia e redistribuição (inclusive embutida em software),
desde que o aviso de copyright e a licença acompanhem a fonte. O texto
completo está em `assets/fonts/DejaVu-LICENSE.txt`.

## Dependências em tempo de execução (não distribuídas aqui)

- **GStreamer** e seus plugins (`base`, `good`, `bad`, `libav`, e `ugly` para
  `x264enc`) — LGPL-2.1+, ligados dinamicamente. Alguns plugins (por exemplo
  `x264enc`, `libav`) têm restrições próprias de patente/licença; verifique-as
  antes de redistribuir binários que os incluam.

## Dependências Rust

As crates listadas em `Cargo.lock` (Iced, serde, chrono, image, …) mantêm suas
licenças originais (MIT, Apache-2.0 e similares, todas compatíveis com a AGPL-3.0).
`cargo install cargo-license && cargo license` lista cada uma.
