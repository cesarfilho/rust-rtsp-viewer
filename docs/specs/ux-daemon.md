# Spec de UX — A janela e o daemon (plano 2.5.7)

Estado: **aprovada em 2026-10-05** (as três decisões abertas foram resolvidas como recomendado;
ver o fim). Implementação em andamento (plano 2.5.7). Depende do
ADR 0010 e do canal da 2.5.5 (`rrv_core::ipc`, já pronto). Decisões que dependem do dono estão
marcadas **[D]** no fim, com a recomendação.

## Por que isto existe
Com o ADR 0010 o NVR grava e detecta **sem a janela**. A janela passa a ter dois modos, e o
usuário precisa saber, num relance, **quem está gravando** e **o que acontece se fechar a janela**.
Os erros a evitar: achar que está gravando quando não está; gravar em dobro (janela e daemon);
perder gravação ao fechar a janela sem aviso.

## Os modos da janela
| Modo | Quando | Quem grava e detecta | Chip na barra |
|---|---|---|---|
| **Motor local** | Não há daemon no socket ao abrir (uso de hoje) | A própria janela | `Motor local` (cinza) |
| **Conectando** | Há socket; o `Hello` está em andamento (< 2 s) | — | `Daemon · conectando…` (âmbar) |
| **Conectado** | `Hello` ok e assinatura de eventos ativa | O daemon. A janela só mostra e comanda | `Daemon · conectado` (verde) |
| **Perdido** | Estava conectado e o canal caiu | O daemon, se ainda vivo; **a janela não sabe** | `Daemon · sem resposta` (âmbar, pulsa) |
| **Incompatível** | `Hello` recusado por versão do protocolo | Nenhum dos dois é assumido | `Daemon · versão incompatível` (vermelho) |
| **Sem permissão** | O socket existe mas não abre (outro usuário) | — | `Daemon · sem permissão` (vermelho) |

Escolha do modo ao abrir: `--embedded` força o motor local; `--daemon <socket>` (ou `RRV_SOCKET`)
aponta o socket; sem nada, tenta o padrão (`$XDG_RUNTIME_DIR/rrv/rrv.sock`) e, **se não existir,
usa o motor local sem perguntar** — quem nunca instalou o daemon não vê diferença nenhuma.
A janela **nunca** sobe um daemon sozinha nem liga o Docker.

## Regra de ouro: um só dono da gravação
No modo **Conectado**, a janela **não** grava, **não** detecta movimento e **não** notifica: o
daemon faz. Senão cada gravação sairia em dobro e o aviso de movimento chegaria duas vezes.
Para mostrar vídeo, a janela decodifica por conta própria só para **exibir** (modo
*display-only* do motor), usando o sub-stream quando existe. Isso abre **uma sessão RTSP extra** por
câmera visível, até a 2.5.4 (o daemon redistribuir o vídeo; decisão D5). Está medido como
custo conhecido, não esquecido.

## O que a pessoa vê

### 1. Chip do daemon (barra, à esquerda do medidor de saúde)
Um chip com bolinha de status e rótulo curto (tabela acima). **Clicar abre o menu de comandos
único** (`menu::command_menu`, o mesmo do `⋯` e do botão direito), com:
- linha de estado em texto corrido: *"Conectado ao rrv-daemon 0.8.0 · 4 câmeras · gravando 2"*;
- **Reconectar agora** (em Perdido/Sem permissão);
- **Usar o motor local** (em Perdido e Incompatível, ver abaixo);
- **Copiar comando para iniciar o daemon** (`docker compose up -d`);
- **Caminho do socket** (somente leitura, copia ao clicar).

O chip nunca some: quem usa só o motor local vê `Motor local`, que também explica no menu o que
significa e como ter gravação 24 h.

### 2. Gravação: a verdade por câmera
- O selo **REC** de cada célula, a bolinha da sidebar e o contador da barra passam a refletir o
  estado **do daemon** (`recording`), não o da janela.
- O medidor de saúde da barra conta o que o daemon diz (`live`/`recording`), não o que a janela
  decodifica.
- Novo texto no chip de saúde quando há gravação: `4/4 · ● 2 gravando`.

### 3. Perdi o daemon
- Banner fino sob a barra, âmbar, **que não cobre o vídeo**: *"Sem resposta do daemon. As câmeras
  podem não estar gravando. Tentando reconectar…"* + botões **Reconectar agora** e **Usar motor
  local**.
- A janela tenta de novo com espera crescente (1, 2, 4, 8 s, no máximo 15 s). Ao voltar: o banner
  some, um aviso breve (*"Daemon reconectado"*) e o estado é recarregado (`status`).
- As células mantêm o último quadro com a faixa *"sem dados"*; **nada é apagado**.
- **Não há troca automática para o motor local.** Se o daemon só perdeu o canal mas continua
  gravando, subir um motor local gravaria em dobro. Quem decide é a pessoa, pelo botão.
- **Usar motor local** pede confirmação: *"Isto começa a gravar daqui. Se o daemon ainda estiver
  gravando, haverá gravações em dobro. Continuar?"*

### 4. Fechar a janela
- **Modo Conectado:** fecha em silêncio. O chip do próximo aviso lembra: *"As gravações continuam
  no daemon."* (uma vez por sessão, no toast de saída do `Ctrl+Q`).
- **Motor local com gravação em curso:** `Ctrl+Q` (e fechar a janela) abre uma confirmação:
  *"Há N gravações em curso. Sair as interrompe. Para gravar com a janela fechada, use o daemon."*
  **[Sair mesmo assim]** **[Cancelar]**. Hoje sair finaliza os segmentos; a confirmação evita a
  surpresa de perder o que ainda deveria estar gravando.
- **Motor local sem gravação:** fecha direto.

### 5. Eventos e notificações
- A aba **Eventos** passa a ser alimentada pela assinatura do daemon (`WireEvent`), com o mesmo
  visual de hoje. Eventos ocorridos com a janela fechada **não** aparecem até a persistência
  (M3, plano 3.2): a spec do histórico trata disso. Até lá a aba mostra só o que chegou desde que
  a janela conectou, e diz isso: *"Mostrando eventos desde que a janela conectou."*
- Notificação de desktop: **só a janela notifica, só enquanto aberta**, e apenas pelo `notification`
  que o daemon já decidiu (política e cooldown são do motor). Com a janela fechada, o aviso é o
  webhook/MQTT (2.5.11), fora do escopo desta spec.

### 6. Zonas de movimento
O editor de zonas **não muda visualmente**. Ao salvar, em modo Conectado a janela envia `set_zones`
ao daemon (que persiste e passa a usar na hora); em motor local grava `zones.toml` como hoje. Falha
(daemon perdido, zona inválida) → toast de erro com o motivo e **o editor continua aberto com o
desenho intacto**.

### 7. Comandos que a janela dá
Gravar/parar (`r`), ligar/desligar câmera e zonas viram pedidos ao daemon. O botão fica em estado
*"aguardando…"* até a resposta (máx. 5 s). Erro → toast com a mensagem do daemon, estado volta ao
anterior. **Nunca** mostrar `REC` antes da confirmação.

## Estados vazios, de erro e de carga
| Situação | O que aparece |
|---|---|
| Daemon sem câmeras | Células vazias com *"O daemon não tem câmeras configuradas"* e como configurar |
| Conectando | Chip âmbar; o conteúdo da janela já aparece (motor local não é iniciado) |
| Versão incompatível | Banner vermelho persistente: *"A janela (protocolo N) e o daemon (M) não combinam. Atualize um dos dois."* + **Usar motor local** |
| Sem permissão | *"Sem permissão para o socket do daemon (é de outro usuário?)"* + caminho |
| Daemon sem câmera X | Célula no estado *Offline* normal, com o rótulo vindo do daemon |

## Acessibilidade e tema
- Todas as cores vêm de `ThemeColors`; texto sobre chip colorido usa `Theme::readable_on`.
- O estado **nunca depende só de cor**: o chip tem rótulo de texto e a bolinha muda de forma
  (cheia, vazada, com `!`).
- Novas cores/estados entram no teste `every_theme_meets_contrast_targets` (contraste WCAG).
- Textos em pt-BR, frases curtas, verbo no imperativo nos botões.

## Teclado
Nenhum atalho novo para o dia a dia. `Esc` fecha o menu do chip e o banner de confirmação
(cascata atual). O chip é alcançável por `Tab` (foco visível) e ativa com `Enter`/`Espaço`.

## Critérios de aceite (testáveis)
1. Sem socket → abre em **Motor local**, idêntico a hoje (nenhum teste atual muda).
2. Com daemon → em ≤ 2 s o chip fica `Conectado` e `status` mostra as câmeras do **daemon**.
3. Em `Conectado`, **zero** gravação e **zero** detecção de movimento no processo da janela
   (teste: o `Engine` da janela em modo display-only não monta ramo de gravação nem de detecção).
4. `kill -STOP` no daemon → em ≤ 5 s o chip vira `sem resposta` e aparece o banner; `kill -CONT`
   → reconecta sozinho e o banner some.
5. Daemon incompatível (simulado) → banner vermelho e **nenhuma** troca automática de modo.
6. `r` mostra `REC` só depois da resposta; erro do daemon vira toast e o estado não muda.
7. `Ctrl+Q` com gravação no motor local abre a confirmação; sem gravação, fecha direto; em modo
   Conectado, fecha direto e o daemon segue gravando (teste de processo).
8. Salvar zonas em modo Conectado persiste no daemon (`get_zones` devolve) e falha com o editor
   aberto quando o daemon está perdido.
9. Contraste dos novos estados passa em todos os temas.

## Fora de escopo
Histórico persistente e reprodução (M3), vídeo redistribuído pelo daemon (2.5.4), webhook/MQTT
(2.5.11), vários daemons em hosts diferentes (o canal é só local), Windows (o canal é Unix).

## Decisões (resolvidas pelo dono em 2026-10-05)
- **D-UX1 — vídeo no modo Conectado.** ✔ Decidido: a janela decodifica só para exibir, com o
  sub-stream (uma sessão extra por câmera visível) **até** a 2.5.4. Alternativa: só começar a
  2.5.7 depois da 2.5.4.
- **D-UX2 — o que fazer ao perder o daemon.** ✔ Decidido: banner + botões, **sem** troca
  automática. Alternativa: cair sozinho no motor local após N segundos (risco de gravar em dobro).
- **D-UX3 — confirmação ao sair com gravação local.** ✔ Decidido: confirmar. Alternativa:
  sair direto (como hoje).
