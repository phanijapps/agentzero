# z-bot — native install for Raspberry Pi / Linux
#
# Build, install, and manage the zbot daemon as a systemd user service.
# All paths are user-scoped XDG; no sudo required.

PREFIX     ?= $(HOME)/.local
BIN_DIR    ?= $(PREFIX)/bin
DIST_DIR   ?= $(PREFIX)/share/zbot/dist
UNIT_DIR   ?= $(HOME)/.config/systemd/user

# UI dist output is at the repo root per apps/ui/vite.config.ts (outDir: "../../dist")
UI_BUILD_DIR := dist

# Read the workspace version from Cargo.toml. The first `version = "..."`
# line in the file is `[workspace.package].version` — every other crate
# inherits it via `version.workspace = true`.
VERSION := $(shell awk -F\" '/^version[[:space:]]*=/ {print $$2; exit}' Cargo.toml)
BUILD_DATE := $(shell date -u +%Y-%m-%d)
BUILD_TIMESTAMP := $(shell date -u +%Y-%m-%dT%H:%M:%SZ)

# Resolve a vault path that mirrors the daemon's runtime resolution
# (`dirs::document_dir().or_else(home_dir).join("zbot")`). Used for the
# `logs` target and the install banner. Equivalent shell test of whether
# `~/Documents/` exists.
VAULT_DIR := $(if $(wildcard $(HOME)/Documents/.),$(HOME)/Documents/zbot,$(HOME)/zbot)

.PHONY: build install install-build uninstall start stop restart status logs clean help

help:
	@echo "z-bot Makefile targets (version: $(VERSION)):"
	@echo "  make build      - cargo build --release && npm run build"
	@echo "  make install    - install daemon + UI + systemd unit, enable + start"
	@echo "  make uninstall  - stop + disable + remove installed files (preserves user data)"
	@echo "  make start      - systemctl --user start zbot"
	@echo "  make stop       - systemctl --user stop zbot"
	@echo "  make restart    - systemctl --user restart zbot"
	@echo "  make status     - systemctl --user status zbot"
	@echo "  make logs       - tail -F $(VAULT_DIR)/logs/*.log"
	@echo "  make clean      - cargo clean + rm -rf $(UI_BUILD_DIR)"

build:
	ZBOT_BUILD_DATE=$(BUILD_DATE) ZBOT_BUILD_TIMESTAMP=$(BUILD_TIMESTAMP) cargo build --release
	cd apps/ui && npm install && npm run build

# Same as `build` but exports `ZBOT_INSTALL=1` so the daemon's and CLI's
# `build.rs` capture the current branch and append it to the embedded
# version. Resulting binary reports e.g. `2026.5.3.develop` instead of
# the bare `2026.5.3` a plain `cargo build` produces.
install-build:
	ZBOT_INSTALL=1 ZBOT_BUILD_DATE=$(BUILD_DATE) ZBOT_BUILD_TIMESTAMP=$(BUILD_TIMESTAMP) cargo build --release
	cd apps/ui && npm install && npm run build

install: install-build
	install -d $(BIN_DIR) $(DIST_DIR) $(UNIT_DIR) $(VAULT_DIR)/logs
	install -m 755 target/release/zbotd $(BIN_DIR)/zbotd
	install -m 755 target/release/zbot $(BIN_DIR)/zbot
	rm -rf $(DIST_DIR)/* && cp -r $(UI_BUILD_DIR)/* $(DIST_DIR)/
	@sed 's|@@BIN@@|$(BIN_DIR)/zbotd|g; s|@@DIST@@|$(DIST_DIR)|g; s|@@LOG_DIR@@|$(VAULT_DIR)/logs|g; s|@@VERSION@@|$(VERSION)|g' \
	    scripts/zbot.service.in > $(UNIT_DIR)/zbot.service
	@# Migrate from the legacy `agentzero.service` if present.
	@if systemctl --user is-enabled agentzero.service >/dev/null 2>&1; then \
	    echo "Migrating from agentzero.service → zbot.service"; \
	    systemctl --user disable --now agentzero.service || true; \
	    rm -f $(UNIT_DIR)/agentzero.service; \
	fi
	@if command -v loginctl >/dev/null 2>&1; then \
	    loginctl enable-linger "$(USER)" || true; \
	fi
	systemctl --user daemon-reload
	systemctl --user enable --now zbot
	@echo ""
	@echo "Installed z-bot $(VERSION). Status: systemctl --user status zbot"

uninstall:
	-systemctl --user stop zbot
	-systemctl --user disable zbot
	rm -f $(UNIT_DIR)/zbot.service
	rm -f $(BIN_DIR)/zbotd
	rm -f $(BIN_DIR)/zbot
	rm -rf $(PREFIX)/share/zbot
	@# Best-effort cleanup of legacy install artifacts.
	-systemctl --user stop agentzero 2>/dev/null
	-systemctl --user disable agentzero 2>/dev/null
	-rm -f $(UNIT_DIR)/agentzero.service
	-rm -f $(BIN_DIR)/zerod
	-rm -f $(BIN_DIR)/zero
	-rm -rf $(PREFIX)/share/agentzero
	systemctl --user daemon-reload
	@echo ""
	@echo "Uninstalled. User data in $(VAULT_DIR) is preserved."

start:
	systemctl --user start zbot

stop:
	systemctl --user stop zbot

restart:
	systemctl --user restart zbot

status:
	systemctl --user status zbot

logs:
	@tail -F $(VAULT_DIR)/logs/*.log

clean:
	cargo clean
	rm -rf $(UI_BUILD_DIR)
