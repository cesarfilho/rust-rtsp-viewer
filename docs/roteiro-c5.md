# Roteiro C5 / B3: conferir na tela (≈ 10 min)

Tudo abaixo roda dentro do diretório do projeto; nada abre janela sozinho.

## 0. Uma vez só
```bash
scripts/fetch-model.sh                 # modelo 640 + libonnxruntime em ./models (fora do git)
make bin FEATURES=detect               # janela, daemon (com detecção) e rrvctl em ./bin/
```
No `config.toml`, acrescente (a Intelbras já usa a senha do chaveiro):
```toml
[motion]
enabled = true

[detect]
enabled = true
model = "models/yolo11n-640.onnx"
labels = ["person", "car"]
```

## 1. Subir
```bash
make run-daemon-detect                 # terminal 1: o daemon (deve logar "detecção ligada: … (640 px)")
./bin/rust-rtsp-viewer config.toml     # terminal 2: a janela (conecta ao daemon sozinha; o chip diz "Daemon · conectado")
```

## 2. O que olhar
**iced 0.14 (B3)**
1. O vídeo **não pisca** (grade e spotlight).
2. Os 5 temas (`⋯` → Aparência): nada lavado nem ilegível.
3. Grade, menu `⋯`, barra lateral; busca (`/`) sem disparar atalhos; editor de zonas (clique direito → Zonas); `Ctrl+Q`.

**Caixas (C5)**
4. Selecione a Intelbras, tecle `f` (spotlight) e passe na frente da câmera: aparece um retângulo com "pessoa 86%".
   Observe: a caixa cobre a pessoa? a etiqueta está legível? o atraso (≈ 1 s) incomoda?
5. Tecle `t` (Gravações): o botão **Objeto: todos** aparece depois da primeira detecção; clique para filtrar; clique numa linha
   "Garagem · pessoa 87%" e veja a caixa no player.

**Inglês**
6. `⋯` → Aparência → **English**: menus e avisos em inglês, "person 86%".

## 3. Me diga
O número do passo e o que viu (um print da janela ajuda). Se o vídeo piscar ou as cores saírem erradas, volto a master ao
iced 0.13: `git switch iced-0.13-final`.
