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
(sin Docker igual puedes crear salas y generar los archivos).

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
| `g` | Generar archivos (astra.toml + docker-compose.yml + Caddyfile) sin tocar Docker |
| `D` | **Deploy**: genera y levanta todas las salas (`docker compose up -d`) |
| `s` / `x` | Start / Stop de la sala seleccionada |
| `u` | **Update**: baja la última imagen y recrea la sala (`pull` + `up -d --force-recreate`) |
| `U` | Update de todas las salas |
| `l` | Ver logs de la sala |
| `r` | Refrescar estado |
| `?` / `h` | Menú de ayuda con todos los atajos |
| `q` | Salir |

En el formulario: `Tab`/`↑`/`↓` moverse, `Espacio` alternar los switches,
`Enter` guardar, `Esc` cancelar.

La lista muestra por sala su estado y la **versión de Astra** que corre en el
contenedor (leída del binario en vivo, así ves qué salas quedaron atrás
después de un update).

### Abrir los puertos

El deploy publica el puerto de cada sala en el host, pero eso no alcanza para
que entre gente de afuera. Hay que abrirlo en **TCP y UDP** en los dos
firewalls:

```bash
# 1. Firewall del sistema operativo:
sudo ufw allow 5009/tcp && sudo ufw allow 5009/udp
```

```
# 2. Firewall del VPS (panel del proveedor):
#    security groups en AWS · reglas de ingreso en Oracle Cloud / GCP ·
#    el panel de Hetzner / Vultr / DigitalOcean
```

Es el paso que más se olvida: si el proveedor bloquea el puerto, nadie entra
aunque `ufw` lo permita y el contenedor figure como `running`.

### HTTPS (opcional, por sala)

Pon un **dominio** en el campo "Dominio HTTPS" del formulario (ej.
`chat.midominio.com`, con su DNS apuntando al servidor). Con al menos una sala
con dominio, el deploy agrega un [Caddy](https://caddyserver.com) como reverse
proxy que obtiene y renueva solo los certificados de Let's Encrypt:

- `https://<dominio>/`      → cliente web de esa sala (TLS)
- `https://<dominio>/admin` → panel de administración (TLS)
- `<ip>:<puerto>`           → clientes Ares (directo; el protocolo Ares no soporta TLS)

Caddy publica los puertos 80/443: deben estar libres en el host **y abiertos
en el firewall de tu VPS** (además del firewall del SO), o Let's Encrypt no
puede validar el dominio y el certificado nunca se emite. Varias salas pueden
tener cada una su dominio; comparten el mismo Caddy. Si ninguna sala tiene
dominio, no se genera ni levanta nada extra.

### Modo headless (automatización / CI)

```bash
# Reconcilia docker-compose.yml + los astra.toml con el estado guardado.
astra-creator generate /srv/astra-salas
```

Reconcilia, no impone: la config de sala se preserva desde los `astra.toml`
existentes (ver [Quién manda sobre astra.toml](#quién-manda-sobre-astratoml)),
así que es seguro correrlo en cada deploy sin revertir lo que el operador haya
cambiado desde el panel.

## Qué genera

```
<dir>/
├── astra-creator.json      # estado (tus salas) — versionable
├── docker-compose.yml      # un servicio por sala (+ caddy si hay dominios)
├── Caddyfile               # solo si alguna sala tiene dominio HTTPS
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
- **Panel web**: `http://<tu-ip>:<puerto>/admin` (con el owner password de esa
  sala), o `https://<dominio>/admin` si la sala tiene dominio HTTPS.
- **Chat**: `/login <owner_password>` y los comandos.

## Quién manda sobre astra.toml

El `astra.toml` de cada sala tiene **dos escritores**: astra-creator y el panel
`/admin` de Astra (pestañas Server / Room linking / Security y el editor TOML
crudo). Por eso astra-creator no lo regenera desde cero — lo reconcilia campo
por campo:

| Campo | Dueño |
|---|---|
| `port`, `web_port`, `data_dir` | **astra-creator**, siempre. Están acoplados al `ports:` del compose y al bind mount de datos; si el panel los cambia, se restituyen. |
| `room_name`, `bot_name`, `room_topic`, `owner_password`, `allow_registration`, `roomsearch` | **Los dos.** Gana el último que los tocó: la TUI relee el `astra.toml` antes de mostrar el formulario, así que al guardar solo escribe lo que cambiaste. |
| `language`, `web_enabled`, `guid`, `link_hub_enabled`, `link_trusted_leaves`, `security.*`, `seed_url`, `update_check`, y cualquier clave desconocida | **El panel.** astra-creator no los toca; se preservan con sus comentarios. |

Consecuencias prácticas:

- Es seguro correr `generate`, deployar o editar otra sala: ya no se pisa lo que
  configuraste desde el panel.
- Para cambiar la config de una sala usá **la TUI o el panel**. Editar a mano
  `astra-creator.json` y correr `generate` **no** la aplica: para esos campos la
  fuente de verdad es el `astra.toml`, y el JSON se refresca desde él.
- El bind mount del config va **sin `:ro`** justamente porque el panel escribe
  ahí. Con `:ro`, guardar desde el panel falla con
  `Read-only file system (os error 30)`.
- Si un `astra.toml` quedó corrupto (TOML inválido), `generate` **falla y lo
  dice** en vez de sobreescribirlo: se prefiere frenar antes que destruir la
  configuración de la sala.
- El puerto se pasa además como `--port` en el `command:` del compose, a
  propósito: es un pin que garantiza que el puerto del contenedor coincida con
  el mapeo `ports:`. Por eso editar el puerto desde el panel no tiene efecto (el
  panel lo advierte).

## Licencia

AGPL-3.0-or-later.
