use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, List, ListItem, Paragraph, StatefulWidget, Widget, Wrap},
};
use crate::app::{App, FileNode};

const LIME: Color = Color::Rgb(120, 230, 80);
const LIME_SOFT: Color = Color::Rgb(90, 190, 70);
const ORANGE: Color = Color::Rgb(255, 160, 0);
const BRANCH_BLUE: Color = Color::Rgb(202, 93, 42);
const LEFT_PAD: &str = "  ";

pub struct TreeWidget<'a> {
    app: &'a mut App,
}

impl<'a> TreeWidget<'a> {
    pub fn new(app: &'a mut App) -> Self {
        Self { app }
    }
}

impl<'a> Widget for TreeWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let items = {
            let visible_nodes = self.app.get_visible_nodes();
            let mut items = Vec::with_capacity(visible_nodes.len());
            for (prefix, node) in visible_nodes.iter() {
                let is_selected = node.path == self.app.selected_path;
                let display_name = Self::display_name(node);
                let name_style = Self::name_style(self.app, node, is_selected);
                let prefix = prefix.clone();

                let spans = vec![
                    Span::raw(LEFT_PAD),
                    Span::styled(prefix, Style::default().fg(BRANCH_BLUE)),
                    Span::styled(display_name, name_style),
                ];

                items.push(ListItem::new(Line::from(spans)));
            }
            items
        };

        let outer_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(LIME));
        let content_area = outer_block.inner(area);
        outer_block.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(2)])
            .split(content_area);

        let list = List::new(items);
        StatefulWidget::render(list, chunks[0], buf, &mut self.app.list_state);

        let files_style = if self.app.show_files {
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(LIME_SOFT)
        };
        let hidden_style = if self.app.show_hidden {
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(LIME_SOFT)
        };

        let guide_line = Line::from(vec![
            Span::raw(LEFT_PAD),
            Span::styled("↑/↓/→/←", Style::default().fg(LIME).add_modifier(Modifier::BOLD)),
            Span::styled(" Move  ", Style::default().fg(LIME_SOFT)),
            Span::styled("f", Style::default().fg(LIME).add_modifier(Modifier::BOLD)),
            Span::styled(" Files  ", files_style),
            Span::styled("h", Style::default().fg(LIME).add_modifier(Modifier::BOLD)),
            Span::styled(" Hidden  ", hidden_style),
            Span::styled("Enter", Style::default().fg(LIME).add_modifier(Modifier::BOLD)),
            Span::styled(" Select  ", Style::default().fg(LIME_SOFT)),
            Span::styled("q/Esc", Style::default().fg(LIME).add_modifier(Modifier::BOLD)),
            Span::styled(" Quit", Style::default().fg(LIME_SOFT)),
        ]);

        let guide_block = Block::default()
            .borders(Borders::TOP)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(LIME));
        let guide_area = guide_block.inner(chunks[1]);
        guide_block.render(chunks[1], buf);

        let guide = Paragraph::new(vec![guide_line])
            .wrap(Wrap { trim: false });
        guide.render(guide_area, buf);
    }
}

impl<'a> TreeWidget<'a> {
    fn display_name(node: &FileNode) -> String {
        let name = node.name();
        if name.is_empty() {
            node.path.display().to_string()
        } else {
            name
        }
    }

    fn name_style(app: &App, node: &FileNode, is_selected: bool) -> Style {
        if is_selected {
            Style::default().bg(LIME).fg(Color::Black).add_modifier(Modifier::BOLD)
        } else if node.path == app.startup_path {
            Style::default().fg(ORANGE).add_modifier(Modifier::BOLD)
        } else if node.path == app.root.path {
            Style::default().fg(LIME).add_modifier(Modifier::BOLD)
        } else if node.is_dir {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    }
}
