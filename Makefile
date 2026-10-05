.PHONY: all run test build clean watch install uninstall

# XDG user prefix — no root needed. Override with `make install PREFIX=/usr/local`.
PREFIX ?= $(HOME)/.local
BINDIR  = $(DESTDIR)$(PREFIX)/bin
APPDIR  = $(DESTDIR)$(PREFIX)/share/applications
ICONDIR = $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps
APP_ID  = rust-rtsp-viewer

all: build

build:
	cargo build --workspace

run:
	cargo run

test:
	cargo test --workspace

clean:
	cargo clean

watch:
	cargo watch -x run

# Desktop integration for Wayland/COSMIC (and GNOME): installs the release
# binary plus the .desktop entry and scalable icon so the compositor shows a
# real name and icon in the dock, overview and window switcher. The app_id set
# in src/ui/mod.rs (APP_ID) matches the .desktop basename and StartupWMClass.
install: build
	cargo build --release
	install -Dm755 target/release/$(APP_ID) $(BINDIR)/$(APP_ID)
	install -Dm644 assets/$(APP_ID).desktop $(APPDIR)/$(APP_ID).desktop
	install -Dm644 assets/$(APP_ID).svg $(ICONDIR)/$(APP_ID).svg
	-update-desktop-database $(DESTDIR)$(PREFIX)/share/applications 2>/dev/null || true
	-gtk-update-icon-cache -qtf $(DESTDIR)$(PREFIX)/share/icons/hicolor 2>/dev/null || true
	@echo "Installed $(APP_ID) to $(PREFIX). Ensure $(PREFIX)/bin is on PATH."

uninstall:
	rm -f $(BINDIR)/$(APP_ID)
	rm -f $(APPDIR)/$(APP_ID).desktop
	rm -f $(ICONDIR)/$(APP_ID).svg
	-update-desktop-database $(DESTDIR)$(PREFIX)/share/applications 2>/dev/null || true
	-gtk-update-icon-cache -qtf $(DESTDIR)$(PREFIX)/share/icons/hicolor 2>/dev/null || true
