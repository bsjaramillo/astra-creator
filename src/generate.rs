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

/// Genera el `docker-compose.yml` con un servicio por sala.
pub fn compose_yaml(project: &Project) -> String {
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
        // (DB, bans, cuentas, avatares, scripts) queden accesibles/editables
        // desde el SO. Cambiá con: PUID=$(id -u) PGID=$(id -g) docker compose up -d
        s.push_str("    user: \"${PUID:-1000}:${PGID:-1000}\"\n");
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
    // Ya no hay volúmenes con nombre: cada sala usa un bind mount a
    // rooms/<id>/data (ver arriba), accesible desde el host.
    s
}

/// Escribe todos los artefactos en `base_dir`:
/// - `astra-creator.json` (estado)
/// - `docker-compose.yml`
/// - `rooms/<id>/astra.toml` por cada sala
pub fn write_project(base_dir: &Path, project: &Project) -> anyhow::Result<()> {
    std::fs::create_dir_all(base_dir)?;
    project.save(base_dir)?;
    std::fs::write(base_dir.join("docker-compose.yml"), compose_yaml(project))?;
    for r in &project.rooms {
        let dir = base_dir.join("rooms").join(&r.id);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("astra.toml"), astra_toml(r))?;
        // Carpeta de datos de la sala (bind mount → /app/data). Se crea acá,
        // con el dueño del usuario que corre astra-creator, para que Docker
        // no la cree como root y quede accesible/editable desde el SO.
        std::fs::create_dir_all(dir.join("data"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let y = compose_yaml(&p);
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
        let y = compose_yaml(&p);
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
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn empty_project_compose_has_no_volumes_section() {
        let p = Project::default();
        let y = compose_yaml(&p);
        assert!(!y.contains("\nvolumes:"));
    }
}
