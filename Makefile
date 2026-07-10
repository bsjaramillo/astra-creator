# Makefile de astra-creator
# Ejecutá `make` o `make help` para ver las tareas disponibles.

BIN      := astra-creator
CARGO    := cargo
PREFIX   ?= $(HOME)/.local

.DEFAULT_GOAL := help

## help: muestra esta ayuda
.PHONY: help
help:
	@echo "astra-creator — tareas disponibles:"
	@grep -E '^## ' $(MAKEFILE_LIST) | sed 's/## /  /'

## build: compila en modo debug
.PHONY: build
build:
	$(CARGO) build

## release: compila el binario optimizado (target/release/$(BIN))
.PHONY: release
release:
	$(CARGO) build --release
	@echo "Binario: target/release/$(BIN) ($$(du -h target/release/$(BIN) | cut -f1))"

## run: corre la TUI en el directorio actual
.PHONY: run
run:
	$(CARGO) run

## test: corre la suite de tests
.PHONY: test
test:
	$(CARGO) test

## fmt: formatea el código
.PHONY: fmt
fmt:
	$(CARGO) fmt

## lint: corre clippy con warnings como errores
.PHONY: lint
lint:
	$(CARGO) clippy --all-targets -- -D warnings

## check: fmt (verificación) + clippy + tests (como en CI)
.PHONY: check
check:
	$(CARGO) fmt --check
	$(CARGO) clippy --all-targets -- -D warnings
	$(CARGO) test

## install: instala el binario en $(PREFIX)/bin (PREFIX=/usr/local para global)
.PHONY: install
install: release
	install -Dm755 target/release/$(BIN) $(PREFIX)/bin/$(BIN)
	@echo "Instalado en $(PREFIX)/bin/$(BIN)"

## uninstall: elimina el binario instalado
.PHONY: uninstall
uninstall:
	rm -f $(PREFIX)/bin/$(BIN)

## clean: limpia los artefactos de compilación
.PHONY: clean
clean:
	$(CARGO) clean

## tag: crea y pushea un tag de release (uso: make tag VERSION=v0.1.0)
##      dispara el workflow que publica los binarios multi-arch.
.PHONY: tag
tag:
	@test -n "$(VERSION)" || { echo "Uso: make tag VERSION=v0.2.0"; exit 1; }
	@echo "$(VERSION)" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+' || { echo "VERSION debe tener forma vMAJOR.MINOR.PATCH"; exit 1; }
	@git diff --quiet || { echo "Hay cambios sin commitear; commiteá antes de taggear."; exit 1; }
	$(eval SEMVER := $(patsubst v%,%,$(VERSION)))
	sed -i 's/^version = "[0-9]*\.[0-9]*\.[0-9]*"/version = "$(SEMVER)"/' Cargo.toml
	$(CARGO) generate-lockfile --quiet
	git add Cargo.toml
	git commit -m "chore: bump version to $(VERSION)"
	git tag -a "$(VERSION)" -m "Release $(VERSION)"
	git push origin "$(VERSION)"
	@echo "Tag $(VERSION) pusheado. El workflow publicará binarios e imágenes Docker."

## untag: borra un tag local y remoto (uso: make untag VERSION=v0.1.0)
.PHONY: untag
untag:
	@test -n "$(VERSION)" || { echo "Uso: make untag VERSION=v0.1.0"; exit 1; }
	-git tag -d "$(VERSION)"
	-git push origin --delete "$(VERSION)"
	@echo "Tag $(VERSION) borrado (local y remoto)."
