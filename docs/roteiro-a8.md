# Roteiro A8: verificação com mouse (≈ 15 min)

O que ainda **nunca foi usado com mouse real**: arrastar a barra, o botão Comparar, o `Ctrl+Q` com gravação local e as cores
nos 5 temas. Os testes automáticos cobrem a lógica; aqui você confere o que se vê e se sente.

## Preparação (1 comando, ≈ 1 min)
```bash
scripts/a8-demo.sh                      # sobe o daemon e grava 4 trechos (CamA e CamB juntas, com uma lacuna)
set -a; . /tmp/rrv-a8/env; set +a
RRV_SOCKET=/tmp/rrv-a8/run/rrv.sock ./target/release/rust-rtsp-viewer /tmp/rrv-a8/window.toml
```
Usa a sua Intelbras do `config.toml` duas vezes (como CamA e CamB). A senha vai só para `/tmp/rrv-a8/env` (0600).
Ao terminar: `scripts/a8-demo.sh stop`. Obs.: o `rrvctl history` dentro do Docker mostra a hora em UTC; a janela, em hora local.

## Passos (marque ok ou anote o que viu)

### 1. Abrir a vista Gravações
1. Na janela, o chip da barra deve dizer **● Daemon · conectado**. Aperte **`t`** (ou `⋯` → Gravações).
2. Deve abrir a vista cheia: título "Gravações", botões `1 h  6 h  24 h  7 d  ‹  ›`, a lista **Eventos** à direita e
   **duas faixas** (CamA, CamB) embaixo. Os trechos aparecem como barrinhas no extremo direito (escala de 6 h).
3. Clique em **`1 h`**: as barras ficam mais largas. **Roda do mouse** sobre as faixas: aproxima/afasta. `‹ ›` deslocam.

### 2. Tocar e arrastar (o que mais quero saber)
4. Clique **em cima de uma barra azul** da CamA. O vídeo deve abrir e tocar; o texto de baixo mostra `CamA · hora · 00:03 / 00:11`.
5. **Segure o botão** do mouse dentro da barra e **arraste devagar** para a esquerda e para a direita.
   **Esperado:** a imagem acompanha o cursor, sem travar e sem piscar preto; a linha branca (cabeça de reprodução) segue o mouse.
6. Continue arrastando **para fora da barra** (para a lacuna entre os dois trechos) e **para a outra barra**.
   **Esperado:** na lacuna aparece o aviso "Sem gravação aqui; indo para o próximo trecho" **uma vez** (não a cada movimento) e o
   vídeo passa para o trecho seguinte.
7. Solte o botão. Aperte **Espaço** (pausa), de novo (toca), **`→`** (+10 s), **`←`** (−10 s), botão **Quadro**, **2×** e **0,5×**.

### 3. Comparar câmeras lado a lado
8. Clique em **"Comparar: CamB"** (abaixo da imagem). **Esperado:** dois quadros lado a lado, "CamA (principal)" e "CamB", no mesmo
   instante (o relógio gravado no vídeo das duas deve bater).
9. **Espaço** e **2×** valem para os dois. Arraste a barra de novo: os dois acompanham.
10. Clique na **faixa da CamB**: ela vira a principal e a CamA continua ao lado. Clique em "Comparar: CamA" para tirar.
11. Clique num ponto onde uma câmera **não gravou** (na lacuna): o quadro dela mostra "Sem gravação neste instante".

### 4. Clipe, proteção e eventos
12. Com um vídeo tocando: **`I`** no ponto de início, deixe tocar uns 5 s, **`O`** no fim (a faixa fica sombreada de verde), **`E`**.
    **Esperado:** aviso "Exportando o clipe…" e depois "Clipe salvo: exports/…" (clicar abre a pasta). Confira o arquivo:
    `ls /tmp/rrv-a8/data/exports/` e abra o `.mp4` no seu player (deve tocar, sem reencode, ~5 s).
13. **`P`** (ou o botão Proteger): aparece um traço claro no topo da barra e o botão vira "Soltar". `P` de novo solta.
14. Na lista **Eventos**: clique em "Gravação iniciada". **Esperado:** toca a gravação **5 s antes** daquele instante.
    Clique em **"Só movimento"** (a lista deve esvaziar se não houve movimento).
15. **`Esc`** fecha a vista e volta à grade.

### 5. Ctrl+Q
16. **Com o daemon** (a janela atual): `Ctrl+Q` deve fechar **direto**, sem pergunta (o daemon segue gravando).
17. Abra de novo em **motor local**, sem daemon:
    `set -a; . /tmp/rrv-a8/env; set +a; ./target/release/rust-rtsp-viewer --embedded /tmp/rrv-a8/window.toml`
    O chip deve dizer **○ Motor local**. Selecione uma câmera, aperte **`r`** (grava) e depois **`Ctrl+Q`**.
    **Esperado:** abre uma confirmação ("Há 1 gravação em curso…"). **Cancelar** mantém a janela; **Sair** fecha e a gravação
    termina inteira (o arquivo em `/tmp/rrv-a8/data` toca).

### 6. Cores nos 5 temas
18. `⋯` → **Aparência**: percorra **Cosmic, Dark, Light, Amoled, OpenCode**. Em cada um, olhe: a grade, o menu `⋯`, a vista
    Gravações (**azul** = gravação, **âmbar** = movimento, **vermelho** = gravando agora; o texto das barras e dos botões legível).
    **Esperado:** nada ilegível e nenhuma cor "lavada" (o iced 0.14, na fase B, é que podia clarear as cores; a 0.13 atual não).

## O que me mandar
Uma linha por passo que **não** ficou como o esperado (número do passo + o que você viu). Se tudo estiver ok, "A8 ok".
Os mais prováveis de dar problema: **5–6** (arrastar), **10** (trocar a principal) e **17** (confirmação do `Ctrl+Q`).
