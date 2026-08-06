//! Generación de los artefactos: `astra.toml` por sala y `docker-compose.yml`.
//!
//! # Dos escritores sobre el mismo `astra.toml`
//!
//! El `astra.toml` de cada sala lo edita **también** el panel `/admin` de Astra
//! en runtime (pestañas Server / Room linking / Security y el editor TOML
//! crudo). Por eso astra-creator NO regenera ese archivo desde cero: lo
//! reconcilia campo por campo, con un dueño definido para cada uno.
//!
//! - **De astra-creator** (orquestación, no puede cederlos porque están
//!   acoplados al `ports:` del compose y al bind mount de datos): `port`,
//!   `web_port`, `data_dir`.
//! - **De astra-creator, pero editables desde los dos lados**: los campos que
//!   modela [`RoomDef`] (`room_name`, `bot_name`, `room_topic`,
//!   `owner_password`, `allow_registration`, `roomsearch`). Gana el último que
//!   los tocó: antes de escribir, [`sync_rooms_from_disk`] refresca el
//!   `RoomDef` desde el archivo, así que reescribirlos es un no-op salvo en lo
//!   que el operador acaba de cambiar en la TUI.
//! - **Del panel, se preservan intactos**: `guid`, `language`, `web_enabled`,
//!   `link_hub_enabled`, `link_trusted_leaves`, `security.*`, `seed_url`,
//!   `update_check` y cualquier clave que astra-creator no conozca.
//!
//! Solo cuando el archivo no existe se escribe completo, como scaffold inicial
//! ([`astra_toml`]).

use std::path::Path;

use anyhow::Context;
use toml_edit::{value, DocumentMut};

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

/// Placeholder de `guid` que el servidor reconoce como "sin configurar".
///
/// astra-creator NO inventa el guid: escribe este valor y Astra lo reemplaza
/// por uno aleatorio (16 bytes en hex) la primera vez que arranca, y lo
/// persiste en el mismo `astra.toml` (por eso el compose lo monta sin `:ro`).
///
/// Antes se derivaba del id de la sala (`astra-<id>-guid`), lo que hacía que
/// el "secreto" fuera el nombre de la sala: el guid es con lo que un leaf se
/// autentica contra un hub (`SHA1(reverse(name ++ guid))`), así que cualquiera
/// que supiera el nombre podía hacerse pasar por la sala.
const PLACEHOLDER_GUID: &str = "astra-default-guid";

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
         # Lo reemplaza el servidor por un secreto aleatorio al primer arranque.\n\
         guid = {guid}\n",
        id = room.id,
        port = room.port,
        bot = toml_str(&room.bot_name),
        name = toml_str(&room.room_name),
        topic = toml_str(&room.topic),
        pw = toml_str(&room.owner_password),
        reg = room.allow_registration,
        search = room.roomsearch,
        guid = toml_str(PLACEHOLDER_GUID),
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

/// Aplica los campos que administra astra-creator sobre un `astra.toml` que
/// **ya existe**, preservando todo lo demás: comentarios, orden de las claves,
/// y cualquier valor que haya escrito el panel `/admin` (ver el doc del
/// módulo para el reparto de dueños).
///
/// Falla si el archivo no parsea como TOML. Es deliberado: antes se
/// sobreescribía a ciegas, y ante un archivo corrupto (o editado a mano con un
/// error) preferimos frenar y decirlo que destruir la configuración de la sala.
pub fn reconcile_astra_toml(existing: &str, room: &RoomDef) -> anyhow::Result<String> {
    let mut doc: DocumentMut = existing
        .parse()
        .context("el astra.toml existente no es TOML válido; corrígelo o bórralo para regenerarlo")?;

    // Orquestación: siempre gana astra-creator.
    doc["port"] = value(room.port as i64);
    doc["web_port"] = value(room.port as i64);
    doc["data_dir"] = value("/app/data");

    // Config de sala: el `RoomDef` viene refrescado desde disco, así que esto
    // solo cambia lo que el operador editó en la TUI.
    doc["room_name"] = value(&room.room_name);
    doc["bot_name"] = value(&room.bot_name);
    doc["room_topic"] = value(&room.topic);
    doc["owner_password"] = value(&room.owner_password);
    doc["allow_registration"] = value(room.allow_registration);
    doc["roomsearch"] = value(room.roomsearch);

    Ok(doc.to_string())
}

/// Refresca en `room` los campos que el panel `/admin` pudo haber cambiado,
/// leyéndolos del texto de su `astra.toml`. Best-effort: si el archivo no
/// parsea, deja el `RoomDef` como estaba (el error se reporta al escribir).
///
/// No toca `port` (es de astra-creator) ni `domain` (no vive en el toml, solo
/// en el estado de astra-creator y en el Caddyfile).
pub fn sync_room_from_toml(room: &mut RoomDef, toml_text: &str) {
    let Ok(doc) = toml_text.parse::<DocumentMut>() else {
        return;
    };
    let s = |k: &str| doc.get(k).and_then(|v| v.as_str()).map(|v| v.to_string());
    let b = |k: &str| doc.get(k).and_then(|v| v.as_bool());
    if let Some(v) = s("room_name") {
        room.room_name = v;
    }
    if let Some(v) = s("bot_name") {
        room.bot_name = v;
    }
    if let Some(v) = s("room_topic") {
        room.topic = v;
    }
    if let Some(v) = s("owner_password") {
        room.owner_password = v;
    }
    if let Some(v) = b("allow_registration") {
        room.allow_registration = v;
    }
    if let Some(v) = b("roomsearch") {
        room.roomsearch = v;
    }
}

/// Refresca todas las salas del proyecto desde sus `astra.toml` en disco, para
/// que la TUI no muestre (ni reescriba) valores viejos cuando el panel `/admin`
/// cambió algo. Importa especialmente para `owner_password`: es la credencial
/// del panel, y si la TUI muestra una distinta a la del archivo el operador no
/// puede entrar.
///
/// `keep` es el id de la sala que el operador acaba de editar en la TUI: esa se
/// deja intacta, porque su valor en memoria es más nuevo que el del disco.
pub fn sync_rooms_from_disk(base_dir: &Path, project: &mut Project, keep: Option<&str>) {
    for r in &mut project.rooms {
        if Some(r.id.as_str()) == keep {
            continue;
        }
        let path = base_dir.join("rooms").join(&r.id).join("astra.toml");
        if let Ok(text) = std::fs::read_to_string(&path) {
            sync_room_from_toml(r, &text);
        }
    }
}

/// Genera el `docker-compose.yml` con un servicio por sala. `default_uid`/
/// `default_gid` son el usuario por defecto del container (ver
/// [`host_uid_gid`]).
pub fn compose_yaml(project: &Project, default_uid: &str, default_gid: &str) -> String {
    let mut s = String::new();
    s.push_str("# Generado por astra-creator. No editar a mano: usa la TUI.\n");
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
        // así funciona aunque no exportes PUID/PGID. Igual puedes overridear:
        // PUID=$(id -u) PGID=$(id -g) docker compose up -d
        s.push_str(&format!(
            "    user: \"${{PUID:-{}}}:${{PGID:-{}}}\"\n",
            default_uid, default_gid
        ));
        s.push_str("    ports:\n");
        s.push_str(&format!("      - \"{p}:{p}\"\n", p = r.port));
        s.push_str(&format!("      - \"{p}:{p}/udp\"\n", p = r.port));
        s.push_str("    volumes:\n");
        // Montado SIN `:ro` a propósito: el panel `/admin` de Astra edita este
        // archivo (pestañas Server/Room linking/Security y el editor TOML). Con
        // `:ro` el kernel devuelve EROFS y guardar desde el panel falla con
        // "Read-only file system (os error 30)".
        s.push_str(&format!(
            "      - ./rooms/{id}/astra.toml:/app/astra.toml\n",
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
    // clientes Ares no pasan por aquí: hablan TCP binario directo al puerto
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
    s.push_str("# Generado por astra-creator. No editar a mano: usa la TUI.\n");
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
        // El panel /admin escribe en este mismo archivo: si ya existe se
        // reconcilia (solo los campos de astra-creator), no se regenera.
        let toml_path = dir.join("astra.toml");
        let text = match std::fs::read_to_string(&toml_path) {
            Ok(existing) => reconcile_astra_toml(&existing, r)
                .with_context(|| format!("reconciliando {}", toml_path.display()))?,
            // No existe todavía (sala nueva): scaffold completo.
            Err(_) => astra_toml(r),
        };
        std::fs::write(&toml_path, text)?;
        // Carpeta de datos de la sala (bind mount → /app/data). Se crea aquí,
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

    /// Un `astra.toml` como queda DESPUÉS de que el operador usó el panel
    /// `/admin`: con secciones que astra-creator no modela, valores cambiados
    /// en los campos que sí modela, y comentarios propios.
    const PANEL_EDITED: &str = r#"# Generado por astra-creator — sala 'uno'
port = 5009
bot_name = "BotDelPanel"
room_name = "Nombre Cambiado En El Panel"
room_topic = "topic del panel"
owner_password = "pw-cambiada-en-panel"
allow_registration = false
roomsearch = false
language = 3
web_enabled = true
web_port = 5009
data_dir = "/app/data"
# Comentario escrito a mano por el operador.
link_hub_enabled = true
guid = "guid-personalizado-del-panel"
seed_url = ""
update_check = false

[security]
max_new_connections_per_ip = 99
captcha_enabled = true

[[link_trusted_leaves]]
name = "otra-sala"
guid = "guid-del-leaf"
"#;

    #[test]
    fn reconcile_preserves_everything_the_panel_owns() {
        let room = RoomDef::new("uno", 5009);
        let out = reconcile_astra_toml(PANEL_EDITED, &room).unwrap();

        // Lo que es del panel sobrevive intacto.
        assert!(out.contains("language = 3"), "{out}");
        assert!(out.contains("link_hub_enabled = true"), "{out}");
        assert!(out.contains("guid = \"guid-personalizado-del-panel\""), "{out}");
        assert!(out.contains("update_check = false"), "{out}");
        assert!(out.contains("max_new_connections_per_ip = 99"), "{out}");
        assert!(out.contains("captcha_enabled = true"), "{out}");
        assert!(out.contains("[[link_trusted_leaves]]"), "{out}");
        assert!(out.contains("guid = \"guid-del-leaf\""), "{out}");
        // Y los comentarios también.
        assert!(out.contains("# Comentario escrito a mano por el operador."), "{out}");

        // Sigue siendo TOML válido y parseable.
        out.parse::<DocumentMut>().expect("el resultado debe ser TOML válido");
    }

    #[test]
    fn reconcile_always_wins_on_orchestration_fields() {
        // Si el panel cambió el puerto o el data_dir, astra-creator los
        // restituye: están acoplados al `ports:` del compose y al bind mount.
        let doctored = PANEL_EDITED
            .replace("port = 5009", "port = 9999")
            .replace("data_dir = \"/app/data\"", "data_dir = \"/otro/lado\"");
        let room = RoomDef::new("uno", 5009);
        let out = reconcile_astra_toml(&doctored, &room).unwrap();
        let doc: DocumentMut = out.parse().unwrap();
        assert_eq!(doc["port"].as_integer(), Some(5009));
        assert_eq!(doc["web_port"].as_integer(), Some(5009));
        assert_eq!(doc["data_dir"].as_str(), Some("/app/data"));
    }

    #[test]
    fn reconcile_applies_tui_edits_to_room_fields() {
        // El operador editó el nombre en la TUI: eso sí debe llegar al archivo.
        let mut room = RoomDef::new("uno", 5009);
        room.room_name = "Nombre Nuevo Desde La TUI".to_string();
        room.owner_password = "pw-de-la-tui".to_string();
        let out = reconcile_astra_toml(PANEL_EDITED, &room).unwrap();
        assert!(out.contains("room_name = \"Nombre Nuevo Desde La TUI\""), "{out}");
        assert!(out.contains("owner_password = \"pw-de-la-tui\""), "{out}");
    }

    #[test]
    fn reconcile_rejects_corrupt_toml_instead_of_clobbering() {
        let room = RoomDef::new("uno", 5009);
        let err = reconcile_astra_toml("esto no { es toml", &room).unwrap_err();
        assert!(format!("{err}").contains("no es TOML válido"), "{err}");
    }

    #[test]
    fn sync_reads_panel_values_back_into_the_room() {
        let mut room = RoomDef::new("uno", 5009);
        room.port = 5009;
        sync_room_from_toml(&mut room, PANEL_EDITED);
        assert_eq!(room.room_name, "Nombre Cambiado En El Panel");
        assert_eq!(room.bot_name, "BotDelPanel");
        assert_eq!(room.topic, "topic del panel");
        assert_eq!(room.owner_password, "pw-cambiada-en-panel");
        assert!(!room.allow_registration);
        assert!(!room.roomsearch);
        // El puerto NO se relee: es de astra-creator.
        assert_eq!(room.port, 5009);
    }

    #[test]
    fn sync_on_corrupt_toml_leaves_the_room_untouched() {
        let mut room = RoomDef::new("uno", 5009);
        let before = room.room_name.clone();
        sync_room_from_toml(&mut room, "roto { {");
        assert_eq!(room.room_name, before);
    }

    /// El ciclo completo: sala nueva → scaffold; el panel edita → astra-creator
    /// regenera y NO pisa; el operador edita en la TUI → sí se aplica.
    #[test]
    fn full_roundtrip_panel_and_tui_coexist() {
        let dir = std::env::temp_dir().join(format!("astra_creator_rt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut p = Project::default();
        p.rooms.push(RoomDef::new("uno", 5009));

        // 1) Primera generación: scaffold completo.
        write_project(&dir, &p).unwrap();
        let toml_path = dir.join("rooms/uno/astra.toml");
        assert!(toml_path.exists());

        // 2) El panel /admin edita el archivo: sube security, prende el link,
        //    y cambia el nombre de la sala.
        let panel = std::fs::read_to_string(&toml_path).unwrap()
            + "\nupdate_check = false\n[security]\nmax_concurrent_per_ip = 42\n";
        let panel = panel.replace(
            &format!("room_name = \"{}\"", p.rooms[0].room_name),
            "room_name = \"Renombrada Por El Panel\"",
        );
        std::fs::write(&toml_path, &panel).unwrap();

        // 3) astra-creator regenera (deploy, cambio de imagen, otra sala...).
        //    Antes esto borraba todo; ahora releé y preserva.
        sync_rooms_from_disk(&dir, &mut p, None);
        write_project(&dir, &p).unwrap();
        let after = std::fs::read_to_string(&toml_path).unwrap();
        assert!(after.contains("max_concurrent_per_ip = 42"), "{after}");
        assert!(after.contains("update_check = false"), "{after}");
        assert!(after.contains("Renombrada Por El Panel"), "{after}");
        // Y la TUI ahora refleja el nombre del panel.
        assert_eq!(p.rooms[0].room_name, "Renombrada Por El Panel");

        // 4) El operador edita el nombre en la TUI: esa sí gana.
        p.rooms[0].room_name = "Renombrada Por La TUI".to_string();
        write_project(&dir, &p).unwrap();
        let after = std::fs::read_to_string(&toml_path).unwrap();
        assert!(after.contains("Renombrada Por La TUI"), "{after}");
        assert!(after.contains("max_concurrent_per_ip = 42"), "{after}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// El camino de la TUI: el operador edita la sala en el formulario y su
    /// valor debe ganar, aunque el panel haya cambiado ese mismo campo antes.
    /// Es lo que hace `keep` — sin él, la relectura desharía la edición que se
    /// está guardando.
    #[test]
    fn keep_lets_the_tui_edit_win_over_disk() {
        let dir = std::env::temp_dir().join(format!("astra_creator_keep_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut p = Project::default();
        p.rooms.push(RoomDef::new("uno", 5009));
        p.rooms.push(RoomDef::new("dos", 5010));
        write_project(&dir, &p).unwrap();

        // El panel renombra las DOS salas por fuera.
        for id in ["uno", "dos"] {
            let path = dir.join("rooms").join(id).join("astra.toml");
            let t = std::fs::read_to_string(&path).unwrap();
            std::fs::write(
                &path,
                t.replace(
                    &format!("room_name = \"Sala {id}\""),
                    &format!("room_name = \"Panel {id}\""),
                ),
            )
            .unwrap();
        }

        // El operador edita SOLO 'uno' en la TUI.
        p.rooms[0].room_name = "TUI uno".to_string();
        sync_rooms_from_disk(&dir, &mut p, Some("uno"));
        write_project(&dir, &p).unwrap();

        // 'uno' se queda con lo de la TUI...
        let uno = std::fs::read_to_string(dir.join("rooms/uno/astra.toml")).unwrap();
        assert!(uno.contains("room_name = \"TUI uno\""), "{uno}");
        // ...y 'dos', que el operador no tocó, conserva lo del panel (antes se
        // le pisaba con el valor viejo en memoria).
        let dos = std::fs::read_to_string(dir.join("rooms/dos/astra.toml")).unwrap();
        assert!(dos.contains("room_name = \"Panel dos\""), "{dos}");

        std::fs::remove_dir_all(&dir).ok();
    }

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
    fn scaffold_guid_is_the_placeholder_not_the_room_id() {
        // Regresión: el scaffold escribía `astra-<id>-guid`, o sea el nombre
        // de la sala como "secreto" del link. Debe ir el placeholder, que el
        // servidor cambia por uno aleatorio al arrancar.
        let t = astra_toml(&RoomDef::new("mi-sala", 5009));
        assert!(t.contains("guid = \"astra-default-guid\""), "{t}");
        assert!(!t.contains("astra-mi-sala-guid"), "{t}");
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
    fn compose_mounts_config_writable() {
        // El panel /admin de Astra guarda cambios EN este archivo. Si el mount
        // lleva `:ro`, guardar falla con EROFS ("Read-only file system").
        let mut p = Project::default();
        p.rooms.push(RoomDef::new("uno", 5009));
        let y = compose_yaml(&p, "1000", "1000");
        assert!(y.contains("- ./rooms/uno/astra.toml:/app/astra.toml\n"));
        assert!(!y.contains("astra.toml:/app/astra.toml:ro"));
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
