use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, StatefulWidget, Widget, Wrap},
};
use crate::app::{App, FileNode};

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
                    Span::styled(prefix, Style::default().fg(self.app.current_theme.branch_color.into())),
                    Span::styled(display_name, name_style),
                ];

                items.push(ListItem::new(Line::from(spans)));
            }
            items
        };

        let outer_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(self.app.current_theme.border_style.into()));
        let content_area = outer_block.inner(area);
        outer_block.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(2)])
            .split(content_area);

        let list = List::new(items);
        StatefulWidget::render(list, chunks[0], buf, &mut self.app.list_state);

        // Status message overlay
        if let Some((msg, is_error)) = &self.app.status_message {
            if let Some(time) = self.app.status_time {
                if time.elapsed().as_secs() < 3 {
                    let color = if *is_error { Color::Red } else { Color::Green };
                    let status_block = Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(color));
                    let area = centered_rect(60, 20, area);
                    let area = Rect::new(area.x, area.y + area.height / 2 - 1, area.width, 3);

                    let paragraph = Paragraph::new(Line::from(vec![
                        Span::styled(msg, Style::default().fg(color).add_modifier(Modifier::BOLD))
                    ]))
                    .block(status_block)
                    .wrap(Wrap { trim: true });

                    Clear.render(area, buf);
                    paragraph.render(area, buf);
                }
            }
        }

        // Input popup
        if self.app.input_mode {
            let area = centered_rect(50, 20, area);
            let area = Rect::new(area.x, area.y + area.height / 2 - 1, area.width, 3);

            let input_block = Block::default()
                .title(" New Directory Name ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(self.app.current_theme.key_highlight.into()));

            let input_text = Paragraph::new(self.app.input_buffer.as_str())
                .style(Style::default().fg(Color::White))
                .block(input_block);

            Clear.render(area, buf);
            input_text.render(area, buf);
        }

        let files_style = if self.app.show_files {
            Style::default().fg(self.app.current_theme.key_highlight.into()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.app.current_theme.border_style_soft.into())
        };
        let hidden_style = if self.app.show_hidden {
            Style::default().fg(self.app.current_theme.key_highlight.into()).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.app.current_theme.border_style_soft.into())
        };

        let guide_line = Line::from(vec![
            Span::raw(LEFT_PAD),
            Span::styled("↑/↓/→/←", Style::default().fg(self.app.current_theme.border_fg.into()).add_modifier(Modifier::BOLD)),
            Span::styled(" Move  ", Style::default().fg(self.app.current_theme.border_style_soft.into())),
            Span::styled("f", Style::default().fg(self.app.current_theme.border_fg.into()).add_modifier(Modifier::BOLD)),
            Span::styled(" Files  ", files_style),
            Span::styled("a", Style::default().fg(self.app.current_theme.border_fg.into()).add_modifier(Modifier::BOLD)),
            Span::styled(" All  ", hidden_style),
            Span::styled("n", Style::default().fg(self.app.current_theme.border_fg.into()).add_modifier(Modifier::BOLD)),
            Span::styled(" New Dir  ", Style::default().fg(self.app.current_theme.border_style_soft.into())),
            Span::styled("Enter", Style::default().fg(self.app.current_theme.border_fg.into()).add_modifier(Modifier::BOLD)),
            Span::styled(" Select  ", Style::default().fg(self.app.current_theme.border_style_soft.into())),
            Span::styled("q/Esc", Style::default().fg(self.app.current_theme.border_fg.into()).add_modifier(Modifier::BOLD)),
            Span::styled(" Quit", Style::default().fg(self.app.current_theme.border_style_soft.into())),
        ]);

        let guide_block = Block::default()
            .borders(Borders::TOP)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(self.app.current_theme.border_style.into()));
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
            Style::default().bg(app.current_theme.border_fg.into()).fg(Color::Black).add_modifier(Modifier::BOLD)
        } else if node.path == app.startup_path {
            Style::default().fg(app.current_theme.key_highlight.into()).add_modifier(Modifier::BOLD)
        } else if node.path == app.root.path {
            Style::default().fg(app.current_theme.border_fg.into()).add_modifier(Modifier::BOLD)
        } else if node.is_dir {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        }
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
