# Spec de UX — Gravações, histórico e reprodução (plano 3.1–3.6)

Estado: **aprovada em 2026-10-05** (decisões no fim). Depende do ADR 0010 (o daemon grava
sem a janela), 0006 (SQLite), 0007 (pré-captura) e da spec `ux-daemon.md`.

## Por que isto existe
O daemon grava 24 h, mas hoje o que ele grava é uma pasta de arquivos. Um NVR só vale quando a
pessoa responde rápido a: **"o que aconteceu na garagem ontem às 22h?"**, **"quanto disco
sobra?"** e **"guarde este trecho"**. Erros a evitar: achar que há gravação onde não há; perder
evento importante por limpeza de disco; ter de abrir pasta e adivinhar o arquivo.

## Modelo mental
- **Segmento:** arquivo de vídeo (1 a N minutos). Unidade de armazenamento e de retenção.
- **Evento:** algo com hora e câmera (movimento, offline, depois pessoa/veículo). Aponta para o
  segmento que o contém.
- **Clipe:** trecho exportado de um ou mais segmentos, sem reencode.
- **Modos de gravação por câmera:** *contínuo* (24 h), *por movimento* (só em torno de eventos,
  com pré-roll e pós-roll) ou *desligado*.

## Telas (na janela; tudo vem do daemon pelo canal, não de arquivos locais)

### 1. Aba "Gravações" (sidebar) e vista Linha do tempo
- **Linha do tempo por câmera**: uma faixa por câmera, eixo = hora. Trechos gravados em cinza
  claro, movimento em âmbar, evento de IA em azul, **lacunas vazias** (nunca preenchidas). Zoom
  de 15 min a 7 dias (roda do mouse / `+` `-`), arrastar para mover.
- **Hoje** aberto por padrão, com cursor na hora atual. Atalho `t` abre a vista; `Esc` volta.
- Clicar numa faixa posiciona o cursor e **abre o vídeo naquele instante** no painel acima.
- Lista de eventos ao lado (filtro por câmera, tipo e intervalo); clicar em um evento leva o
  cursor a **pré-roll antes** do evento.

### 2. Reprodução
- Painel de vídeo (o mesmo da spotlight) com controles: play/pausa (`Espaço`), ±10 s (`←` `→`),
  velocidade 0,5×/1×2×/4× (`,` `.`), quadro a quadro (`Shift+←/→`), **Ao vivo** (`l`).
- Pula sozinho para o segmento seguinte; em lacuna, mostra *"Sem gravação entre 22:10 e 22:14"*
  e avança para o próximo trecho.
- Vários canais lado a lado (até 4) sincronizados no mesmo instante, para ver o mesmo momento em
  câmeras diferentes.
- Faixa "REC" vermelha quando o instante exibido ainda está sendo gravado (borda do ao vivo).

### 3. Exportar e proteger
- Selecionar intervalo na linha do tempo (arrastar com `Shift`) → **Exportar clipe** (remux
  `.mp4`, sem reencode, rápido) para uma pasta escolhida; progresso em toast.
- **Proteger trecho** (cadeado): o trecho não é apagado pela retenção até a pessoa soltar.
  Aparece com cadeado na linha do tempo.

### 4. Retenção e disco (Configurações → Gravação e na aba)
- Barra de uso do disco: *"412 GB de 500 GB · 11 dias de histórico · ~37 GB/dia"*.
- Política: **dias** por modo (ex.: contínuo 7 dias, por movimento 30 dias) **e** limite de
  espaço; vale o que bater primeiro. Apaga o mais antigo, **nunca** protegido e **nunca** o que
  está sendo gravado.
- Aviso quando faltar < 10% de espaço ou a estimativa for < 2 dias (toast + webhook); nunca
  para de gravar em silêncio. Se o disco encher de verdade: apaga o mais antigo não protegido e
  **registra evento "disco cheio"**.

## Estados vazios, de erro e de carga
| Situação | O que aparece |
|---|---|
| Sem gravação ainda | *"Nada gravado ainda. Ative a gravação contínua ou por movimento."* + atalho para a configuração |
| Daemon perdido | Linha do tempo congela com banner da spec do daemon; histórico já carregado continua navegável |
| Arquivo apagado/ilegível | O trecho mostra "arquivo ausente" (cinza hachurado); a linha do banco é limpa na reconciliação |
| Carregando | Esqueleto da faixa; o cursor já responde |
| Disco quase cheio | Chip âmbar na barra: *"Disco 92%"* |

## Acessibilidade e tema
Cores de `ThemeColors` (novas: gravado, movimento, IA, lacuna, protegido), contraste no teste
`every_theme_meets_contrast_targets`. Estado nunca só por cor: hachura para lacuna, cadeado
para protegido, ícone para tipo de evento. Texto em pt-BR; horas no fuso local, 24 h.

## Critérios de aceite (testáveis)
1. Matar o daemon no meio de um segmento deixa o arquivo **tocável** (ou marcado como
   incompleto) e o banco consistente na reinício (reconciliação).
2. A linha do tempo mostra exatamente os segmentos do banco; lacuna aparece vazia.
3. Clicar num evento abre o vídeo em ≤ 1 s, no keyframe anterior ao evento.
4. Exportar um clipe de 30 s sem reencode leva < 3 s e toca em qualquer player.
5. Com limite de espaço atingido, o mais antigo **não protegido** é apagado (arquivo primeiro,
   linha depois) e o protegido permanece.
6. Gravação por movimento inclui o pré-roll configurado (limitado pelo GOP da câmera).
7. A janela fechada não perde nenhum evento: ao abrir, a linha do tempo tem tudo.
8. Contraste dos novos estados passa em todos os temas.

## Fora de escopo
Busca por pessoa/placa (M4), exportação para a nuvem, áudio bidirecional, vários servidores,
Windows/macOS.

## Decisões (resolvidas pelo dono em 2026-10-05)
- **D-H1 — modo padrão:** ✔ **movimento com pré-roll de 5 s**. Contínuo fica a um clique por câmera.
- **D-H2 — retenção padrão:** ✔ **somente movimento, 7 dias** (sem padrão para contínuo; quem
  ligar o contínuo define os dias). O limite de espaço continua valendo (padrão 80% do disco), e o
  que bater primeiro apaga o mais antigo não protegido.
- **D-H3 — reprodução:** ✔ **embutida na janela** (seek, velocidade, quadro a quadro).
- **D-H4 — onde ficam as gravações no Docker:** ✔ volume dedicado em `/recordings` (como hoje).
