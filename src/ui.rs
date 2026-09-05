use crate::app::{App, FileNode, name_match_ranges};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, List, ListItem, Paragraph, StatefulWidget, Widget, Wrap,
    },
};

/// Preferred inner width of the top-right search field (excluding brackets).
const SEARCH_FIELD_WIDTH: usize = 16;
/// `" CDTREE "` as rendered on the left of the outer block.
const LEFT_TITLE_WIDTH: usize = 9;
/// `" Find ["` + `"] "` around the field.
const SEARCH_FORM_CHROME: usize = 9;

const LEFT_PAD: &str = "  ";

pub struct TreeWidget<'a> {
    app: &'a mut App,
}

// NOTE: The list content area below (double-bordered outer block + vertical
// [Min(1), Length(2)]) is mirrored by `list_content_area` in main.rs for mouse
// hit-testing. If you change the border type, borders, or layout constraints
// here, update `list_content_area` to match.

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

        let mut outer_block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::default().fg(self.app.current_theme.border_style.into()))
            .title(title);
        if self.app.search_mode {
            let label_style = Style::default()
                .fg(self.app.current_theme.key_highlight.into())
                .add_modifier(Modifier::BOLD);
            let field_style = Style::default()
                .bg(self.app.current_theme.key_highlight.into())
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD);
            outer_block = outer_block.title(Self::search_form_title(
                &self.app.search_query,
                area.width,
                label_style,
                field_style,
            ));
        }
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

    fn search_field_width(area_width: u16) -> usize {
        let available = (area_width as usize)
            .saturating_sub(2) // left/right border corners
            .saturating_sub(LEFT_TITLE_WIDTH)
            .saturating_sub(2) // gap between left title and form
            .saturating_sub(SEARCH_FORM_CHROME);
        available.clamp(4, SEARCH_FIELD_WIDTH)
    }

    /// Visible contents of the search field, with a trailing cursor and right padding.
    fn search_field_contents(query: &str, field_width: usize) -> String {
        let budget = field_width.saturating_sub(1);
        let visible = if query.chars().count() <= budget {
            query.to_string()
        } else {
            let skip = query.chars().count() - budget;
            query.chars().skip(skip).collect()
        };
        let used = visible.chars().count() + 1;
        let pad = field_width.saturating_sub(used);
        format!("{visible}_{}", " ".repeat(pad))
    }

    fn search_form_line(
        query: &str,
        field_width: usize,
        label_style: Style,
        field_style: Style,
    ) -> Line<'static> {
        Line::from(vec![
            Span::styled(" Find ", label_style),
            Span::styled(
                format!("[{}]", Self::search_field_contents(query, field_width)),
                field_style,
            ),
            Span::raw(" "),
        ])
    }

    fn search_form_title(
        query: &str,
        area_width: u16,
        label_style: Style,
        field_style: Style,
    ) -> Line<'static> {
        Self::search_form_line(
            query,
            Self::search_field_width(area_width),
            label_style,
            field_style,
        )
        .right_aligned()
    }

    fn search_match_style() -> Style {
        Style::default()
            .bg(Color::Yellow)
            .fg(Color::Black)
            .add_modifier(Modifier::BOLD)
    }

    fn name_spans(
        name: &str,
        query: &str,
        name_style: Style,
        match_style: Style,
    ) -> Vec<Span<'static>> {
        let ranges = name_match_ranges(name, query);
        if ranges.is_empty() {
            return vec![Span::styled(name.to_string(), name_style)];
        }

        let mut spans = Vec::new();
        let mut last = 0;
        for &(start, end) in &ranges {
            if start > last {
                spans.push(Span::styled(name[last..start].to_string(), name_style));
            }
            spans.push(Span::styled(name[start..end].to_string(), match_style));
            last = end;
        }
        if last < name.len() {
            spans.push(Span::styled(name[last..].to_string(), name_style));
        }
        spans
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
                let query = if self.app.search_mode {
                    self.app.search_query.as_str()
                } else {
                    ""
                };

                let mut spans = vec![
                    Span::raw(LEFT_PAD),
                    Span::styled(
                        prefix,
                        Style::default().fg(self.app.current_theme.branch_color.into()),
                    ),
                ];
                spans.extend(Self::name_spans(
                    &display_name,
                    query,
                    name_style,
                    Self::search_match_style(),
                ));

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

        let guide_line = if self.app.search_mode {
            Line::from(vec![
                Span::raw(LEFT_PAD),
                Span::styled("Esc", key_style),
                Span::styled(" Exit  ", label_style),
                Span::styled("Enter", key_style),
                Span::styled(" Select  ", label_style),
                Span::styled("↑/↓/→/←", key_style),
                Span::styled(" Move", label_style),
            ])
        } else {
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

            Line::from(vec![
                Span::raw(LEFT_PAD),
                Span::styled("Space", key_style),
                Span::styled(" History  ", label_style),
                Span::styled("Tab", key_style),
                Span::styled(" Mode  ", label_style),
                Span::styled("↑/↓/→/←", key_style),
                Span::styled(" Move  ", label_style),
                Span::styled("f", key_style),
                Span::styled(" Find  ", label_style),
                Span::styled("v", key_style),
                Span::styled(" Files  ", files_style),
                Span::styled("a", key_style),
                Span::styled(" All  ", hidden_style),
                Span::styled("Enter", key_style),
                Span::styled(" Select  ", label_style),
                Span::styled("q/Esc", key_style),
                Span::styled(" Quit", label_style),
            ])
        };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_spans_splits_matched_substrings() {
        let style = Style::default();
        let match_style = Style::default().fg(Color::Yellow);
        let spans = TreeWidget::name_spans("ReadMe", "ad", style, match_style);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "Re");
        assert_eq!(spans[1].content.as_ref(), "ad");
        assert_eq!(spans[2].content.as_ref(), "Me");
        assert_eq!(spans[1].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn name_spans_without_query_is_a_single_span() {
        let style = Style::default().fg(Color::White);
        let match_style = Style::default().fg(Color::Yellow);
        let spans = TreeWidget::name_spans("src", "", style, match_style);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "src");
        assert_eq!(spans[0].style.fg, Some(Color::White));
    }

    #[test]
    fn name_spans_without_match_keeps_name_style() {
        let style = Style::default().fg(Color::White);
        let match_style = Style::default().fg(Color::Yellow);
        let spans = TreeWidget::name_spans("src", "zzz", style, match_style);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "src");
        assert_eq!(spans[0].style.fg, Some(Color::White));
    }

    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn search_field_pads_empty_query_to_form_width() {
        let contents = TreeWidget::search_field_contents("", 12);
        assert_eq!(contents, "_           ");
        assert_eq!(contents.chars().count(), 12);
    }

    #[test]
    fn search_field_keeps_cursor_after_query() {
        let contents = TreeWidget::search_field_contents("src", 12);
        assert_eq!(contents, "src_        ");
    }

    #[test]
    fn search_field_keeps_the_tail_of_a_long_query() {
        let contents = TreeWidget::search_field_contents("abcdefghijklmnop", 8);
        assert_eq!(contents, "jklmnop_");
    }

    #[test]
    fn search_form_line_wraps_the_field_in_brackets() {
        let line = TreeWidget::search_form_line(
            "src",
            12,
            Style::default(),
            Style::default().fg(Color::Yellow),
        );
        assert_eq!(line_text(&line), " Find [src_        ] ");
        assert_eq!(line.spans[1].style.fg, Some(Color::Yellow));
    }

    #[test]
    fn search_form_title_is_right_aligned() {
        use ratatui::layout::Alignment;
        let line = TreeWidget::search_form_title("src", 80, Style::default(), Style::default());
        assert_eq!(line.alignment, Some(Alignment::Right));
        assert!(line_text(&line).starts_with(" Find ["));
        assert!(line_text(&line).contains("src_"));
    }

    #[test]
    fn search_field_width_is_stable_on_normal_terminals() {
        assert_eq!(TreeWidget::search_field_width(80), SEARCH_FIELD_WIDTH);
        assert_eq!(TreeWidget::search_field_width(40), SEARCH_FIELD_WIDTH);
        // 24 - 2 (corners) - 9 (left title) - 2 (gap) - 9 (chrome) = 2, clamped to 4
        assert_eq!(TreeWidget::search_field_width(24), 4);
    }
}
