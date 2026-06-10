use crate::app::{App, FileNode};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, List, ListItem, Paragraph, StatefulWidget, Widget, Wrap,
    },
};

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
        let title = if self.app.history_mode {
            Line::from(vec![
                Span::styled(
                    " CDTREE ",
                    Style::default()
                        .fg(self.app.current_theme.key_highlight.into())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "HISTORY",
                    Style::default()
                        .fg(self.app.current_theme.border_fg.into())
                        .add_modifier(Modifier::BOLD),
                ),
            ])
        } else {
            Line::from(vec![Span::styled(
                " CDTREE ",
                Style::default()
                    .fg(self.app.current_theme.key_highlight.into())
                    .add_modifier(Modifier::BOLD),
            )])
        };

        let outer_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(self.app.current_theme.border_style.into()))
            .title(title);
        let content_area = outer_block.inner(area);
        outer_block.render(area, buf);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(2)])
            .split(content_area);

        if self.app.history_mode {
            self.render_history(chunks[0], chunks[1], buf);
        } else {
            self.render_tree(chunks[0], chunks[1], buf);
        }
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

    fn selected_style(app: &App) -> Style {
        Style::default()
            .bg(app.current_theme.border_fg.into())
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    }

    fn name_style(app: &App, node: &FileNode, is_selected: bool) -> Style {
        if is_selected {
            Self::selected_style(app)
        } else if node.path == app.startup_path {
            Style::default()
                .fg(app.current_theme.key_highlight.into())
                .add_modifier(Modifier::BOLD)
        } else if node.path == app.root.path {
            Style::default()
                .fg(app.current_theme.border_fg.into())
                .add_modifier(Modifier::BOLD)
        } else if node.is_dir {
            Style::default().fg(app.current_theme.border_fg.into())
        } else {
            Style::default().fg(Color::DarkGray)
        }
    }

    fn render_tree(self, tree_area: Rect, guide_area: Rect, buf: &mut Buffer) {
        let key_style = Style::default()
            .fg(self.app.current_theme.border_fg.into())
            .add_modifier(Modifier::BOLD);
        let label_style = Style::default().fg(self.app.current_theme.border_style_soft.into());

        // Pre-load lightweight child counts only for rows that can be drawn.
        self.app
            .ensure_visible_child_counts(self.app.list_state.offset(), tree_area.height as usize);

        let items = {
            let visible_nodes = self.app.get_visible_nodes();
            let mut items = Vec::with_capacity(visible_nodes.len());
            for (prefix, node) in visible_nodes.iter() {
                let is_selected = node.path == self.app.selected_path;
                let display_name = Self::display_name(node);
                let name_style = Self::name_style(self.app, node, is_selected);
                let prefix = prefix.clone();

                let mut spans = vec![
                    Span::raw(LEFT_PAD),
                    Span::styled(
                        prefix,
                        Style::default().fg(self.app.current_theme.branch_color.into()),
                    ),
                    Span::styled(display_name, name_style),
                ];

                if node.is_dir {
                    if let Some((dirs, files)) = node.child_counts() {
                        let count_style =
                            Style::default().fg(self.app.current_theme.border_fg.into());
                        let mut count_parts = Vec::new();
                        if dirs > 0 {
                            count_parts.push(format!("📂{}", dirs));
                        }
                        if files > 0 {
                            count_parts.push(format!("📄{}", files));
                        }
                        if !count_parts.is_empty() {
                            spans.push(Span::styled(
                                format!(" {}", count_parts.join(" ")),
                                count_style,
                            ));
                        }
                    }
                }

                if is_selected && node.is_dir {
                    let mode_style = Style::default().fg(self.app.current_theme.border_fg.into());
                    spans.push(Span::styled(
                        format!(" {}", self.app.mode.suffix()),
                        mode_style,
                    ));
                }

                items.push(ListItem::new(Line::from(spans)));
            }
            items
        };

        let list = List::new(items);
        StatefulWidget::render(list, tree_area, buf, &mut self.app.list_state);

        let files_style = if self.app.show_files {
            Style::default()
                .fg(self.app.current_theme.key_highlight.into())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.app.current_theme.border_style_soft.into())
        };
        let hidden_style = if self.app.show_hidden {
            Style::default()
                .fg(self.app.current_theme.key_highlight.into())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(self.app.current_theme.border_style_soft.into())
        };

        let guide_line = Line::from(vec![
            Span::raw(LEFT_PAD),
            Span::styled("Space", key_style),
            Span::styled(" History  ", label_style),
            Span::styled("Tab", key_style),
            Span::styled(" Mode  ", label_style),
            Span::styled("↑/↓/→/←", key_style),
            Span::styled(" Move  ", label_style),
            Span::styled("f", key_style),
            Span::styled(" Files  ", files_style),
            Span::styled("a", key_style),
            Span::styled(" All  ", hidden_style),
            Span::styled("Enter", key_style),
            Span::styled(" Select  ", label_style),
            Span::styled("q/Esc", key_style),
            Span::styled(" Quit", label_style),
        ]);

        self.render_guide(guide_area, guide_line, buf);
    }

    fn render_history(self, list_area: Rect, guide_area: Rect, buf: &mut Buffer) {
        let key_style = Style::default()
            .fg(self.app.current_theme.border_fg.into())
            .add_modifier(Modifier::BOLD);
        let label_style = Style::default().fg(self.app.current_theme.border_style_soft.into());

        if self.app.history.entries.is_empty() {
            let empty_msg = Paragraph::new(Line::from(vec![
                Span::raw(LEFT_PAD),
                Span::styled(
                    "No history yet. Navigate directories with Enter to build history.",
                    label_style,
                ),
            ]));
            empty_msg.render(list_area, buf);
        } else {
            let home = &self.app.home_dir;
            let selected_idx = self.app.history_list_state.selected();
            let mode_suffix = self.app.mode.suffix();
            let selected_style = Self::selected_style(self.app);
            let normal_style = Style::default().fg(Color::White);
            let index_style = Style::default().fg(self.app.current_theme.key_highlight.into());
            let mode_style = Style::default().fg(self.app.current_theme.border_fg.into());

            let items: Vec<ListItem> = self
                .app
                .history
                .entries
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    let is_selected = selected_idx == Some(i);
                    let display_path = entry
                        .path
                        .strip_prefix(home)
                        .map(|p| format!("~/{}", p.display()))
                        .unwrap_or_else(|_| entry.path.display().to_string());

                    let style = if is_selected {
                        selected_style
                    } else {
                        normal_style
                    };

                    let mut spans = vec![
                        Span::raw(LEFT_PAD),
                        Span::styled(format!("{:>3} ", i + 1), index_style),
                        Span::styled(display_path, style),
                    ];

                    if is_selected {
                        spans.push(Span::styled(mode_suffix, mode_style));
                    }

                    ListItem::new(Line::from(spans))
                })
                .collect();

            let list = List::new(items);
            StatefulWidget::render(list, list_area, buf, &mut self.app.history_list_state);
        }

        let guide_line = Line::from(vec![
            Span::raw(LEFT_PAD),
            Span::styled("Space/Esc", key_style),
            Span::styled(" Back  ", label_style),
            Span::styled("Tab", key_style),
            Span::styled(" Mode  ", label_style),
            Span::styled("j/k", key_style),
            Span::styled(" Navigate  ", label_style),
            Span::styled("Enter", key_style),
            Span::styled(" Select  ", label_style),
            Span::styled("q", key_style),
            Span::styled(" Quit", label_style),
        ]);

        self.render_guide(guide_area, guide_line, buf);
    }

    fn render_guide(self, guide_area_full: Rect, guide_line: Line, buf: &mut Buffer) {
        let guide_block = Block::default()
            .borders(Borders::TOP)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(self.app.current_theme.border_style.into()));
        let guide_inner = guide_block.inner(guide_area_full);
        guide_block.render(guide_area_full, buf);

        let guide = Paragraph::new(vec![guide_line]).wrap(Wrap { trim: false });
        guide.render(guide_inner, buf);
    }
}
