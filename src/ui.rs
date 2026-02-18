use crate::app::{App, Dialog, SortField, Tab};
use crate::patterns::Category;
use bytesize::ByteSize;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs, Wrap,
};
use ratatui::Frame;

const CATEGORY_COLORS: &[(Category, Color)] = &[
    (Category::JavaScript, Color::Yellow),
    (Category::Rust, Color::Red),
    (Category::Python, Color::Blue),
    (Category::Java, Color::Magenta),
    (Category::DotNet, Color::Cyan),
    (Category::Swift, Color::LightRed),
    (Category::Dart, Color::LightBlue),
    (Category::Go, Color::LightCyan),
    (Category::Build, Color::Green),
    (Category::Cache, Color::DarkGray),
];

fn category_color(cat: Category) -> Color {
    CATEGORY_COLORS
        .iter()
        .find(|(c, _)| *c == cat)
        .map(|(_, color)| *color)
        .unwrap_or(Color::White)
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // tabs + status
            Constraint::Min(5),   // main content
            Constraint::Length(2), // footer
        ])
        .split(f.area());

    draw_tabs(f, app, chunks[0]);

    match app.tab {
        Tab::Cruft => draw_cruft_table(f, app, chunks[1]),
        Tab::LargeFiles => draw_large_files_table(f, app, chunks[1]),
        Tab::Overview => draw_overview(f, app, chunks[1]),
    }

    draw_footer(f, app, chunks[2]);

    // Draw dialog overlay if active
    match &app.dialog {
        Dialog::None => {}
        Dialog::ConfirmDelete { count, total_size } => {
            draw_confirm_dialog(f, *count, *total_size);
        }
        Dialog::DeleteResult { deleted, errors } => {
            draw_result_dialog(f, *deleted, errors);
        }
    }
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles: Vec<Line> = [Tab::Cruft, Tab::LargeFiles, Tab::Overview]
        .iter()
        .map(|t| {
            let count = match t {
                Tab::Cruft => format!(" {} ({}) ", t.title(), app.cruft_items.len()),
                Tab::LargeFiles => format!(" {} ({}) ", t.title(), app.large_file_items.len()),
                Tab::Overview => format!(" {} ", t.title()),
            };
            Line::from(count)
        })
        .collect();

    let selected = match app.tab {
        Tab::Cruft => 0,
        Tab::LargeFiles => 1,
        Tab::Overview => 2,
    };

    let status = if app.scanning {
        let path = app
            .scan_progress
            .as_ref()
            .map(|p| {
                let s = p.display().to_string();
                if s.len() > 50 {
                    format!("...{}", &s[s.len() - 47..])
                } else {
                    s
                }
            })
            .unwrap_or_default();
        format!("  Scanning: {path}")
    } else {
        format!(
            "  Scan complete — {} cruft dirs, {} large files",
            app.cruft_items.len(),
            app.large_file_items.len()
        )
    };

    let block = Block::default()
        .title(Span::styled(status, Style::default().fg(Color::DarkGray)))
        .borders(Borders::BOTTOM);

    let tabs = Tabs::new(titles)
        .block(block)
        .select(selected)
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .divider("|");

    f.render_widget(tabs, area);
}

fn draw_cruft_table(f: &mut Frame, app: &mut App, area: Rect) {
    let sort_indicator = sort_indicator_str(app.sort_field, app.sort_ascending);

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("Path"),
        Cell::from(format!("Size {sort_indicator}")),
        Cell::from("Category"),
        Cell::from("Description"),
    ])
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = app
        .cruft_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let check = if app.cruft_selected.contains(&i) {
                "[x]"
            } else {
                "[ ]"
            };
            let size = ByteSize(item.size).to_string();
            let cat_color = category_color(item.category);
            Row::new(vec![
                Cell::from(check),
                Cell::from(item.path.display().to_string()),
                Cell::from(size),
                Cell::from(item.category.as_str()).style(Style::default().fg(cat_color)),
                Cell::from(item.description),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Min(30),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(22),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(format!(
        " Cruft Directories — Total: {} ",
        ByteSize(app.total_cruft_size())
    )))
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut app.cruft_table_state);
}

fn draw_large_files_table(f: &mut Frame, app: &mut App, area: Rect) {
    let sort_indicator = sort_indicator_str(app.sort_field, app.sort_ascending);

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from("Path"),
        Cell::from(format!("Size {sort_indicator}")),
    ])
    .style(
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    );

    let rows: Vec<Row> = app
        .large_file_items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let check = if app.large_selected.contains(&i) {
                "[x]"
            } else {
                "[ ]"
            };
            let size = ByteSize(item.size).to_string();
            Row::new(vec![
                Cell::from(check),
                Cell::from(item.path.display().to_string()),
                Cell::from(size),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Min(40),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(format!(
        " Large Files — Total: {} ",
        ByteSize(app.total_large_size())
    )))
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut app.large_table_state);
}

fn draw_overview(f: &mut Frame, app: &App, area: Rect) {
    let stats = app.category_stats();
    let total_size = app.total_cruft_size();

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Category Overview — Total: {} ",
            ByteSize(total_size)
        ));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if stats.is_empty() {
        let msg = if app.scanning {
            "Scanning..."
        } else {
            "No cruft directories found."
        };
        let p = Paragraph::new(msg).style(Style::default().fg(Color::DarkGray));
        f.render_widget(p, inner);
        return;
    }

    // Horizontal bar chart
    let bar_width = inner.width.saturating_sub(30) as u64; // leave room for labels

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(
            stats
                .iter()
                .map(|_| Constraint::Length(2))
                .chain(std::iter::once(Constraint::Min(0)))
                .collect::<Vec<_>>(),
        )
        .split(inner);

    for (i, (cat, size, count)) in stats.iter().enumerate() {
        if i >= rows.len() - 1 {
            break;
        }
        let color = category_color(*cat);
        let fraction = if total_size > 0 {
            (*size as f64) / (total_size as f64)
        } else {
            0.0
        };
        let filled = (fraction * bar_width as f64).round() as usize;
        let bar: String = "█".repeat(filled) + &"░".repeat(bar_width as usize - filled);
        let pct = (fraction * 100.0).round() as u32;

        let label = format!(
            "{:<12} {:>9} ({:>2}%) {:>3} dirs",
            cat.as_str(),
            ByteSize(*size),
            pct,
            count,
        );

        let line = Line::from(vec![
            Span::styled(label, Style::default().fg(Color::White)),
            Span::raw(" "),
            Span::styled(bar, Style::default().fg(color)),
        ]);

        let p = Paragraph::new(line);
        f.render_widget(p, rows[i]);
    }
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let sel_count = app.selected_count();
    let sel_size = app.selected_size();

    let left = if sel_count > 0 {
        format!("{sel_count} selected ({})", ByteSize(sel_size))
    } else {
        String::new()
    };

    let sort_label = match app.sort_field {
        SortField::Size => "size",
        SortField::Path => "path",
        SortField::Category => "category",
    };

    let right =
        "j/k:nav  space:select  a:all  d:delete  s:sort  tab:switch  q:quit";

    let line = Line::from(vec![
        Span::styled(left, Style::default().fg(Color::Yellow)),
        Span::raw("  "),
        Span::styled(
            format!("[sort: {sort_label}]"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("  "),
        Span::styled(right, Style::default().fg(Color::DarkGray)),
    ]);

    let p = Paragraph::new(line).block(Block::default().borders(Borders::TOP));
    f.render_widget(p, area);
}

fn draw_confirm_dialog(f: &mut Frame, count: usize, total_size: u64) {
    let area = centered_rect(50, 8, f.area());
    f.render_widget(Clear, area);

    let text = format!(
        "Delete {count} item{}? ({} will be freed)\n\nPress y to confirm, n to cancel",
        if count == 1 { "" } else { "s" },
        ByteSize(total_size)
    );

    let block = Block::default()
        .title(" Confirm Delete ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red));

    let p = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Color::White));

    f.render_widget(p, area);
}

fn draw_result_dialog(f: &mut Frame, deleted: usize, errors: &[String]) {
    let height = 6 + errors.len().min(5) as u16;
    let area = centered_rect(50, height, f.area());
    f.render_widget(Clear, area);

    let mut text = format!("Deleted {deleted} item(s).");
    if !errors.is_empty() {
        text.push_str(&format!("\n\n{} error(s):", errors.len()));
        for (i, e) in errors.iter().take(5).enumerate() {
            text.push_str(&format!("\n  {}. {e}", i + 1));
        }
    }
    text.push_str("\n\nPress any key to close.");

    let border_color = if errors.is_empty() {
        Color::Green
    } else {
        Color::Yellow
    };

    let block = Block::default()
        .title(" Result ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let p = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: true })
        .style(Style::default().fg(Color::White));

    f.render_widget(p, area);
}

fn sort_indicator_str(field: SortField, ascending: bool) -> &'static str {
    match (field, ascending) {
        (SortField::Size, false) => "▼",
        (SortField::Size, true) => "▲",
        _ => "",
    }
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let popup_width = (area.width * percent_x / 100).max(30);
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, popup_width, height.min(area.height))
}
