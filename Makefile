# scarlett-tui — build, test, lint, and install targets.

PREFIX      ?= $(HOME)/.local
BINDIR      := $(PREFIX)/bin
APPDIR      := $(PREFIX)/share/applications
PLUGIN_ID   := io.github.kinggs.scarlett-tui
PLUGIN_DIR  := $(or $(XDG_CONFIG_HOME),$(HOME)/.config)/omarchy/plugins/$(PLUGIN_ID)

.PHONY: all build test lint fmt-check clippy check-toolchain check-plugin install install-plugin uninstall clean

all: build

check-toolchain:
	@pin_rt=$$(awk -F'"' '/^channel[[:space:]]*=/{print $$2; exit}' rust-toolchain.toml); \
	pin_mise=$$(awk -F'"' '/^rust[[:space:]]*=/{print $$2; exit}' mise.toml); \
	pin_cargo=$$(awk -F'"' '/^rust-version[[:space:]]*=/{print $$2; exit}' Cargo.toml); \
	if ! printf '%s' "$$pin_rt" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$$'; then \
	  echo "Invalid rust-toolchain.toml pin (expected X.Y.Z): '$$pin_rt'" >&2; \
	  exit 1; \
	fi; \
	if [ "$$pin_rt" != "$$pin_mise" ] || [ "$$pin_rt" != "$$pin_cargo" ]; then \
	  echo "Toolchain mismatch: rust-toolchain.toml ($$pin_rt), mise.toml ($$pin_mise), Cargo.toml ($$pin_cargo)" >&2; \
	  exit 1; \
	fi; \
	echo "toolchain pin ok ($$pin_rt)"

build: check-toolchain
	cargo build --release --locked

test: check-toolchain check-plugin
	cargo test --locked

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --all-targets --locked -- -D warnings

lint: check-toolchain fmt-check clippy check-plugin

check-plugin:
	@bash scripts/check-plugin.sh

install: build
	install -Dm755 target/release/scarlett-tui "$(BINDIR)/scarlett-tui"
	@mkdir -p "$(APPDIR)"
	@printf '%s\n' \
	  '[Desktop Entry]' \
	  'Version=1.0' \
	  'Name=Scarlett TUI' \
	  'Comment=Focusrite Scarlett mixer, routing matrix, live meters, and PipeWire links' \
	  'Exec=$(BINDIR)/scarlett-tui' \
	  'Terminal=true' \
	  'Type=Application' \
	  'Icon=audio-card' \
	  'Categories=AudioVideo;Audio;Mixer;' \
	  'StartupNotify=true' > "$(APPDIR)/Scarlett TUI.desktop"
	@chmod +x "$(APPDIR)/Scarlett TUI.desktop"
	@echo "Installed $(BINDIR)/scarlett-tui and portable $(APPDIR)/Scarlett TUI.desktop"

install-plugin: install check-plugin
	@if [ -e "$(PLUGIN_DIR)" ] && [ ! -L "$(PLUGIN_DIR)" ]; then \
	  echo "Refusing to overwrite non-symlink plugin checkout at $(PLUGIN_DIR)." >&2; \
	  echo "If installed via 'omarchy plugin add', remove it first with: omarchy plugin remove $(PLUGIN_ID)" >&2; \
	  exit 1; \
	fi
	@mkdir -p "$$(dirname "$(PLUGIN_DIR)")"
	ln -sfn "$(CURDIR)" "$(PLUGIN_DIR)"
	@if command -v omarchy-launch-or-focus-tui >/dev/null 2>&1; then \
	  printf '%s\n' \
	    '[Desktop Entry]' \
	    'Version=1.0' \
	    'Name=Scarlett TUI' \
	    'Comment=Focusrite Scarlett mixer, routing matrix, live meters, and PipeWire links' \
	    'Exec=omarchy-launch-or-focus-tui --app-id=TUI.float $(BINDIR)/scarlett-tui' \
	    'Terminal=false' \
	    'Type=Application' \
	    'Icon=audio-card' \
	    'Categories=AudioVideo;Audio;Mixer;' \
	    'StartupNotify=true' > "$(APPDIR)/Scarlett TUI.desktop"; \
	  chmod +x "$(APPDIR)/Scarlett TUI.desktop"; \
	fi
	@if command -v omarchy-shell >/dev/null 2>&1; then \
	  omarchy-shell shell rescanPlugins; \
	  omarchy plugin enable "$(PLUGIN_ID)" --section right; \
	fi
	@echo "Installed Omarchy plugin $(PLUGIN_ID) -> $(PLUGIN_DIR)"

uninstall:
	@if [ -L "$(PLUGIN_DIR)" ]; then \
	  if command -v omarchy >/dev/null 2>&1; then \
	    omarchy plugin disable "$(PLUGIN_ID)" 2>/dev/null || true; \
	  fi; \
	  rm -f "$(PLUGIN_DIR)"; \
	  if command -v omarchy-shell >/dev/null 2>&1; then \
	    omarchy-shell shell rescanPlugins 2>/dev/null || true; \
	  fi; \
	elif [ -e "$(PLUGIN_DIR)" ]; then \
	  echo "Note: $(PLUGIN_DIR) is a directory (not a symlink). Remove it with: omarchy plugin remove $(PLUGIN_ID)" >&2; \
	fi
	rm -f "$(BINDIR)/scarlett-tui" "$(APPDIR)/Scarlett TUI.desktop"
	@echo "Uninstalled scarlett-tui binary and desktop launcher."

clean:
	cargo clean
