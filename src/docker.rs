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

/// Baja y elimina todos los contenedores (mantiene los volúmenes de datos).
pub fn down(dir: &Path) -> anyhow::Result<String> {
    run(dir, &["down"])
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
    fn run_without_compose_file_errors() {
        let dir = std::env::temp_dir().join("astra_creator_no_compose_xyz");
        std::fs::create_dir_all(&dir).ok();
        std::fs::remove_file(dir.join("docker-compose.yml")).ok();
        assert!(deploy(&dir, None).is_err());
    }
}
