//! Modelo de datos y persistencia de las salas.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Definición de una sala de chat (mapea a un contenedor Astra).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomDef {
    /// Identificador único (slug), usado para nombres de servicio/volumen/carpeta.
    pub id: String,
    /// Nombre visible de la sala.
    pub room_name: String,
    /// Nombre del bot del sistema.
    pub bot_name: String,
    /// Password de owner (habilita `/login` y el panel `/admin`).
    pub owner_password: String,
    /// Puerto host (único entre salas).
    pub port: u16,
    /// Topic inicial.
    pub topic: String,
    /// ¿Permitir registro de cuentas?
    pub allow_registration: bool,
    /// ¿Anunciar la sala en el room search UDP?
    pub roomsearch: bool,
    /// Dominio para HTTPS (vacío = sin TLS). Si se setea, el compose incluye
    /// un Caddy como reverse proxy: la web/admin de la sala queda en
    /// `https://<dominio>` con certificado automático de Let's Encrypt.
    /// Los clientes Ares siguen entrando directo por el puerto (TCP plano).
    #[serde(default)]
    pub domain: String,
}

impl RoomDef {
    /// Crea una sala con valores por defecto razonables.
    pub fn new(id: impl Into<String>, port: u16) -> Self {
        let id = slugify(&id.into());
        Self {
            room_name: format!("Sala {}", id),
            bot_name: "Astra".to_string(),
            owner_password: String::new(),
            port,
            topic: "Bienvenidos".to_string(),
            allow_registration: true,
            roomsearch: true,
            domain: String::new(),
            id,
        }
    }

    /// Nombre del servicio Docker Compose para esta sala.
    pub fn service_name(&self) -> String {
        format!("astra-{}", self.id)
    }
}

/// Estado completo del proyecto: la lista de salas + la imagen a usar.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    /// Imagen Docker de Astra a correr.
    #[serde(default = "default_image")]
    pub image: String,
    /// Salas definidas.
    #[serde(default)]
    pub rooms: Vec<RoomDef>,
}

fn default_image() -> String {
    "ghcr.io/bsjaramillo/astra:latest".to_string()
}

impl Default for Project {
    fn default() -> Self {
        Self {
            image: default_image(),
            rooms: Vec::new(),
        }
    }
}

impl Project {
    /// Carga el proyecto desde `<dir>/astra-creator.json`, o uno vacío si no existe.
    pub fn load(dir: &Path) -> Self {
        let path = Self::state_path(dir);
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persiste el proyecto a `<dir>/astra-creator.json`.
    pub fn save(&self, dir: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(Self::state_path(dir), json)?;
        Ok(())
    }

    /// Ruta del archivo de estado.
    pub fn state_path(dir: &Path) -> PathBuf {
        dir.join("astra-creator.json")
    }

    /// Sugiere el siguiente puerto libre (a partir de 5009).
    pub fn next_free_port(&self) -> u16 {
        let mut p = 5009u16;
        while self.rooms.iter().any(|r| r.port == p) {
            p += 1;
        }
        p
    }

    /// ¿El id ya existe?
    pub fn has_id(&self, id: &str) -> bool {
        self.rooms.iter().any(|r| r.id == id)
    }

    /// ¿El puerto ya está en uso por otra sala (excluyendo `except_id`)?
    pub fn port_in_use(&self, port: u16, except_id: Option<&str>) -> bool {
        self.rooms
            .iter()
            .any(|r| r.port == port && Some(r.id.as_str()) != except_id)
    }

    /// ¿El dominio ya está en uso por otra sala (excluyendo `except_id`)?
    pub fn domain_in_use(&self, domain: &str, except_id: Option<&str>) -> bool {
        !domain.is_empty()
            && self
                .rooms
                .iter()
                .any(|r| r.domain == domain && Some(r.id.as_str()) != except_id)
    }

    /// Salas con dominio configurado (las que entran al reverse proxy).
    pub fn tls_rooms(&self) -> impl Iterator<Item = &RoomDef> {
        self.rooms.iter().filter(|r| !r.domain.is_empty())
    }

    /// Inserta o reemplaza una sala por id.
    pub fn upsert(&mut self, room: RoomDef) {
        if let Some(existing) = self.rooms.iter_mut().find(|r| r.id == room.id) {
            *existing = room;
        } else {
            self.rooms.push(room);
        }
    }

    /// Elimina una sala por id. Retorna `true` si existía.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.rooms.len();
        self.rooms.retain(|r| r.id != id);
        self.rooms.len() != before
    }
}

/// Convierte un texto libre en un slug seguro (minúsculas, alfanumérico y `-`).
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "sala".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Mi Sala Cool!"), "mi-sala-cool");
        assert_eq!(slugify("  Español ñ  "), "espa-ol");
        assert_eq!(slugify("///"), "sala");
        assert_eq!(slugify("A_B-C"), "a-b-c");
    }

    #[test]
    fn next_free_port_skips_used() {
        let mut p = Project::default();
        p.rooms.push(RoomDef::new("a", 5009));
        p.rooms.push(RoomDef::new("b", 5010));
        assert_eq!(p.next_free_port(), 5011);
    }

    #[test]
    fn upsert_and_remove() {
        let mut p = Project::default();
        p.upsert(RoomDef::new("room1", 5009));
        assert!(p.has_id("room1"));
        assert_eq!(p.rooms.len(), 1);
        // upsert con mismo id reemplaza
        let mut r = RoomDef::new("room1", 5009);
        r.room_name = "Cambiada".into();
        p.upsert(r);
        assert_eq!(p.rooms.len(), 1);
        assert_eq!(p.rooms[0].room_name, "Cambiada");
        assert!(p.remove("room1"));
        assert!(!p.remove("room1"));
    }

    #[test]
    fn port_conflict_detection() {
        let mut p = Project::default();
        p.upsert(RoomDef::new("a", 5009));
        assert!(p.port_in_use(5009, None));
        assert!(!p.port_in_use(5009, Some("a"))); // la misma sala no cuenta
        assert!(!p.port_in_use(6000, None));
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("astra_creator_test_{}", std::process::id()));
        let mut p = Project::default();
        p.upsert(RoomDef::new("test-room", 5009));
        p.save(&dir).unwrap();
        let loaded = Project::load(&dir);
        assert_eq!(loaded.rooms.len(), 1);
        assert_eq!(loaded.rooms[0].id, "test-room");
        std::fs::remove_dir_all(&dir).ok();
    }
}
