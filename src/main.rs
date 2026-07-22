//! # astra-creator
//!
//! TUI para crear y administrar salas de chat Astra sobre Docker.
//!
//! Cada sala se define en la interfaz y se materializa como un contenedor
//! Astra propio (su puerto, su `astra.toml`, su volumen de datos). La
//! herramienta genera un `docker-compose.yml` que orquesta todas las salas y
//! permite deploy/start/stop/logs sin salir de la terminal.

mod docker;
mod generate;
mod model;
mod ui;

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use model::{Project, RoomDef};

/// Pantalla activa de la TUI.
pub enum Screen {
    /// Lista de salas (vista principal).
    List,
    /// Formulario de alta/edición.
    Form,
    /// Visor de logs de una sala.
    Logs,
    /// Confirmación de borrado.
    ConfirmDelete,
    /// Edición de la imagen Docker de Astra (project-level).
    EditImage,
    /// Menú de ayuda con todos los atajos de teclado.
    Help,
}

/// Operación Docker en curso (usada para feedback visual en el footer).
pub enum BusyOp {
    /// `docker compose up -d` para todas las salas.
    Deploy,
    /// `docker compose up -d <service>` para una sala concreta.
    Start(String),
    /// `docker compose stop <service>`.
    Stop(String),
    /// `pull + up --force-recreate` para una sala concreta.
    Update(String),
    /// `pull + up --force-recreate` para todas las salas.
    UpdateAll,
    /// Borrado completo de una sala: contenedor, volumen legado y carpeta de datos.
    Destroy(String),
}

impl BusyOp {
    /// Mensaje mostrado mientras la operación está en curso.
    pub fn label(&self) -> String {
        match self {
            BusyOp::Deploy     => "Desplegando todas las salas…".into(),
            BusyOp::Start(id)  => format!("Iniciando '{}'…", id),
            BusyOp::Stop(id)   => format!("Deteniendo '{}'…", id),
            BusyOp::Update(id) => format!("Actualizando '{}' (pull + recreate)…", id),
            BusyOp::UpdateAll  => "Actualizando todas las salas (pull + recreate)…".into(),
            BusyOp::Destroy(id) => format!("Eliminando '{}' (contenedor + datos)…", id),
        }
    }
    /// Mensaje de éxito una vez finalizada la operación.
    pub fn success(&self) -> String {
        match self {
            BusyOp::Deploy     => "✓ Deploy OK: todas las salas levantadas.".into(),
            BusyOp::Start(id)  => format!("✓ Sala '{}' iniciada.", id),
            BusyOp::Stop(id)   => format!("✓ Sala '{}' detenida.", id),
            BusyOp::Update(id) => format!("✓ Sala '{}' actualizada a la última imagen.", id),
            BusyOp::UpdateAll  => "✓ Todas las salas actualizadas a la última imagen.".into(),
            BusyOp::Destroy(id) => format!("✓ Sala '{}' eliminada: contenedor, volumen y datos.", id),
        }
    }
}

/// Campos editables en el formulario, en orden de tabulación.
#[derive(Clone, Copy, PartialEq)]
pub enum Field {
    Id,
    RoomName,
    BotName,
    OwnerPassword,
    Port,
    Topic,
    Domain,
    AllowRegistration,
    RoomSearch,
}

impl Field {
    pub const ALL: [Field; 9] = [
        Field::Id,
        Field::RoomName,
        Field::BotName,
        Field::OwnerPassword,
        Field::Port,
        Field::Topic,
        Field::Domain,
        Field::AllowRegistration,
        Field::RoomSearch,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            Field::Id                => "ID (slug)",
            Field::RoomName          => "Room name",
            Field::BotName           => "Bot name",
            Field::OwnerPassword     => "Owner password",
            Field::Port              => "Port",
            Field::Topic             => "Topic",
            Field::Domain            => "Dominio HTTPS (opcional)",
            Field::AllowRegistration => "Allow registration (space toggles)",
            Field::RoomSearch        => "Room search (space toggles)",
        }
    }
    pub fn is_toggle(&self) -> bool {
        matches!(self, Field::AllowRegistration | Field::RoomSearch)
    }
}

/// Buffer del formulario mientras se edita.
pub struct FormBuf {
    pub editing_existing: bool,
    pub orig_id: String,
    pub id: String,
    pub room_name: String,
    pub bot_name: String,
    pub owner_password: String,
    pub port: String,
    pub topic: String,
    pub domain: String,
    pub allow_registration: bool,
    pub roomsearch: bool,
    pub focus: usize,
    pub error: Option<String>,
}

impl FormBuf {
    fn from_room(r: &RoomDef) -> Self {
        Self {
            editing_existing: true,
            orig_id: r.id.clone(),
            id: r.id.clone(),
            room_name: r.room_name.clone(),
            bot_name: r.bot_name.clone(),
            owner_password: r.owner_password.clone(),
            port: r.port.to_string(),
            topic: r.topic.clone(),
            domain: r.domain.clone(),
            allow_registration: r.allow_registration,
            roomsearch: r.roomsearch,
            focus: 0,
            error: None,
        }
    }
    fn new(suggested_port: u16) -> Self {
        let r = RoomDef::new("nueva-sala", suggested_port);
        Self {
            editing_existing: false,
            orig_id: String::new(),
            id: r.id,
            room_name: r.room_name,
            bot_name: r.bot_name,
            owner_password: String::new(),
            port: suggested_port.to_string(),
            topic: r.topic,
            domain: String::new(),
            allow_registration: true,
            roomsearch: true,
            focus: 0,
            error: None,
        }
    }
}

/// Estado global de la aplicación TUI.
pub struct App {
    pub dir: PathBuf,
    pub project: Project,
    pub screen: Screen,
    pub selected: usize,
    pub form: Option<FormBuf>,
    pub status: Vec<docker::ServiceStatus>,
    pub docker_ok: bool,
    pub message: String,
    pub logs: String,
    /// Buffer de edición de la imagen (activo en `Screen::EditImage`).
    pub image_buf: String,
    pub should_quit: bool,
    /// Operación Docker en curso (`None` si no hay ninguna activa).
    pub busy: Option<BusyOp>,
    /// Contador de ticks para animar el spinner (incrementa cada ~100 ms).
    pub spinner_tick: u8,
    /// Slot compartido con el hilo de fondo: contiene el resultado al terminar.
    pub pending: Option<Arc<Mutex<Option<anyhow::Result<()>>>>>,
}

impl App {
    fn new(dir: PathBuf) -> Self {
        let project = Project::load(&dir);
        let docker_ok = docker::available();
        let mut app = Self {
            dir,
            project,
            screen: Screen::List,
            selected: 0,
            form: None,
            status: Vec::new(),
            docker_ok,
            message: if docker_ok {
                "Listo. a: agregar · D: deploy · s/x: start/stop · u: update · l: logs · ?: ayuda · q: salir".into()
            } else {
                "⚠ docker no disponible: puedes crear salas y generar archivos, pero no gestionar contenedores.".into()
            },
            logs: String::new(),
            image_buf: String::new(),
            should_quit: false,
            busy: None,
            spinner_tick: 0,
            pending: None,
        };
        app.refresh_status();
        app
    }

    fn refresh_status(&mut self) {
        if self.docker_ok {
            self.status = docker::status(&self.dir).unwrap_or_default();
        }
    }

    fn state_of(&self, room: &RoomDef) -> String {
        self.status
            .iter()
            .find(|s| s.service == room.service_name())
            .map(|s| s.state.clone())
            .unwrap_or_else(|| "—".into())
    }

    /// Versión de Astra corriendo en la sala (o "—" si no corre / no se sabe).
    fn version_of(&self, room: &RoomDef) -> String {
        self.status
            .iter()
            .find(|s| s.service == room.service_name())
            .and_then(|s| s.version.clone())
            .unwrap_or_else(|| "—".into())
    }

    fn selected_room(&self) -> Option<RoomDef> {
        self.project.rooms.get(self.selected).cloned()
    }

    /// Persiste el proyecto y regenera los archivos (sin tocar Docker).
    fn save_and_generate(&mut self) -> Result<()> {
        generate::write_project(&self.dir, &self.project)?;
        Ok(())
    }

    fn submit_form(&mut self) {
        let Some(f) = self.form.as_mut() else { return };
        let id = model::slugify(&f.id);
        let port: u16 = match f.port.trim().parse() {
            Ok(p) if p >= 1024 => p,
            _ => {
                f.error = Some("Puerto inválido (usa 1024–65535).".into());
                return;
            }
        };
        // Validaciones de unicidad.
        let except = if f.editing_existing {
            Some(f.orig_id.as_str())
        } else {
            None
        };
        if !f.editing_existing && self.project.has_id(&id) {
            f.error = Some(format!("Ya existe una sala con id '{}'.", id));
            return;
        }
        if self.project.port_in_use(port, except) {
            f.error = Some(format!("El puerto {} ya está en uso por otra sala.", port));
            return;
        }
        // Dominio: opcional. Se normaliza (minúsculas, sin esquema) y debe ser
        // único entre salas: cada site block del Caddyfile proxea a una sola.
        let domain = f
            .domain
            .trim()
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/')
            .to_ascii_lowercase();
        if !domain.is_empty() && (domain.contains(' ') || !domain.contains('.')) {
            f.error = Some("Dominio inválido (ej: chat.midominio.com).".into());
            return;
        }
        if self.project.domain_in_use(&domain, except) {
            f.error = Some(format!("El dominio '{}' ya está en uso por otra sala.", domain));
            return;
        }
        let room = RoomDef {
            id: id.clone(),
            room_name: f.room_name.trim().to_string(),
            bot_name: f.bot_name.trim().to_string(),
            owner_password: f.owner_password.clone(),
            port,
            topic: f.topic.trim().to_string(),
            domain,
            allow_registration: f.allow_registration,
            roomsearch: f.roomsearch,
        };
        // Si se renombró el id al editar, quitar el viejo.
        if f.editing_existing && f.orig_id != id {
            self.project.remove(&f.orig_id);
        }
        self.project.upsert(room);
        self.form = None;
        self.screen = Screen::List;
        match self.save_and_generate() {
            Ok(_) => self.message = format!("✓ Sala '{}' guardada y archivos regenerados.", id),
            Err(e) => self.message = format!("✗ Error al escribir archivos: {}", e),
        }
        if self.selected >= self.project.rooms.len() && !self.project.rooms.is_empty() {
            self.selected = self.project.rooms.len() - 1;
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Subcomando headless: `astra-creator generate [dir]` regenera los
    // archivos desde el estado guardado, sin abrir la TUI (útil para CI /
    // automatización).
    if args.first().map(|s| s.as_str()) == Some("generate") {
        let dir = args
            .get(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let project = Project::load(&dir);
        generate::write_project(&dir, &project)?;
        println!(
            "Generados docker-compose.yml + {} astra.toml en {}",
            project.rooms.len(),
            dir.display()
        );
        return Ok(());
    }

    // Directorio de trabajo: argumento opcional, si no el cwd.
    let dir = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    std::fs::create_dir_all(&dir).ok();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(dir);
    let res = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

fn run_app<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        // Verificar si el hilo de fondo completó su operación.
        let done = if let Some(pending) = &app.pending {
            let mut guard = pending.lock().unwrap();
            guard.take()
        } else {
            None
        };
        if let Some(result) = done {
            app.message = match result {
                Ok(_) => app.busy.take().map(|b| b.success()).unwrap_or_default(),
                Err(e) => {
                    app.busy = None;
                    format!("✗ {}", e)
                }
            };
            app.pending = None;
            app.refresh_status();
        }

        // Esperar evento hasta 100 ms; el timeout permite animar el spinner.
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match app.screen {
                    Screen::List          => handle_list_key(app, key.code),
                    Screen::Form          => handle_form_key(app, key.code),
                    Screen::Logs          => {
                        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                            app.screen = Screen::List;
                        }
                    }
                    Screen::ConfirmDelete => handle_confirm_key(app, key.code),
                    Screen::EditImage     => handle_image_key(app, key.code),
                    Screen::Help          => {
                        // Cualquier tecla cierra la ayuda.
                        app.screen = Screen::List;
                    }
                }
            }
        } else if app.busy.is_some() {
            // Timeout sin evento: avanzar el spinner.
            app.spinner_tick = app.spinner_tick.wrapping_add(1);
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

/// Lanza una operación Docker en un hilo secundario para no bloquear la TUI.
/// El resultado queda en `app.pending`; el loop principal lo recoge en el
/// siguiente ciclo y actualiza el mensaje y el estado de los contenedores.
fn spawn_docker<F>(app: &mut App, op: BusyOp, f: F)
where
    F: FnOnce() -> anyhow::Result<()> + Send + 'static,
{
    let slot: Arc<Mutex<Option<anyhow::Result<()>>>> = Arc::new(Mutex::new(None));
    let slot_thread = Arc::clone(&slot);
    std::thread::spawn(move || {
        let res = f();
        *slot_thread.lock().unwrap() = Some(res);
    });
    app.message = op.label();
    app.busy = Some(op);
    app.pending = Some(slot);
}

fn handle_list_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Down | KeyCode::Char('j') if !app.project.rooms.is_empty() => {
            app.selected = (app.selected + 1) % app.project.rooms.len();
        }
        KeyCode::Up | KeyCode::Char('k') if !app.project.rooms.is_empty() => {
            app.selected = (app.selected + app.project.rooms.len() - 1) % app.project.rooms.len();
        }
        KeyCode::Char('a') => {
            app.form = Some(FormBuf::new(app.project.next_free_port()));
            app.screen = Screen::Form;
        }
        KeyCode::Char('e') => {
            if let Some(r) = app.selected_room() {
                app.form = Some(FormBuf::from_room(&r));
                app.screen = Screen::Form;
            }
        }
        KeyCode::Char('d') if app.selected_room().is_some() => {
            app.screen = Screen::ConfirmDelete;
        }
        KeyCode::Char('D') => {
            if app.busy.is_some() {
                return;
            }
            if !app.docker_ok {
                app.message = "✗ docker no disponible.".into();
                return;
            }
            if let Err(e) = app.save_and_generate() {
                app.message = format!("✗ Error generando: {}", e);
                return;
            }
            let dir = app.dir.clone();
            spawn_docker(app, BusyOp::Deploy, move || {
                docker::deploy(&dir, None).map(|_| ())
            });
        }
        KeyCode::Char('s') => {
            if app.busy.is_some() {
                return;
            }
            if let Some(r) = app.selected_room() {
                let _ = app.save_and_generate();
                let dir = app.dir.clone();
                let svc = r.service_name();
                spawn_docker(app, BusyOp::Start(r.id.clone()), move || {
                    docker::deploy(&dir, Some(&svc)).map(|_| ())
                });
            }
        }
        KeyCode::Char('x') => {
            if app.busy.is_some() {
                return;
            }
            if let Some(r) = app.selected_room() {
                let dir = app.dir.clone();
                let svc = r.service_name();
                spawn_docker(app, BusyOp::Stop(r.id.clone()), move || {
                    docker::stop(&dir, Some(&svc)).map(|_| ())
                });
            }
        }
        KeyCode::Char('u') => {
            if app.busy.is_some() {
                return;
            }
            if !app.docker_ok {
                app.message = "✗ docker no disponible.".into();
                return;
            }
            if let Some(r) = app.selected_room() {
                let _ = app.save_and_generate();
                let dir = app.dir.clone();
                let svc = r.service_name();
                spawn_docker(app, BusyOp::Update(r.id.clone()), move || {
                    docker::update(&dir, Some(&svc)).map(|_| ())
                });
            }
        }
        KeyCode::Char('U') => {
            if app.busy.is_some() {
                return;
            }
            if !app.docker_ok {
                app.message = "✗ docker no disponible.".into();
                return;
            }
            if app.project.rooms.is_empty() {
                app.message = "No hay salas para actualizar.".into();
                return;
            }
            let _ = app.save_and_generate();
            let dir = app.dir.clone();
            spawn_docker(app, BusyOp::UpdateAll, move || {
                docker::update(&dir, None).map(|_| ())
            });
        }
        KeyCode::Char('?') | KeyCode::Char('h') => {
            app.screen = Screen::Help;
        }
        KeyCode::Char('l') => {
            if let Some(r) = app.selected_room() {
                app.logs = docker::logs(&app.dir, &r.service_name(), 200)
                    .unwrap_or_else(|e| format!("(sin logs: {})", e));
                app.screen = Screen::Logs;
            }
        }
        KeyCode::Char('r') => {
            app.refresh_status();
            app.message = "Estado actualizado.".into();
        }
        KeyCode::Char('g') => match app.save_and_generate() {
            Ok(_) => app.message = "✓ Archivos generados (astra.toml + docker-compose.yml).".into(),
            Err(e) => app.message = format!("✗ Error: {}", e),
        },
        KeyCode::Char('i') => {
            app.image_buf = app.project.image.clone();
            app.screen = Screen::EditImage;
        }
        _ => {}
    }
}

fn handle_image_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => app.screen = Screen::List,
        KeyCode::Enter => {
            let img = app.image_buf.trim();
            if img.is_empty() {
                app.message = "La imagen no puede estar vacía.".into();
                return;
            }
            app.project.image = img.to_string();
            match app.save_and_generate() {
                Ok(_) => app.message = format!("✓ Imagen actualizada: {}", app.project.image),
                Err(e) => app.message = format!("✗ Error: {}", e),
            }
            app.screen = Screen::List;
        }
        KeyCode::Char(c) => app.image_buf.push(c),
        KeyCode::Backspace => {
            app.image_buf.pop();
        }
        _ => {}
    }
}

fn handle_form_key(app: &mut App, code: KeyCode) {
    let Some(f) = app.form.as_mut() else { return };
    let field = Field::ALL[f.focus];
    match code {
        KeyCode::Esc => {
            app.form = None;
            app.screen = Screen::List;
        }
        KeyCode::Tab | KeyCode::Down => f.focus = (f.focus + 1) % Field::ALL.len(),
        KeyCode::BackTab | KeyCode::Up => {
            f.focus = (f.focus + Field::ALL.len() - 1) % Field::ALL.len()
        }
        KeyCode::Enter => app.submit_form(),
        KeyCode::Char(' ') if field.is_toggle() => match field {
            Field::AllowRegistration => f.allow_registration = !f.allow_registration,
            Field::RoomSearch => f.roomsearch = !f.roomsearch,
            _ => {}
        },
        KeyCode::Char(c) if !field.is_toggle() => {
            let buf = field_buf(f, field);
            buf.push(c);
        }
        KeyCode::Backspace if !field.is_toggle() => {
            field_buf(f, field).pop();
        }
        _ => {}
    }
}

fn field_buf(f: &mut FormBuf, field: Field) -> &mut String {
    match field {
        Field::Id            => &mut f.id,
        Field::RoomName      => &mut f.room_name,
        Field::BotName       => &mut f.bot_name,
        Field::OwnerPassword => &mut f.owner_password,
        Field::Port          => &mut f.port,
        Field::Topic         => &mut f.topic,
        Field::Domain        => &mut f.domain,
        _ => unreachable!("toggle no tiene buffer de texto"),
    }
}

fn handle_confirm_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('y') | KeyCode::Enter => {
            app.screen = Screen::List;
            if app.busy.is_some() {
                app.message = "✗ Hay una operación en curso; espera a que termine.".into();
                return;
            }
            if let Some(r) = app.selected_room() {
                app.project.remove(&r.id);
                let _ = app.save_and_generate();
                if app.selected > 0 && app.selected >= app.project.rooms.len() {
                    app.selected -= 1;
                }
                let dir = app.dir.clone();
                let docker_ok = app.docker_ok;
                let id = r.id.clone();
                let svc = r.service_name();
                spawn_docker(app, BusyOp::Destroy(r.id), move || {
                    // Primero el contenedor (libera el bind mount), después la
                    // carpeta con los datos. Cada paso se intenta aunque el
                    // anterior falle; los errores se juntan en un solo mensaje.
                    let mut errors: Vec<String> = Vec::new();
                    if docker_ok {
                        if let Err(e) = docker::destroy_room(&dir, &svc, &id) {
                            errors.push(e.to_string());
                        }
                    }
                    let room_dir = dir.join("rooms").join(&id);
                    if room_dir.exists() {
                        if let Err(e) = std::fs::remove_dir_all(&room_dir) {
                            errors.push(format!(
                                "no se pudo borrar {}: {}",
                                room_dir.display(),
                                e
                            ));
                        }
                    }
                    if errors.is_empty() {
                        Ok(())
                    } else {
                        anyhow::bail!(errors.join(" · "))
                    }
                });
            }
        }
        _ => app.screen = Screen::List,
    }
}
