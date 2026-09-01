.DEFAULT_GOAL := help
SHELL := /bin/bash

CARGO ?= cargo
BIN   := srelog
NOTES ?= $(HOME)/sre-notes
ARGS  ?=

.PHONY: help build release test fmt fmt-check lint check run install uninstall parity clean

help: ## показать цели
	@grep -E '^[a-z-]+:.*?## ' $(MAKEFILE_LIST) | awk -F':.*?## ' '{printf "  %-10s %s\n", $$1, $$2}'

build: ## debug-сборка
	@$(CARGO) build

release: ## release-сборка
	@$(CARGO) build --release

test: ## юнит-тесты
	@$(CARGO) test

fmt: ## отформатировать код
	@$(CARGO) fmt

fmt-check: ## проверить форматирование, ничего не меняя
	@$(CARGO) fmt --check

lint: ## clippy, предупреждения как ошибки
	@$(CARGO) clippy --all-targets -- -D warnings

check: fmt-check lint test ## формат, линт и тесты разом

run: ## запустить: make run ARGS="index --root /path/to/notes"
	@$(CARGO) run --quiet -- $(ARGS)

install: ## поставить бинарник в ~/.cargo/bin
	@$(CARGO) install --path . --force

uninstall: ## убрать бинарник из ~/.cargo/bin
	@$(CARGO) uninstall $(BIN)

parity: release ## сверить вывод с bash-скриптами (NOTES=<путь к sre-notes>)
	@test -d "$(NOTES)/oncall" || { echo "нет $(NOTES)/oncall — задай NOTES=<путь>" >&2; exit 1; }
	@test -x "$(NOTES)/scripts/oncall-index.sh" || { echo "в $(NOTES) больше нет bash-скриптов, сверять не с чем" >&2; exit 1; }
	@set -e; \
	tmp=$$(mktemp -d); \
	trap 'rm -rf "$$tmp"' EXIT; \
	cp -R "$(NOTES)" "$$tmp/bash"; \
	cp -R "$(NOTES)" "$$tmp/rust"; \
	( cd "$$tmp/bash" && ./scripts/oncall-index.sh && ./scripts/oncall-backlog.sh ) >/dev/null; \
	./target/release/$(BIN) --root "$$tmp/rust" index >/dev/null; \
	./target/release/$(BIN) --root "$$tmp/rust" backlog >/dev/null; \
	rc=0; \
	for f in INDEX.md BACKLOG.md; do \
	  if diff -u <(sed '3d' "$$tmp/bash/oncall/$$f") <(sed '3d' "$$tmp/rust/oncall/$$f"); then \
	    echo "  $$f: паритет"; \
	  else rc=1; fi; \
	done; \
	exit $$rc

clean: ## удалить target/
	@$(CARGO) clean
