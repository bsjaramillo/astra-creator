//! Generación de los artefactos: `astra.toml` por sala y `docker-compose.yml`.

use std::path::Path;

use crate::model::{Project, RoomDef};

/// Escapa una string para un valor TOML entre comillas dobles.
fn toml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// GUID estable por sala (≥16 chars), derivado del id.
fn room_guid(id: &str) -> String {
    let base = format!("astra-{}-guid", id);
    if base.len() >= 16 {
        base
    } else {
        format!("{:0<16}", base)
    }
}

/// Genera el contenido de `astra.toml` para una sala. Incluye todos los
/// campos requeridos por `Settings`; el resto usa defaults del server.
pub fn astra_toml(room: &RoomDef) -> String {
    format!(
        "# Generado por astra-creator — sala '{id}'\n\
         port = {port}\n\
         bot_name = {bot}\n\
         room_name = {name}\n\
         room_topic = {topic}\n\
         owner_password = {pw}\n\
         allow_registration = {reg}\n\
         roomsearch = {search}\n\
         language = 0\n\
         web_enabled = true\n\
         web_port = {port}\n\
         data_dir = \"/app/data\"\n\
         # El link (multi-servidor) entra multiplexado por el puerto principal\n\
         # (`port`); no usa un puerto propio. Para enlazar otro servidor a esta\n\
         # sala como hub: `--link-client <host>:{port}`.\n\
         link_hub_enabled = false\n\
         guid = {guid}\n",
        id = room.id,
        port = room.port,
        bot = toml_str(&room.bot_name),
        name = toml_str(&room.room_name),
        topic = toml_str(&room.topic),
        pw = toml_str(&room.owner_password),
        reg = room.allow_registration,
        search = room.roomsearch,
        guid = toml_str(&room_guid(&room.id)),
    )
}

/// Detecta el UID/GID del usuario que corre astra-creator (Unix), para
/// usarlo como DEFAULT del container en el compose. Así el container corre
/// como el dueño de las carpetas montadas (`rooms/<id>/data`) y puede
/// escribir en `/app/data` (crear `logs/`, la DB, etc.). Sin esto, el
/// default era `1000` fijo: si tu UID no es 1000, el container no podía
/// crear `/app/data/logs` y el server paniqueaba al arrancar.
///
/// Fallback a `1000` si no se puede detectar o en plataformas sin `id`
/// (en Windows/Mac, Docker Desktop maneja los permisos de bind mounts por su
/// cuenta, así que el UID del host no aplica igual).
pub fn host_uid_gid() -> (String, String) {
    #[cfg(unix)]
    {
        let run = |arg: &str| -> Option<String> {
            let out = std::process::Command::new("id").arg(arg).output().ok()?;
            if !out.status.success() {
                return None;
            }
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()) {
                Some(s)
            } else {
                None
            }
        };
        let uid = run("-u").unwrap_or_else(|| "1000".to_string());
        let gid = run("-g").unwrap_or_else(|| "1000".to_string());
        (uid, gid)
    }
    #[cfg(not(unix))]
    {
        ("1000".to_string(), "1000".to_string())
    }
}

/// Genera el `docker-compose.yml` con un servicio por sala. `default_uid`/
/// `default_gid` son el usuario por defecto del container (ver
/// [`host_uid_gid`]).
pub fn compose_yaml(project: &Project, default_uid: &str, default_gid: &str) -> String {
    let mut s = String::new();
    s.push_str("# Generado por astra-creator. No editar a mano: usá la TUI.\n");
    s.push_str("services:\n");
    for r in &project.rooms {
        let svc = r.service_name();
        s.push_str(&format!("  {svc}:\n"));
        s.push_str(&format!("    image: {}\n", project.image));
        s.push_str(&format!("    container_name: {svc}\n"));
        s.push_str("    restart: unless-stopped\n");
        // Corre como el usuario del host para que los archivos de la sala
        // (DB, bans, cuentas, avatares, scripts, logs) queden accesibles/
        // editables desde el SO Y para que el container pueda ESCRIBIR en el
        // bind mount. El default es el UID/GID real del host (no 1000 fijo),
        // así funciona aunque no exportes PUID/PGID. Igual podés overridear:
        // PUID=$(id -u) PGID=$(id -g) docker compose up -d
        s.push_str(&format!(
            "    user: \"${{PUID:-{}}}:${{PGID:-{}}}\"\n",
            default_uid, default_gid
        ));
        s.push_str("    ports:\n");
        s.push_str(&format!("      - \"{p}:{p}\"\n", p = r.port));
        s.push_str(&format!("      - \"{p}:{p}/udp\"\n", p = r.port));
        s.push_str("    volumes:\n");
        s.push_str(&format!(
            "      - ./rooms/{id}/astra.toml:/app/astra.toml:ro\n",
            id = r.id
        ));
        // Bind mount: los datos de la sala viven en su propia carpeta
        // (rooms/<id>/data), accesibles directamente desde el explorador.
        s.push_str(&format!("      - ./rooms/{id}/data:/app/data\n", id = r.id));
        // Solo argumentos: la imagen ya tiene ENTRYPOINT ["/app/astra"].
        // Repetir el binario haría que Astra lo lea como subcomando.
        s.push_str(&format!(
            "    command: [\"--config\", \"/app/astra.toml\", \"--port\", \"{}\"]\n",
            r.port
        ));
        s.push_str("    environment:\n");
        s.push_str("      RUST_LOG: info\n");
    }
    // Si alguna sala tiene dominio, entra un Caddy compartido como reverse
    // proxy: da HTTPS (Let's Encrypt) a la web/admin de esas salas. Los
    // clientes Ares no pasan por acá: hablan TCP binario directo al puerto
    // publicado de cada sala (el protocolo Ares no soporta TLS).
    let tls: Vec<&RoomDef> = project.tls_rooms().collect();
    if !tls.is_empty() {
        s.push_str("  caddy:\n");
        s.push_str("    image: caddy:2-alpine\n");
        s.push_str("    container_name: astra-caddy\n");
        s.push_str("    restart: unless-stopped\n");
        s.push_str("    ports:\n");
        s.push_str("      - \"80:80\"\n");
        s.push_str("      - \"443:443\"\n");
        s.push_str("    volumes:\n");
        s.push_str("      - ./Caddyfile:/etc/caddy/Caddyfile:ro\n");
        s.push_str("      - caddy-data:/data\n");
        s.push_str("      - caddy-config:/config\n");
        s.push_str("    depends_on:\n");
        for r in &tls {
            s.push_str(&format!("      - {}\n", r.service_name()));
        }
        s.push_str("volumes:\n");
        s.push_str("  caddy-data:\n");
        s.push_str("  caddy-config:\n");
    }
    // Las salas no usan volúmenes con nombre: cada una monta un bind mount a
    // rooms/<id>/data (ver arriba), accesible desde el host.
    s
}

/// Genera el `Caddyfile`: un site block por sala con dominio, proxeando la
/// superficie HTTP/WebSocket (cliente web + `/admin`) al contenedor de esa
/// sala. Caddy obtiene y renueva los certificados solo; únicamente hace falta
/// que el DNS del dominio apunte al host.
pub fn caddyfile(project: &Project) -> String {
    let mut s = String::new();
    s.push_str("# Generado por astra-creator. No editar a mano: usá la TUI.\n");
    s.push_str("# El DNS de cada dominio debe apuntar a este servidor (puertos 80/443).\n");
    for r in project.tls_rooms() {
        s.push_str(&format!(
            "\n{domain} {{\n\treverse_proxy {svc}:{port}\n\tencode gzip\n}}\n",
            domain = r.domain,
            svc = r.service_name(),
            port = r.port,
        ));
    }
    s
}

/// Escribe todos los artefactos en `base_dir`:
/// - `astra-creator.json` (estado)
/// - `docker-compose.yml`
/// - `Caddyfile` (solo si alguna sala tiene dominio; si no, se elimina)
/// - `rooms/<id>/astra.toml` por cada sala
pub fn write_project(base_dir: &Path, project: &Project) -> anyhow::Result<()> {
    std::fs::create_dir_all(base_dir)?;
    project.save(base_dir)?;
    let (uid, gid) = host_uid_gid();
    std::fs::write(
        base_dir.join("docker-compose.yml"),
        compose_yaml(project, &uid, &gid),
    )?;
    let caddy_path = base_dir.join("Caddyfile");
    if project.tls_rooms().next().is_some() {
        std::fs::write(&caddy_path, caddyfile(project))?;
    } else if caddy_path.exists() {
        // Sin dominios no hay servicio caddy en el compose; un Caddyfile
        // viejo solo confunde. El deploy completo usa --remove-orphans, así
        // que el contenedor caddy previo también se limpia.
        std::fs::remove_file(&caddy_path)?;
    }
    for r in &project.rooms {
        let dir = base_dir.join("rooms").join(&r.id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("astra.toml"), astra_toml(r))?;
        // Carpeta de datos de la sala (bind mount → /app/data). Se crea acá,
        // con el dueño del usuario que corre astra-creator, para que Docker
        // no la cree como root y quede accesible/editable desde el SO.
        // También los subdirs `logs` y `scripts`: así ya existen con el dueño
        // correcto (el container corre como ese mismo UID) y no dan WARN al
        // arrancar. `scripts/` es además donde el operador deja sus scripts JS
        // (accesible desde el host, editable directamente).
        std::fs::create_dir_all(dir.join("data").join("logs"))?;
        std::fs::create_dir_all(dir.join("data").join("scripts"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn host_uid_gid_matches_id_command() {
        // host_uid_gid() debe coincidir con `id -u`/`id -g` reales, para que
        // el container corra como el dueño de las carpetas montadas.
        let (uid, gid) = host_uid_gid();
        let real_uid = String::from_utf8_lossy(
            &std::process::Command::new("id").arg("-u").output().unwrap().stdout,
        ).trim().to_string();
        let real_gid = String::from_utf8_lossy(
            &std::process::Command::new("id").arg("-g").output().unwrap().stdout,
        ).trim().to_string();
        assert_eq!(uid, real_uid, "el UID del compose debe ser el real del host");
        assert_eq!(gid, real_gid, "el GID del compose debe ser el real del host");
    }

    #[test]
    fn compose_user_uses_provided_uid_gid() {
        // Un UID distinto de 1000 debe aparecer en el compose (regresión del
        // bug: default 1000 fijo no coincidía con el dueño de las carpetas).
        let mut p = Project::default();
        p.rooms.push(RoomDef::new("sala", 5009));
        let y = compose_yaml(&p, "1007", "1007");
        assert!(y.contains("user: \"${PUID:-1007}:${PGID:-1007}\""), "compose:\n{}", y);
    }

    #[test]
    fn astra_toml_has_required_fields() {
        let r = RoomDef::new("mi-sala", 5009);
        let t = astra_toml(&r);
        for field in [
            "port =",
            "bot_name =",
            "room_name =",
            "owner_password =",
            "roomsearch =",
            "guid =",
            "data_dir =",
        ] {
            assert!(t.contains(field), "falta {} en:\n{}", field, t);
        }
        assert!(t.contains("port = 5009"));
    }

    #[test]
    fn astra_toml_escapes_values() {
        let mut r = RoomDef::new("x", 5009);
        r.room_name = "Sala \"con comillas\"".into();
        let t = astra_toml(&r);
        assert!(t.contains("\\\"con comillas\\\""));
    }

    #[test]
    fn compose_has_service_per_room() {
        let mut p = Project::default();
        p.rooms.push(RoomDef::new("uno", 5009));
        p.rooms.push(RoomDef::new("dos", 5010));
        let y = compose_yaml(&p, "1000", "1000");
        assert!(y.contains("astra-uno:"));
        assert!(y.contains("astra-dos:"));
        assert!(y.contains("\"5009:5009\""));
        assert!(y.contains("\"5010:5010/udp\""));
        // Data como bind mount en la carpeta propia de cada sala.
        assert!(y.contains("- ./rooms/uno/data:/app/data"));
        assert!(y.contains("- ./rooms/dos/data:/app/data"));
        // Corre como el usuario del host.
        assert!(y.contains("user: \"${PUID:-1000}:${PGID:-1000}\""));
    }

    #[test]
    fn compose_command_has_no_binary_path() {
        // La imagen tiene ENTRYPOINT ["/app/astra"]; el command debe llevar
        // solo args, no repetir /app/astra (si no, Astra lo lee como subcomando).
        let mut p = Project::default();
        p.rooms.push(RoomDef::new("x", 5009));
        let y = compose_yaml(&p, "1000", "1000");
        assert!(y.contains("command: [\"--config\", \"/app/astra.toml\", \"--port\", \"5009\"]"));
        assert!(!y.contains("command: [\"/app/astra\""));
    }

    #[test]
    fn write_project_creates_all_files() {
        let dir = std::env::temp_dir().join(format!("astra_creator_gen_{}", std::process::id()));
        let mut p = Project::default();
        p.rooms.push(RoomDef::new("room-a", 5009));
        write_project(&dir, &p).unwrap();
        assert!(dir.join("docker-compose.yml").exists());
        assert!(dir.join("astra-creator.json").exists());
        assert!(dir.join("rooms/room-a/astra.toml").exists());
        assert!(dir.join("rooms/room-a/data").is_dir());
        // Subdirs pre-creados con el dueño correcto (evitan WARN y problemas
        // de permisos al arrancar el container).
        assert!(dir.join("rooms/room-a/data/logs").is_dir());
        assert!(dir.join("rooms/room-a/data/scripts").is_dir());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_project_compose_has_no_volumes_section() {
        let p = Project::default();
        let y = compose_yaml(&p, "1000", "1000");
        assert!(!y.contains("\nvolumes:"));
    }

    #[test]
    fn compose_without_domains_has_no_caddy() {
        let mut p = Project::default();
        p.rooms.push(RoomDef::new("sala", 5009));
        let y = compose_yaml(&p, "1000", "1000");
        assert!(!y.contains("caddy"));
    }

    #[test]
    fn compose_with_domain_includes_caddy_service() {
        let mut p = Project::default();
        let mut r = RoomDef::new("sala", 5009);
        r.domain = "chat.example.com".into();
        p.rooms.push(r);
        p.rooms.push(RoomDef::new("otra", 5010)); // sin dominio
        let y = compose_yaml(&p, "1000", "1000");
        assert!(y.contains("  caddy:"));
        assert!(y.contains("\"80:80\""));
        assert!(y.contains("\"443:443\""));
        assert!(y.contains("- ./Caddyfile:/etc/caddy/Caddyfile:ro"));
        // depends_on solo de las salas con dominio.
        assert!(y.contains("      - astra-sala\n"));
        assert!(!y.contains("      - astra-otra\n"));
        // Volúmenes de Caddy (certs + config).
        assert!(y.contains("volumes:\n  caddy-data:\n  caddy-config:"));
    }

    #[test]
    fn caddyfile_has_block_per_domain() {
        let mut p = Project::default();
        let mut a = RoomDef::new("a", 5009);
        a.domain = "chat.example.com".into();
        let mut b = RoomDef::new("b", 5010);
        b.domain = "otra.example.com".into();
        p.rooms.push(a);
        p.rooms.push(b);
        p.rooms.push(RoomDef::new("c", 5011)); // sin dominio: no aparece
        let c = caddyfile(&p);
        assert!(c.contains("chat.example.com {"));
        assert!(c.contains("reverse_proxy astra-a:5009"));
        assert!(c.contains("otra.example.com {"));
        assert!(c.contains("reverse_proxy astra-b:5010"));
        // Ojo: "astra-c" a secas matchea el header "astra-creator".
        assert!(!c.contains("reverse_proxy astra-c:"));
    }

    #[test]
    fn write_project_creates_and_removes_caddyfile() {
        let dir = std::env::temp_dir().join(format!("astra_creator_caddy_{}", std::process::id()));
        let mut p = Project::default();
        let mut r = RoomDef::new("sala", 5009);
        r.domain = "chat.example.com".into();
        p.rooms.push(r);
        write_project(&dir, &p).unwrap();
        assert!(dir.join("Caddyfile").exists());
        // Al quitar el dominio, el Caddyfile obsoleto se elimina.
        p.rooms[0].domain.clear();
        write_project(&dir, &p).unwrap();
        assert!(!dir.join("Caddyfile").exists());
        std::fs::remove_dir_all(&dir).ok();
    }
}
