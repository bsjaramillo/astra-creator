//! Wrappers sobre el CLI `docker compose` para el ciclo de vida de las salas.
//!
//! Se hace shell-out a `docker` (no hay crate oficial estable). Todas las
//! funciones son best-effort y devuelven la salida combinada o un error
//! legible; si `docker` no está instalado, lo reportan con claridad.

use std::path::Path;
use std::process::Command;

/// Estado de un servicio (sala) reportado por `docker compose ps`.
#[derive(Debug, Clone)]
pub struct ServiceStatus {
    /// Nombre del servicio (ej. `astra-misala`).
    pub service: String,
    /// Estado (`running`, `exited`, `created`, …).
    pub state: String,
}

/// ¿Está `docker` disponible y el daemon corriendo?
pub fn available() -> bool {
    Command::new("docker")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn compose_file(dir: &Path) -> std::path::PathBuf {
    dir.join("docker-compose.yml")
}

/// Ejecuta `docker compose -f <dir>/docker-compose.yml <args...>` y devuelve
/// la salida combinada (stdout+stderr) o un error.
fn run(dir: &Path, args: &[&str]) -> anyhow::Result<String> {
    let file = compose_file(dir);
    if !file.exists() {
        anyhow::bail!("no hay docker-compose.yml (deploy primero)");
    }
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(&file)
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| anyhow::anyhow!("no se pudo ejecutar docker: {}", e))?;

    let mut out = String::from_utf8_lossy(&output.stdout).into_owned();
    out.push_str(&String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(out)
    } else {
        anyhow::bail!("docker compose falló: {}", out.trim())
    }
}

/// `up -d` de todas las salas (o `up -d <service>` si se da uno).
pub fn deploy(dir: &Path, service: Option<&str>) -> anyhow::Result<String> {
    match service {
        Some(s) => run(dir, &["up", "-d", s]),
        None => run(dir, &["up", "-d", "--remove-orphans"]),
    }
}

/// Detiene una sala (o todas si `service` es `None`).
pub fn stop(dir: &Path, service: Option<&str>) -> anyhow::Result<String> {
    match service {
        Some(s) => run(dir, &["stop", s]),
        None => run(dir, &["stop"]),
    }
}

/// Actualiza el servidor Astra: baja la imagen más nueva (`pull`) y recrea el
/// contenedor (`up -d --force-recreate`). Si `service` es `None`, actualiza
/// todas las salas.
///
/// El `pull` es best-effort: para imágenes locales (ej. `astra:local`) que no
/// están en un registry, falla y se omite, pero igual se recrea el contenedor
/// (útil si la imagen local se reconstruyó por fuera).
pub fn update(dir: &Path, service: Option<&str>) -> anyhow::Result<String> {
    let mut out = String::new();

    let pull_args: Vec<&str> = match service {
        Some(s) => vec!["pull", s],
        None => vec!["pull"],
    };
    match run(dir, &pull_args) {
        Ok(o) => out.push_str(&o),
        Err(e) => out.push_str(&format!("(pull omitido — imagen local o registry inaccesible: {})\n", e)),
    }

    let up_args: Vec<&str> = match service {
        Some(s) => vec!["up", "-d", "--force-recreate", s],
        None => vec!["up", "-d", "--force-recreate", "--remove-orphans"],
    };
    out.push_str(&run(dir, &up_args)?);
    Ok(out)
}

/// Ejecuta `docker <args...>` (sin compose) y devuelve la salida combinada.
/// Si la salida contiene alguna frase de `tolerate` (ej. "No such container",
/// "not found"), el fallo se trata como éxito: el recurso ya no existe, que
/// es lo que se buscaba. Las frases varían entre versiones de Docker.
fn docker_raw(args: &[&str], tolerate: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(|e| anyhow::anyhow!("no se pudo ejecutar docker: {}", e))?;
    let mut out = String::from_utf8_lossy(&output.stdout).into_owned();
    out.push_str(&String::from_utf8_lossy(&output.stderr));
    let lower = out.to_lowercase();
    if output.status.success() || tolerate.iter().any(|t| lower.contains(&t.to_lowercase())) {
        Ok(out)
    } else {
        anyhow::bail!("docker {} falló: {}", args.join(" "), out.trim())
    }
}

/// ¿Existe el volumen? (`docker volume inspect` best-effort)
fn volume_exists(name: &str) -> bool {
    Command::new("docker")
        .args(["volume", "inspect", name])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Nombre de proyecto que Docker Compose deriva del directorio: el basename
/// en minúsculas, filtrado a `[a-z0-9_-]` y sin separadores al inicio.
fn compose_project_name(dir: &Path) -> String {
    let base = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    let name = base
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let filtered: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    filtered
        .trim_start_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_string()
}

/// Elimina por completo los recursos Docker de una sala: fuerza el borrado de
/// su contenedor y de los volúmenes con nombre que usaban versiones previas
/// (`astra-<id>-data`, con y sin prefijo de proyecto compose). No toca el
/// compose file, así funciona aunque la sala ya no figure en él. Tolerante a
/// "no existe": solo falla ante errores reales del daemon.
pub fn destroy_room(dir: &Path, container: &str, room_id: &str) -> anyhow::Result<()> {
    docker_raw(&["rm", "-f", container], &["No such container", "not found"])?;
    let legacy = format!("astra-{}-data", room_id);
    let prefixed = format!("{}_{}", compose_project_name(dir), legacy);
    for vol in [prefixed, legacy] {
        // Se consulta la existencia primero en vez de tolerar el error del
        // `rm`: la frase exacta ("no such volume" / "not found") cambia entre
        // versiones de Docker y no es confiable para distinguir fallos reales.
        if volume_exists(&vol) {
            docker_raw(&["volume", "rm", &vol], &["no such volume", "not found"])?;
        }
    }
    Ok(())
}

/// Logs de una sala (últimas `tail` líneas).
pub fn logs(dir: &Path, service: &str, tail: u32) -> anyhow::Result<String> {
    run(
        dir,
        &["logs", "--no-color", "--tail", &tail.to_string(), service],
    )
}

/// Estado de todos los servicios definidos.
pub fn status(dir: &Path) -> anyhow::Result<Vec<ServiceStatus>> {
    let out = run(dir, &["ps", "-a", "--format", "json"])?;
    let mut result = Vec::new();
    // `docker compose ps --format json` emite un objeto JSON por línea
    // (compose v2). Toleramos también un array JSON completo.
    let trimmed = out.trim();
    if trimmed.starts_with('[') {
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(trimmed) {
            for v in arr {
                if let Some(s) = parse_status(&v) {
                    result.push(s);
                }
            }
        }
    } else {
        for line in trimmed.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                if let Some(s) = parse_status(&v) {
                    result.push(s);
                }
            }
        }
    }
    Ok(result)
}

fn parse_status(v: &serde_json::Value) -> Option<ServiceStatus> {
    let service = v
        .get("Service")
        .and_then(|x| x.as_str())
        .or_else(|| v.get("Name").and_then(|x| x.as_str()))?
        .to_string();
    let state = v
        .get("State")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    Some(ServiceStatus { service, state })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_from_compose_json() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"Service":"astra-x","State":"running","Name":"astra-x"}"#)
                .unwrap();
        let s = parse_status(&v).unwrap();
        assert_eq!(s.service, "astra-x");
        assert_eq!(s.state, "running");
    }

    #[test]
    fn compose_project_name_normalizes_basename() {
        let dir = std::env::temp_dir().join("Mi Proyecto.Astra");
        std::fs::create_dir_all(&dir).ok();
        assert_eq!(compose_project_name(&dir), "miproyectoastra");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn run_without_compose_file_errors() {
        let dir = std::env::temp_dir().join("astra_creator_no_compose_xyz");
        std::fs::create_dir_all(&dir).ok();
        std::fs::remove_file(dir.join("docker-compose.yml")).ok();
        assert!(deploy(&dir, None).is_err());
    }

    /// Integración real contra el daemon: crea contenedor + volumen legado y
    /// verifica que `destroy_room` los elimina. Correr con:
    /// `cargo test destroy_room_removes -- --ignored` (requiere docker + alpine).
    #[test]
    #[ignore]
    fn destroy_room_removes_container_and_legacy_volume() {
        let sh = |args: &[&str]| Command::new("docker").args(args).output().unwrap();
        sh(&["run", "-d", "--name", "astra-prueba-del", "alpine", "sleep", "120"]);
        sh(&["volume", "create", "astra-prueba-del-data"]);

        destroy_room(&std::env::temp_dir(), "astra-prueba-del", "prueba-del").unwrap();

        let ps = sh(&["ps", "-a", "--format", "{{.Names}}"]);
        assert!(!String::from_utf8_lossy(&ps.stdout).contains("astra-prueba-del"));
        let vols = sh(&["volume", "ls", "--format", "{{.Name}}"]);
        assert!(!String::from_utf8_lossy(&vols.stdout).contains("astra-prueba-del-data"));

        // Idempotente: repetir sobre recursos ya inexistentes no falla.
        destroy_room(&std::env::temp_dir(), "astra-prueba-del", "prueba-del").unwrap();
    }

    #[test]
    fn update_without_compose_file_errors() {
        let dir = std::env::temp_dir().join("astra_creator_no_compose_update");
        std::fs::create_dir_all(&dir).ok();
        std::fs::remove_file(dir.join("docker-compose.yml")).ok();
        // Sin compose ni pull posible, el `up` falla → error propagado.
        assert!(update(&dir, None).is_err());
    }
}
