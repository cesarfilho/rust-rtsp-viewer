# rust-rtsp-viewer: atalhos para o dia a dia. `make` sozinho mostra esta lista.
#
# Variáveis que valem a pena conhecer (passe na linha: `make status SOCKET=/tmp/rrv-a8/run/rrv.sock`):
#   CONFIG   config.toml da janela / do daemon local        (padrão: config.toml)
#   SOCKET   socket do daemon que o rrvctl e a janela usam  (padrão: o do compose.yaml)
#   CAM      nome da câmera para `make record|clip`
#   HORAS    janela do `make history`                       (padrão: 24)
#   DE       início do `make clip` (ex.: -30m, -2h)       (padrão: -5m)

.DEFAULT_GOAL := help
.PHONY: help all build release bin run-daemon-detect run run-embedded run-daemon-local run-with-daemon check ci fmt fmt-check lint test test-lib \
        deny clean watch config-check install uninstall \
        docker-build up up-vaapi up-nvidia down restart logs ps init-docker \
        status history record enable disable clip events \
        gpu-check a8 a8-window a8-embedded a8-stop stress baseline

CONFIG ?= config.toml
SOCKET ?= $(or $(XDG_RUNTIME_DIR),/run/user/$(shell id -u))/rrv/rrv.sock
HORAS  ?= 24
DE     ?= -5m
COMPOSE ?= docker compose
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
IMAGE   ?= rust-rtsp-viewer/rrv-daemon
RRVCTL   = cargo run -q --release -p rrv-daemon --bin rrvctl -- --socket $(SOCKET)

# XDG user prefix: sem root. Troque com `make install PREFIX=/usr/local`.
PREFIX ?= $(HOME)/.local
BINDIR  = $(DESTDIR)$(PREFIX)/bin
APPDIR  = $(DESTDIR)$(PREFIX)/share/applications
ICONDIR = $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps
APP_ID  = rust-rtsp-viewer

help: ## Mostra esta lista
	@echo "rust-rtsp-viewer $(VERSION)  (variáveis: CONFIG=$(CONFIG)  SOCKET=$(SOCKET))"
	@awk 'BEGIN{FS=":.*## "} /^##@/{printf "\n\033[1m%s\033[0m\n", substr($$0,5); next} /^[a-zA-Z0-9_.-]+:.*## /{printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)
	@echo

##@ Compilar e verificar
all: build ## Igual a `make build`

build: ## Compila tudo (debug)
	cargo build --workspace

release: ## Compila tudo otimizado (janela, daemon e rrvctl em target/release/)
	cargo build --release --workspace

bin: ## Compila e deixa os executáveis em ./bin/ (rust-rtsp-viewer, rrv-daemon, rrvctl); `make bin FEATURES=detect` liga a detecção de objetos
	cargo build --release --workspace $(if $(FEATURES),--features rrv-daemon/$(FEATURES),)
	mkdir -p bin
	cp target/release/rust-rtsp-viewer target/release/rrv-daemon target/release/rrvctl bin/
	@echo "pronto: ./bin/rust-rtsp-viewer  ./bin/rrv-daemon  ./bin/rrvctl"

check: fmt-check lint deny ## fmt + clippy + cargo deny (rápido, sem testes)

ci: check test ## Tudo que o CI roda: fmt, clippy, deny e a bateria completa de testes

fmt: ## Formata o código
	cargo fmt --all

fmt-check: ## Confere a formatação (falha se faltar `make fmt`)
	cargo fmt --all --check

lint: ## clippy com avisos como erro
	cargo clippy --workspace --all-targets -- -D warnings

deny: ## Licenças, vulnerabilidades (RustSec) e fontes
	cargo deny check

test: ## Bateria completa (inclui os testes que rodam GStreamer de verdade, ~1 min)
	cargo test --workspace

test-lib: ## Só os testes de biblioteca (rápido)
	cargo test --workspace --lib

config-check: ## Valida o config.toml sem abrir janela (senhas mascaradas)
	cargo run -q --release -- --check $(CONFIG)

clean: ## Apaga target/
	cargo clean

watch: ## Recompila e reabre a janela a cada mudança (precisa do cargo-watch)
	cargo watch -x run

##@ Rodar
run: ## Abre a janela (motor local, usa o config.toml)
	cargo run --release -- $(CONFIG)

run-embedded: ## Igual a `run`, forçando o motor local mesmo que haja um daemon
	cargo run --release -- --embedded $(CONFIG)

run-daemon-local: ## Roda o daemon direto, sem Docker (grava e detecta com o config.toml)
	cargo run --release -p rrv-daemon -- $(CONFIG)

run-daemon-detect: ## Daemon com detecção de objetos (precisa de `make bin FEATURES=detect` e scripts/fetch-model.sh)
	ORT_DYLIB_PATH=$(CURDIR)/models/onnxruntime/lib/libonnxruntime.so ./bin/rrv-daemon $(CONFIG)

run-with-daemon: ## Abre a janela ligada ao daemon em o socket do daemon (a vista Gravações, tecla t)
	RRV_SOCKET=$(SOCKET) cargo run --release -- $(CONFIG)

##@ Docker (o daemon que grava com a janela fechada)
init-docker: ## Cria as pastas e o config.docker.toml a partir do exemplo (só se faltarem)
	mkdir -p recordings state secrets "$${XDG_RUNTIME_DIR:-/run/user/$$(id -u)}/rrv"
	@[ -f config.docker.toml ] || { cp config.docker.toml.example config.docker.toml; echo "criado config.docker.toml: edite as câmeras"; }
	@echo "Segredos: printf '%s' 'senha' > secrets/cam_portao_password && chmod 600 secrets/cam_portao_password"

docker-build: ## Constrói a imagem (tags :local e :a versão)
	DOCKER_BUILDKIT=1 docker build -t $(IMAGE):local -t $(IMAGE):$(VERSION) .

up: ## Sobe o daemon
	$(COMPOSE) up -d

up-vaapi: ## Sobe o daemon decodificando na iGPU Intel (VA-API)
	$(COMPOSE) -f compose.yaml -f compose.vaapi.yaml up -d

up-nvidia: ## Sobe o daemon com a GPU NVIDIA (precisa do toolkit: `make gpu-check`)
	$(COMPOSE) -f compose.yaml -f compose.nvidia.yaml up -d

down: ## Para o daemon (finaliza as gravações antes)
	$(COMPOSE) down

restart: ## Reinicia o daemon
	$(COMPOSE) restart

logs: ## Acompanha o log do daemon
	$(COMPOSE) logs -f --tail=100

ps: ## Estado do contêiner
	$(COMPOSE) ps

##@ Falar com o daemon (rrvctl)
status: ## Câmeras, estado, resolução, fps e decodificador
	$(RRVCTL) status

history: ## Gravações e eventos das últimas HORAS horas (CAM=nome para uma câmera)
	$(RRVCTL) history $(CAM) --hours $(HORAS)

record: ## Liga/desliga a gravação de uma câmera: make record CAM=Garagem
	@test -n "$(CAM)" || { echo "use: make record CAM=nome-da-câmera"; exit 1; }
	$(RRVCTL) record "$(CAM)"

enable: ## Liga uma câmera: make enable CAM=Garagem
	@test -n "$(CAM)" || { echo "use: make enable CAM=nome-da-câmera"; exit 1; }
	$(RRVCTL) enable "$(CAM)"

disable: ## Desliga uma câmera: make disable CAM=Garagem
	@test -n "$(CAM)" || { echo "use: make disable CAM=nome-da-câmera"; exit 1; }
	$(RRVCTL) disable "$(CAM)"

clip: ## Exporta um clipe .mp4: make clip CAM=Garagem DE=-10m
	@test -n "$(CAM)" || { echo "use: make clip CAM=nome-da-câmera [DE=-10m]"; exit 1; }
	$(RRVCTL) export "$(CAM)" -- $(DE)

events: ## Acompanha os eventos em tempo real (Ctrl+C sai)
	$(RRVCTL) events

##@ GPU
gpu-check: ## Confere o host para a GPU NVIDIA e diz o comando que faltar (só lê)
	scripts/check-nvidia-host.sh $(IMAGE):local

##@ Verificação manual (roteiro A8, docs/roteiro-a8.md)
a8: ## Sobe o daemon de demonstração com a sua Intelbras e grava 4 trechos
	scripts/a8-demo.sh start $(CONFIG)

a8-window: ## Abre a janela ligada ao daemon de demonstração
	set -a; . /tmp/rrv-a8/env; set +a; RRV_SOCKET=/tmp/rrv-a8/run/rrv.sock ./target/release/$(APP_ID) /tmp/rrv-a8/window.toml

a8-embedded: ## Abre a janela em motor local (passo do Ctrl+Q)
	set -a; . /tmp/rrv-a8/env; set +a; ./target/release/$(APP_ID) --embedded /tmp/rrv-a8/window.toml

a8-stop: ## Derruba e apaga o daemon de demonstração
	scripts/a8-demo.sh stop

##@ Medir
stress: ## Teste de estresse com 16 fontes sintéticas: make stress SEGUNDOS=300 (padrão 300; 3600 = o de 1 h)
	RRV_STRESS_SECS=$(or $(SEGUNDOS),300) cargo test --release --test stress -- --ignored --nocapture

baseline: ## CPU/RAM de um config no contêiner: make baseline ROTULO=x CFG=meu.toml [ENVFILE=env]
	@test -n "$(CFG)" || { echo "use: make baseline ROTULO=nome CFG=arquivo.toml [ENVFILE=arquivo-de-segredos]"; exit 1; }
	scripts/baseline-docker.sh "$(or $(ROTULO),teste)" $(CFG) 40 40 $(ENVFILE)

##@ Instalar no desktop (Wayland/COSMIC/GNOME)
# Instala o binário de release, o .desktop e o ícone, para o compositor mostrar nome e ícone no dock. O app_id de
# src/ui/mod.rs (APP_ID) casa com o nome do .desktop e com o StartupWMClass.
install: build ## Instala em ~/.local (sem root)
	cargo build --release
	install -Dm755 target/release/$(APP_ID) $(BINDIR)/$(APP_ID)
	install -Dm644 assets/$(APP_ID).desktop $(APPDIR)/$(APP_ID).desktop
	install -Dm644 assets/$(APP_ID).svg $(ICONDIR)/$(APP_ID).svg
	-update-desktop-database $(DESTDIR)$(PREFIX)/share/applications 2>/dev/null || true
	-gtk-update-icon-cache -qtf $(DESTDIR)$(PREFIX)/share/icons/hicolor 2>/dev/null || true
	@echo "Instalado $(APP_ID) em $(PREFIX). Confira se $(PREFIX)/bin está no PATH."

uninstall: ## Remove o que o install pôs
	rm -f $(BINDIR)/$(APP_ID)
	rm -f $(APPDIR)/$(APP_ID).desktop
	rm -f $(ICONDIR)/$(APP_ID).svg
	-update-desktop-database $(DESTDIR)$(PREFIX)/share/applications 2>/dev/null || true
	-gtk-update-icon-cache -qtf $(DESTDIR)$(PREFIX)/share/icons/hicolor 2>/dev/null || true
