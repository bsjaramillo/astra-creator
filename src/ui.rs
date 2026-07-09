//! Renderizado de la TUI con ratatui.

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::{App, Field, Screen};

const ACCENT: Color = Color::Cyan;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(3),    // body
            Constraint::Length(3), // footer
        ])
        .split(area);

    draw_header(f, chunks[0], app);
    draw_body(f, chunks[1], app);
    draw_footer(f, chunks[2], app);

    match app.screen {
        Screen::Form => draw_form(f, area, app),
        Screen::Logs => draw_logs(f, area, app),
        Screen::ConfirmDelete => draw_confirm(f, area, app),
        Screen::EditImage => draw_image(f, area, app),
        Screen::List => {}
    }
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let docker = if app.docker_ok {
        Span::styled(" docker ✓ ", Style::default().fg(Color::Green))
    } else {
        Span::styled(" docker ✗ ", Style::default().fg(Color::Red))
    };
    let line = Line::from(vec![
        Span::styled(
            " astra-creator ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            " {} salas · imagen: {} ",
            app.project.rooms.len(),
            app.project.image
        )),
        docker,
    ]);
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn draw_body(f: &mut Frame, area: Rect, app: &App) {
    if app.project.rooms.is_empty() {
        let msg = Paragraph::new("No hay salas todavía.\n\nPresioná 'a' para crear la primera.")
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title(" Salas "));
        f.render_widget(msg, area);
        return;
    }

    let header = Row::new(vec!["", "ID", "Nombre", "Puerto", "Estado", "Admin"]).style(
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = app
        .project
        .rooms
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let sel = if i == app.selected { "▶" } else { " " };
            let state = app.state_of(r);
            let state_style = match state.as_str() {
                "running" => Style::default().fg(Color::Green),
                "exited" | "dead" => Style::default().fg(Color::Red),
                "—" => Style::default().fg(Color::DarkGray),
                _ => Style::default().fg(Color::Yellow),
            };
            let row_style = if i == app.selected {
                Style::default().add_modifier(Modifier::BOLD).fg(ACCENT)
            } else {
                Style::default()
            };
            Row::new(vec![
                Span::raw(sel.to_string()),
                Span::styled(r.id.clone(), row_style),
                Span::raw(r.room_name.clone()),
                Span::raw(r.port.to_string()),
                Span::styled(state, state_style),
                Span::raw(format!(":{}/admin", r.port)),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(2),
        Constraint::Length(16),
        Constraint::Min(14),
        Constraint::Length(7),
        Constraint::Length(9),
        Constraint::Length(14),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" Salas "));
    f.render_widget(table, area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let keys = "a:add  e:edit  d:del  i:image  D:deploy  s:start  x:stop  l:logs  g:gen  r:refresh  q:quit";
    let text = vec![
        Line::from(Span::styled(
            app.message.clone(),
            Style::default().fg(ACCENT),
        )),
        Line::from(Span::styled(keys, Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn centered(area: Rect, pct_x: u16, pct_y: u16) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - pct_y) / 2),
            Constraint::Percentage(pct_y),
            Constraint::Percentage((100 - pct_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(v[1])[1]
}

fn draw_form(f: &mut Frame, area: Rect, app: &App) {
    let Some(fb) = app.form.as_ref() else { return };
    let popup = centered(area, 70, 70);
    f.render_widget(Clear, popup);

    let title = if fb.editing_existing {
        " Editar sala "
    } else {
        " Nueva sala "
    };
    let mut lines: Vec<Line> = Vec::new();
    for (i, field) in Field::ALL.iter().enumerate() {
        let focused = i == fb.focus;
        let value = match field {
            Field::Id => fb.id.clone(),
            Field::RoomName => fb.room_name.clone(),
            Field::BotName => fb.bot_name.clone(),
            Field::OwnerPassword => "*".repeat(fb.owner_password.chars().count()),
            Field::Port => fb.port.clone(),
            Field::Topic => fb.topic.clone(),
            Field::AllowRegistration => (if fb.allow_registration { "[x]" } else { "[ ]" }).into(),
            Field::RoomSearch => (if fb.roomsearch { "[x]" } else { "[ ]" }).into(),
        };
        let cursor = if focused && !field.is_toggle() {
            "_"
        } else {
            ""
        };
        let label_style = if focused {
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{:>32}: ", field.label()), label_style),
            Span::styled(
                format!("{}{}", value, cursor),
                Style::default().fg(Color::White),
            ),
        ]));
    }
    lines.push(Line::from(""));
    if let Some(err) = &fb.error {
        lines.push(Line::from(Span::styled(
            err.clone(),
            Style::default().fg(Color::Red),
        )));
    }
    lines.push(Line::from(Span::styled(
        "Tab/↑↓: mover · Enter: guardar · Esc: cancelar · Espacio: toggles",
        Style::default().fg(Color::DarkGray),
    )));

    let p = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(Style::default().fg(ACCENT)),
    );
    f.render_widget(p, popup);
}

fn draw_logs(f: &mut Frame, area: Rect, app: &App) {
    let popup = centered(area, 90, 85);
    f.render_widget(Clear, popup);
    // Mostrar las últimas líneas que entren en el alto disponible.
    let height = popup.height.saturating_sub(2) as usize;
    let text: String = app
        .logs
        .lines()
        .rev()
        .take(height)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let p = Paragraph::new(text).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Logs (Esc para volver) ")
            .border_style(Style::default().fg(ACCENT)),
    );
    f.render_widget(p, popup);
}

fn draw_image(f: &mut Frame, area: Rect, app: &App) {
    let popup = centered(area, 70, 30);
    f.render_widget(Clear, popup);
    let text = vec![
        Line::from(Span::styled(
            "Imagen Docker de Astra (la misma para todas las salas):",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("{}_", app.image_buf),
            Style::default().fg(Color::White),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Enter: guardar · Esc: cancelar    (ej: ghcr.io/bsjaramillo/astra:latest o astra:local)",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Imagen ")
            .border_style(Style::default().fg(ACCENT)),
    );
    f.render_widget(p, popup);
}

fn draw_confirm(f: &mut Frame, area: Rect, app: &App) {
    let popup = centered(area, 50, 20);
    f.render_widget(Clear, popup);
    let name = app.selected_room().map(|r| r.id).unwrap_or_default();
    let text = vec![
        Line::from(format!("¿Eliminar la sala '{}'?", name)),
        Line::from(""),
        Line::from(Span::styled(
            "El volumen de datos se conserva. y: sí · cualquier otra: no",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let p = Paragraph::new(text).alignment(Alignment::Center).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Confirmar ")
            .border_style(Style::default().fg(Color::Red)),
    );
    f.render_widget(p, popup);
}
