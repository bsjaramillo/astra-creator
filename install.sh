#!/usr/bin/env sh
# Instalador de astra-creator: baja el binario correcto para tu plataforma
# desde GitHub Releases y lo instala en /usr/local/bin (o ~/.local/bin).
#
#   curl -sSL https://raw.githubusercontent.com/<OWNER>/astra-creator/main/install.sh | sh
#
# Variables:
#   ASTRA_CREATOR_REPO  owner/repo (default: OWNER/astra-creator)
#   ASTRA_CREATOR_VERSION  tag a instalar (default: latest)
#   BINDIR              directorio de instalación

set -eu

REPO="${ASTRA_CREATOR_REPO:-OWNER/astra-creator}"
VERSION="${ASTRA_CREATOR_VERSION:-latest}"

# Detectar OS/arch.
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Linux)  os_tag="unknown-linux-musl" ;;
    Darwin) os_tag="apple-darwin" ;;
    *) echo "OS no soportado: $os (usá cargo install o compilá desde fuente)"; exit 1 ;;
esac
case "$arch" in
    x86_64|amd64) arch_tag="x86_64" ;;
    aarch64|arm64) arch_tag="aarch64" ;;
    *) echo "Arquitectura no soportada: $arch"; exit 1 ;;
esac

asset="astra-creator-${arch_tag}-${os_tag}"
if [ "$VERSION" = "latest" ]; then
    url="https://github.com/${REPO}/releases/latest/download/${asset}"
else
    url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
fi

# Elegir directorio de instalación.
if [ -n "${BINDIR:-}" ]; then
    bindir="$BINDIR"
elif [ -w /usr/local/bin ] 2>/dev/null; then
    bindir="/usr/local/bin"
else
    bindir="$HOME/.local/bin"
fi
mkdir -p "$bindir"

echo "Descargando $asset ($VERSION)…"
tmp="$(mktemp)"
curl -sSL "$url" -o "$tmp"
chmod +x "$tmp"
mv "$tmp" "$bindir/astra-creator"

echo "Instalado en $bindir/astra-creator"
case ":$PATH:" in
    *":$bindir:"*) : ;;
    *) echo "⚠ Agregá $bindir a tu PATH." ;;
esac
echo "Ejecutá: astra-creator"
