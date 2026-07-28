//! Rendering. Reads [`App`]; decides nothing.
//!
//! Every pane is a function of state, so the same state always draws the same screen - which
//! is what lets [`crate::draw::render_to_string`] test the output as text.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use smysl_core::{Lod, Uid};
use smysl_pack::Reason;

use crate::app::{App, Sim};
use crate::Pane;

/// Colour by epistemic status. The scale runs weak to strong, and the point of the format
/// is that the difference is visible without reading.
fn status_style(status: smysl_core::Status) -> Style {
    use smysl_core::Status::*;
    let c = match status {
        Unfounded => Color::DarkGray,
        Speculative => Color::Red,
        Inferred => Color::Yellow,
        Derived => Color::Cyan,
        Cited => Color::Green,
        Measured => Color::LightGreen,
        // `Status` is non-exhaustive: a kernel type added in a later minor must render as
        // something rather than fail to compile a downstream UI.
        _ => Color::White,
    };
    Style::default().fg(c)
}

/// Why a unit is in the pack, in the words `pack --explain` uses.
fn reason_text(r: &Reason) -> String {
    match r {
        Reason::Focus => "focus (C5)".into(),
        Reason::ThreadPin => "thread (C5)".into(),
        Reason::Rebuts(u) => format!("rebuts {} (C3)", short(u)),
        Reason::Contests(u) => format!("contests {} (C4)", short(u)),
        Reason::DepOf(u) => format!("dep of {} (C1)", short(u)),
        Reason::GroundOf(u) => format!("ground of {} (C2)", short(u)),
        Reason::WarrantOf(u) => format!("warrant of {} (C6)", short(u)),
        _ => "earned on density".into(),
    }
}

fn short(u: &Uid) -> String {
    u.to_string().chars().take(10).collect()
}

fn lod_text(l: Lod) -> &'static str {
    match l {
        Lod::L0 => "L0",
        Lod::L1 => "L1",
        Lod::L2 => "L2",
        _ => "L?",
    }
}

/// The whole screen.
pub fn draw(f: &mut Frame<'_>, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // pane tabs
            Constraint::Min(3),    // body
            Constraint::Length(1), // status
        ])
        .split(f.area());

    draw_tabs(f, rows[0], app);
    if app.help_open() {
        draw_help(f, rows[1]);
    } else {
        draw_body(f, rows[1], app);
    }
    draw_status(f, rows[2], app);
}

fn draw_tabs(f: &mut Frame<'_>, area: Rect, app: &App) {
    let mut spans = Vec::new();
    for (i, p) in Pane::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        let style = if *p == app.pane() {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(format!(" {} ", p.title()), style));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_body(f: &mut Frame<'_>, area: Rect, app: &App) {
    // The graph list is always visible beside whatever pane is focused: every other pane
    // describes the selected unit, and a detail view with no visible subject is a puzzle.
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    draw_graph(f, cols[0], app);
    match app.pane() {
        Pane::Graph | Pane::Detail => draw_detail(f, cols[1], app),
        Pane::Thread => draw_thread(f, cols[1], app),
        Pane::Contentions => draw_contentions(f, cols[1], app),
        Pane::Lineage => draw_lineage(f, cols[1], app),
        Pane::PackSimulator => draw_pack(f, cols[1], app),
        Pane::Staging => draw_staging(f, cols[1], app),
    }
}

fn draw_graph(f: &mut Frame<'_>, area: Rect, app: &App) {
    let items: Vec<ListItem<'_>> = app
        .order()
        .iter()
        .enumerate()
        .map(|(i, uid)| {
            let Some(unit) = app.store().get(uid) else {
                return ListItem::new("<missing>");
            };
            // A leading marker shows pack membership at a glance, which is the pane's
            // reason for sitting beside the simulator.
            let mark = match app.sim().level(uid) {
                Some(l) => lod_text(l),
                None => "  ",
            };
            let cursor = if i == app.cursor() { ">" } else { " " };
            let pin = if app.focus().contains(uid) { "*" } else { " " };
            let line = Line::from(vec![
                Span::raw(format!("{cursor}{pin}{mark} ")),
                Span::styled(app.name(uid), status_style(unit.core.status)),
                Span::raw("  "),
                Span::styled(
                    unit.core.gist.clone(),
                    Style::default().add_modifier(Modifier::DIM),
                ),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = format!(" Graph ({} units) ", app.order().len());
    f.render_widget(
        List::new(items).block(pane_block(&title, app.pane() == Pane::Graph)),
        area,
    );
}

fn draw_detail(f: &mut Frame<'_>, area: Rect, app: &App) {
    let text = match app
        .selected()
        .and_then(|u| app.store().get(u).map(|x| (u, x)))
    {
        None => vec![Line::from("no unit selected")],
        Some((uid, unit)) => {
            let mut lines = vec![
                kv("uid", &uid.to_string()),
                kv("type", &unit.core.schema.to_string()),
                Line::from(vec![
                    Span::styled(
                        format!("{:<9}", "status"),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(unit.core.status.to_string(), status_style(unit.core.status)),
                ]),
                kv("salience", &format!("{:.3}", app.salience_of(uid))),
                Line::from(""),
                Line::from(Span::styled(
                    unit.core.gist.clone(),
                    Style::default().add_modifier(Modifier::BOLD),
                )),
            ];
            if let Some(body) = &unit.core.body {
                lines.push(Line::from(""));
                lines.push(Line::from(body.clone()));
            }
            if let Some(src) = &unit.core.source {
                lines.push(Line::from(""));
                lines.push(kv("source", &format!("{} {}", src.kind, src.reference)));
            }
            lines
        }
    };
    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .block(pane_block(" Detail ", app.pane() == Pane::Detail)),
        area,
    );
}

/// **The pane that earns the TUI.** A budget dial, what it costs, and - when the budget
/// cannot be met - the floor that would meet it.
fn draw_pack(f: &mut Frame<'_>, area: Rect, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    let full = app.full_cost().max(1);
    let ratio = (app.budget() as f64 / full as f64).clamp(0.0, 1.0);
    f.render_widget(
        Gauge::default()
            .block(pane_block(
                " Pack simulator ",
                app.pane() == Pane::PackSimulator,
            ))
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(ratio)
            .label(format!("{} / {} tokens", app.budget(), app.full_cost())),
        rows[0],
    );

    let body = match app.sim() {
        Sim::Infeasible { required } => vec![
            Line::from(Span::styled(
                "INFEASIBLE",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(format!(
                "The mandatory floor needs {required} tokens; the budget is {}.",
                app.budget()
            )),
            Line::from(""),
            Line::from("Packing refuses rather than shipping a claim without the"),
            Line::from("rebuttal that answers it. Widen with + until it fits."),
            Line::from("(A floor above zero means something is pinned - see f.)"),
        ],
        Sim::Packed {
            used,
            dropped,
            degraded,
            selected,
            ..
        } => {
            let mut lines = vec![
                kv(
                    "selected",
                    &format!("{} of {}", selected.len(), app.order().len()),
                ),
                kv("used", &format!("{used} of {} tokens", app.budget())),
                kv("dropped", &dropped.to_string()),
                kv("degraded", &degraded.to_string()),
                Line::from(""),
            ];
            // Why the unit under the cursor is in, or that it is not.
            match app.selected() {
                Some(uid) => {
                    lines.push(Line::from(Span::styled(
                        app.name(uid),
                        Style::default().add_modifier(Modifier::BOLD),
                    )));
                    lines.push(match app.sim().reason(uid) {
                        Some(r) => kv("kept", &reason_text(r)),
                        None => Line::from(Span::styled(
                            "dropped at this budget",
                            Style::default().fg(Color::DarkGray),
                        )),
                    });
                }
                None => lines.push(Line::from("no unit selected")),
            }
            lines
        }
    };

    f.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Constraints "),
        ),
        rows[1],
    );
}

fn draw_lineage(f: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines = Vec::new();
    match app.selected() {
        None => lines.push(Line::from("no unit selected")),
        Some(uid) => {
            let grounds = app.grounds_of(uid);
            let deps = app.deps_of(uid);
            let rebuttals = app.store().rebuttals_of(uid);

            lines.push(section("grounds", grounds.len()));
            for g in &grounds {
                lines.push(Line::from(format!("  {}", app.name(g))));
            }
            lines.push(section("deps", deps.len()));
            for d in &deps {
                lines.push(Line::from(format!("  {}", app.name(d))));
            }
            lines.push(section("rebutted by", rebuttals.len()));
            for r in &rebuttals {
                lines.push(Line::from(Span::styled(
                    format!("  {}", app.name(r)),
                    Style::default().fg(Color::Red),
                )));
            }
        }
    }
    f.render_widget(
        Paragraph::new(lines).block(pane_block(" Lineage ", app.pane() == Pane::Lineage)),
        area,
    );
}

fn draw_contentions(f: &mut Frame<'_>, area: Rect, app: &App) {
    let recorded = app.store().contentions();
    let mut lines = vec![kv("recorded", &recorded.len().to_string()), Line::from("")];
    if recorded.is_empty() {
        lines.push(Line::from(Span::styled(
            "None recorded in this store.",
            Style::default().add_modifier(Modifier::DIM),
        )));
        lines.push(Line::from(""));
        lines.push(Line::from(
            "Merge reports detections rather than writing them",
        ));
        lines.push(Line::from("into the log, so an unmerged store shows none."));
    }
    for c in recorded {
        lines.push(Line::from(format!("{} over {}", c.id, short(&c.over))));
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(pane_block(" Contentions ", app.pane() == Pane::Contentions)),
        area,
    );
}

fn draw_thread(f: &mut Frame<'_>, area: Rect, app: &App) {
    let mut lines = Vec::new();
    for t in app.store().threads() {
        lines.push(Line::from(Span::styled(
            format!("{} ({})", t.id, t.schema),
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for step in &t.steps {
            lines.push(Line::from(format!(
                "  {:<12} {}",
                step.role.to_string(),
                app.name(&step.unit)
            )));
        }
        lines.push(Line::from(""));
    }
    if lines.is_empty() {
        lines.push(Line::from("no threads in this store"));
        lines.push(Line::from(""));
        lines.push(Line::from("`smysl thread --derive` builds one."));
    }
    f.render_widget(
        Paragraph::new(lines).block(pane_block(" Thread ", app.pane() == Pane::Thread)),
        area,
    );
}

fn draw_staging(f: &mut Frame<'_>, area: Rect, app: &App) {
    let path = std::path::Path::new(".smysl/staged.smy");
    let staged = std::fs::read_to_string(path);
    let lines = match &staged {
        Ok(text) => text
            .lines()
            .take(200)
            .map(|l| Line::from(l.to_string()))
            .collect::<Vec<_>>(),
        Err(_) => vec![
            Line::from("nothing staged"),
            Line::from(""),
            Line::from("`smysl ingest` writes .smysl/staged.smy, and rule S keeps"),
            Line::from("model output there until `merge --staged` accepts it."),
        ],
    };
    f.render_widget(
        Paragraph::new(lines).block(pane_block(" Staging ", app.pane() == Pane::Staging)),
        area,
    );
}

fn draw_help(f: &mut Frame<'_>, area: Rect) {
    let lines = vec![
        Line::from("  Tab / shift-Tab   next / previous pane   (also h l)"),
        Line::from("  j k / arrows      move the cursor        (g G  first / last)"),
        Line::from("  + -               widen / narrow the pack budget"),
        Line::from("  0                 reset the budget to the whole graph"),
        Line::from("  f / Enter         pin the selected unit into the pack (C5)"),
        Line::from("  ?                 close this help"),
        Line::from("  q / Esc / ^C      quit"),
        Line::from(""),
        Line::from("  The pack simulator is the pane worth the trouble: hold - and"),
        Line::from("  watch which units survive, and why each one was kept."),
    ];
    f.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Help ")),
        area,
    );
}

fn draw_status(f: &mut Frame<'_>, area: Rect, app: &App) {
    let sim = match app.sim() {
        Sim::Infeasible { required } => format!("budget {} < floor {required}", app.budget()),
        Sim::Packed { used, selected, .. } => {
            format!("{} units, {used} tok", selected.len())
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {} ", app.pane().title()),
                Style::default().fg(Color::Black).bg(Color::Cyan),
            ),
            Span::raw(format!("  {sim}   ")),
            Span::styled("? help   q quit", Style::default().fg(Color::DarkGray)),
        ])),
        area,
    );
}

fn pane_block(title: &str, focused: bool) -> Block<'_> {
    let style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_style(style)
        .title(title.to_string())
}

fn kv<'a>(k: &str, v: &str) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{k:<9}"), Style::default().fg(Color::DarkGray)),
        Span::raw(v.to_string()),
    ])
}

fn section<'a>(name: &str, n: usize) -> Line<'a> {
    Line::from(Span::styled(
        format!("{name} ({n})"),
        Style::default().add_modifier(Modifier::BOLD),
    ))
}

/// Render one frame into a string, for tests and for `--screenshot`.
///
/// A TUI that can only be checked by looking at it is a TUI whose panes rot silently. This
/// is the whole reason [`App`] holds no terminal.
pub fn render_to_string(app: &App, width: u16, height: u16) -> String {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut term = Terminal::new(TestBackend::new(width, height)).expect("test backend");
    term.draw(|f| draw(f, app)).expect("draw");

    let buf = term.backend().buffer();
    let mut out = String::new();
    for y in 0..buf.area.height {
        for x in 0..buf.area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
