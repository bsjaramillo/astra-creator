# astra-creator

> TUI para crear y administrar salas de chat [Astra](https://github.com/bsjaramillo/astra) sobre Docker.

`astra-creator` es una herramienta de terminal que te deja definir **una o
varias salas** de chat, generar por cada una su `astra.toml` + un
`docker-compose.yml` que las orquesta, y administrar su ciclo de vida
(deploy / start / stop / logs) sin salir de la terminal. Cada sala es un
contenedor Astra independiente (su puerto, su config, su volumen de datos).

Corre directo por SSH en tu servidor — sin navegador, sin abrir puertos extra,
un solo binario estático.

## Instalación

**Linux / macOS (binario):**
```bash
curl -sSL https://raw.githubusercontent.com/bsjaramillo/astra-creator/main/install.sh | sh
```

**Windows (PowerShell):**
```powershell
irm https://raw.githubusercontent.com/bsjaramillo/astra-creator/main/install.ps1 | iex
```

**Con Cargo:**
```bash
cargo install --git https://github.com/bsjaramillo/astra-creator
```

**Desde fuente:**
```bash
git clone https://github.com/bsjaramillo/astra-creator && cd astra-creator
cargo build --release   # -> target/release/astra-creator
```

Requiere `docker` + `docker compose` en la máquina para gestionar contenedores
(sin Docker igual podés crear salas y generar los archivos).

## Uso

```bash
# Abre la TUI en el directorio actual (guarda ahí el estado y los archivos).
astra-creator

# O en un directorio específico:
astra-creator /srv/astra-salas
```

En la TUI:

| Tecla | Acción |
|---|---|
| `a` | Agregar sala |
| `e` | Editar sala seleccionada |
| `d` | Eliminar sala (borra el contenedor, el volumen y la carpeta de datos) |
| `i` | Cambiar la imagen Docker de Astra (ej. `ghcr.io/bsjaramillo/astra:latest` o `astra:local`) |
| `g` | Generar archivos (astra.toml + docker-compose.yml) sin tocar Docker |
| `D` | **Deploy**: genera y levanta todas las salas (`docker compose up -d`) |
| `s` / `x` | Start / Stop de la sala seleccionada |
| `u` | **Update**: baja la última imagen y recrea la sala (`pull` + `up -d --force-recreate`) |
| `U` | Update de todas las salas |
| `l` | Ver logs de la sala |
| `r` | Refrescar estado |
| `?` / `h` | Menú de ayuda con todos los atajos |
| `q` | Salir |

En el formulario: `Tab`/`↑`/`↓` moverse, `Espacio` togglear los switches,
`Enter` guardar, `Esc` cancelar.

### Modo headless (automatización / CI)

```bash
# Regenera docker-compose.yml + los astra.toml desde el estado guardado.
astra-creator generate /srv/astra-salas
```

## Qué genera

```
<dir>/
├── astra-creator.json      # estado (tus salas) — editable/versionable
├── docker-compose.yml      # un servicio por sala
└── rooms/
    ├── <sala-1>/
    │   ├── astra.toml
    │   └── data/           # bind mount → /app/data (bans, cuentas, historial)
    └── <sala-2>/
```

Cada sala mapea su puerto host→contenedor (TCP + UDP) y guarda sus datos en
`rooms/<id>/data`, accesible directamente desde el host. Al eliminar una sala
con `d` se borra todo: contenedor, volumen legado y `rooms/<id>`.

## Administrar cada sala

Una vez desplegada, cada sala se administra como cualquier Astra:
- **Panel web**: `http://<tu-ip>:<puerto>/admin` (con el owner password de esa sala).
- **Chat**: `/login <owner_password>` y los comandos.

## Licencia

AGPL-3.0-or-later.
