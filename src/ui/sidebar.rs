mod sgr;
mod tokens;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::Span,
};

pub(crate) use self::tokens::{
    agent_rows as sidebar_agent_rows, space_rows as sidebar_space_rows, AgentTokenContext,
    ResolvedToken, ResolvedTokenKind, SpaceTokenContext,
};
use super::text::{display_width, truncate_end};
use crate::app::state::Palette;
use crate::app::AppState;
use crate::detect::AgentState;
use crate::terminal::TerminalRuntimeRegistry;

pub(crate) struct AgentPanelEntry {
    pub ws_idx: usize,
    pub tab_idx: usize,
    pub pane_id: crate::layout::PaneId,
    pub agent_kind_label: Option<String>,
    pub state: AgentState,
    pub seen: bool,
    pub last_agent_state_change_seq: Option<u64>,
    pub tokens: std::collections::HashMap<String, String>,
}

fn sidebar_section_heights(total_height: u16, split_ratio: f32) -> (u16, u16) {
    if total_height == 0 {
        return (0, 0);
    }
    if total_height < 6 {
        let workspace_height = total_height.div_ceil(2);
        return (
            workspace_height,
            total_height.saturating_sub(workspace_height),
        );
    }

    let workspace_height = ((total_height as f32) * split_ratio.clamp(0.1, 0.9)).round() as u16;
    let workspace_height = workspace_height.clamp(3, total_height.saturating_sub(3));
    (
        workspace_height,
        total_height.saturating_sub(workspace_height),
    )
}

pub(crate) fn expanded_sidebar_sections(area: Rect, split_ratio: f32) -> (Rect, Rect) {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.is_empty() {
        return (Rect::default(), Rect::default());
    }

    let (workspace_height, detail_height) = sidebar_section_heights(content.height, split_ratio);
    (
        Rect::new(content.x, content.y, content.width, workspace_height),
        Rect::new(
            content.x,
            content.y + workspace_height,
            content.width,
            detail_height,
        ),
    )
}

pub(crate) fn sidebar_section_divider_rect(area: Rect, split_ratio: f32) -> Rect {
    let content = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
    if content.width == 0 || content.height < 6 {
        return Rect::default();
    }

    let (workspace_height, _) = sidebar_section_heights(content.height, split_ratio);
    Rect::new(content.x, content.y + workspace_height, content.width, 1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkspaceListEntry {
    Workspace { ws_idx: usize, indented: bool },
}

/// Tree order matching the spaces sidebar: each worktree is spliced next to
/// its parent workspace instead of trailing wherever `app.workspaces`
/// happened to append it. Sidebar group-collapse state is TUI/client-owned,
/// so this always returns the fully expanded tree.
pub(crate) fn workspace_list_entries_expanded(app: &AppState) -> Vec<WorkspaceListEntry> {
    let mut members_by_key = std::collections::HashMap::<String, Vec<usize>>::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        if let Some(space) = ws.worktree_space() {
            members_by_key
                .entry(space.key.clone())
                .or_default()
                .push(ws_idx);
        }
    }
    let grouped_keys = members_by_key
        .iter()
        .filter(|(_, members)| {
            members.len() >= 2
                && members.iter().any(|idx| {
                    app.workspaces
                        .get(*idx)
                        .and_then(|ws| ws.worktree_space())
                        .is_some_and(|space| !space.is_linked_worktree)
                })
        })
        .map(|(key, _)| key.clone())
        .collect::<std::collections::HashSet<_>>();

    let mut emitted_groups = std::collections::HashSet::<String>::new();
    let mut entries = Vec::new();
    for (ws_idx, ws) in app.workspaces.iter().enumerate() {
        let Some(space) = ws
            .worktree_space()
            .filter(|space| grouped_keys.contains(&space.key))
        else {
            entries.push(WorkspaceListEntry::Workspace {
                ws_idx,
                indented: false,
            });
            continue;
        };

        if !emitted_groups.insert(space.key.clone()) {
            continue;
        }

        let Some(members) = members_by_key.get(&space.key) else {
            continue;
        };
        let Some(parent_idx) = members.iter().copied().find(|idx| {
            app.workspaces
                .get(*idx)
                .and_then(|member| member.worktree_space())
                .is_some_and(|member_space| !member_space.is_linked_worktree)
        }) else {
            entries.push(WorkspaceListEntry::Workspace {
                ws_idx,
                indented: false,
            });
            continue;
        };
        entries.push(WorkspaceListEntry::Workspace {
            ws_idx: parent_idx,
            indented: false,
        });

        for member_idx in members {
            if *member_idx == parent_idx {
                continue;
            }
            entries.push(WorkspaceListEntry::Workspace {
                ws_idx: *member_idx,
                indented: true,
            });
        }
    }
    entries
}

#[cfg(test)]
pub(crate) fn agent_panel_entries(app: &AppState) -> Vec<AgentPanelEntry> {
    agent_panel_entries_from(app, &TerminalRuntimeRegistry::new())
}

pub(crate) fn agent_panel_entries_from(
    app: &AppState,
    _terminal_runtimes: &TerminalRuntimeRegistry,
) -> Vec<AgentPanelEntry> {
    let mut entries = app
        .workspaces
        .iter()
        .enumerate()
        .flat_map(|(ws_idx, workspace)| {
            workspace
                .pane_details(&app.terminals)
                .into_iter()
                .map(move |detail| AgentPanelEntry {
                    ws_idx,
                    tab_idx: detail.tab_idx,
                    pane_id: detail.pane_id,
                    agent_kind_label: detail.agent_kind_label,
                    state: detail.state,
                    seen: detail.seen,
                    last_agent_state_change_seq: detail.last_agent_state_change_seq,
                    tokens: detail.tokens,
                })
        })
        .collect();
    crate::app::agent_view::apply_agent_view(app, &mut entries);
    entries
}

pub(crate) fn resolved_token_spans(
    resolved: &[ResolvedToken],
    state_icon: (&str, Style),
    state_text_style: Style,
    workspace_style: Style,
    secondary_style: Style,
    custom_style: Style,
    palette: &Palette,
    max_width: usize,
) -> Vec<Span<'static>> {
    let fixed_widths = resolved
        .iter()
        .map(|token| match &token.kind {
            ResolvedTokenKind::StateIcon => display_width(state_icon.0),
            ResolvedTokenKind::GitStatus { ahead, behind } => {
                usize::from(*ahead > 0) * display_width(&format!("↑{ahead}"))
                    + usize::from(*behind > 0) * display_width(&format!("↓{behind}"))
                    + usize::from(*ahead > 0 && *behind > 0)
            }
            _ => 0,
        })
        .collect::<Vec<_>>();
    let custom_segments = resolved
        .iter()
        .map(|token| match &token.kind {
            ResolvedTokenKind::Custom(text) => sgr::parse_sgr_segments(text),
            _ => None,
        })
        .collect::<Vec<_>>();
    let flexible_widths = resolved
        .iter()
        .enumerate()
        .map(|(index, token)| match &token.kind {
            ResolvedTokenKind::StateText(text)
            | ResolvedTokenKind::Machine(text)
            | ResolvedTokenKind::Workspace(text)
            | ResolvedTokenKind::Tab(text)
            | ResolvedTokenKind::Pane(text)
            | ResolvedTokenKind::Agent(text)
            | ResolvedTokenKind::TerminalTitle(text)
            | ResolvedTokenKind::Branch(text) => display_width(text),
            ResolvedTokenKind::Custom(text) => custom_segments[index].as_ref().map_or_else(
                || display_width(text),
                |segments| display_width(&sgr::plain_text(segments)),
            ),
            _ => 0,
        })
        .collect::<Vec<_>>();
    let minimum_width = |active: &[bool]| {
        let indices = active
            .iter()
            .enumerate()
            .filter_map(|(index, active)| active.then_some(index))
            .collect::<Vec<_>>();
        let content = indices
            .iter()
            .map(|index| fixed_widths[*index] + usize::from(flexible_widths[*index] > 0))
            .sum::<usize>();
        let separators = indices
            .windows(2)
            .map(|pair| display_width(tokens::separator(&resolved[pair[0]], &resolved[pair[1]])))
            .sum::<usize>();
        content + separators
    };
    let mut active = resolved.iter().map(|_| true).collect::<Vec<_>>();
    if minimum_width(&active) > max_width {
        for (index, width) in flexible_widths.iter().enumerate() {
            if *width > 0 {
                active[index] = false;
            }
        }
        for index in (0..resolved.len()).rev() {
            if flexible_widths[index] == 0 {
                continue;
            }
            active[index] = true;
            if minimum_width(&active) > max_width {
                active[index] = false;
            }
        }
    }
    let visible_indices = active
        .iter()
        .enumerate()
        .filter_map(|(index, active)| active.then_some(index))
        .collect::<Vec<_>>();
    let separator_width = visible_indices
        .windows(2)
        .map(|pair| display_width(tokens::separator(&resolved[pair[0]], &resolved[pair[1]])))
        .sum::<usize>();
    let fixed_width = visible_indices
        .iter()
        .map(|index| fixed_widths[*index])
        .sum::<usize>();
    let mut budgets = flexible_widths
        .iter()
        .enumerate()
        .map(|(index, width)| usize::from(active[index] && *width > 0))
        .collect::<Vec<_>>();
    let minimum = budgets.iter().sum::<usize>();
    let mut remaining = max_width
        .saturating_sub(separator_width + fixed_width)
        .saturating_sub(minimum);
    while remaining > 0 {
        let mut grew = false;
        for (budget, width) in budgets.iter_mut().zip(&flexible_widths) {
            if *budget > 0 && *budget < *width {
                *budget += 1;
                remaining -= 1;
                grew = true;
                if remaining == 0 {
                    break;
                }
            }
        }
        if !grew {
            break;
        }
    }

    let mut spans = Vec::new();
    for (position, index) in visible_indices.iter().copied().enumerate() {
        let token = &resolved[index];
        if position > 0 {
            let previous = &resolved[visible_indices[position - 1]];
            spans.push(Span::styled(
                tokens::separator(previous, token),
                Style::default().fg(palette.overlay0),
            ));
        }
        match &token.kind {
            ResolvedTokenKind::StateIcon => spans.push(Span::styled(
                state_icon.0.to_string(),
                apply_token_style(state_icon.1, token.style),
            )),
            ResolvedTokenKind::StateText(text) => spans.push(Span::styled(
                truncate_end(text, budgets[index]),
                apply_token_style(state_text_style, token.style),
            )),
            ResolvedTokenKind::Workspace(text) => spans.push(Span::styled(
                truncate_end(text, budgets[index]),
                apply_token_style(workspace_style, token.style),
            )),
            ResolvedTokenKind::Machine(text)
            | ResolvedTokenKind::Tab(text)
            | ResolvedTokenKind::Pane(text)
            | ResolvedTokenKind::Agent(text)
            | ResolvedTokenKind::Branch(text) => spans.push(Span::styled(
                truncate_end(text, budgets[index]),
                apply_token_style(secondary_style, token.style),
            )),
            ResolvedTokenKind::GitStatus { ahead, behind } => {
                if *ahead > 0 {
                    spans.push(Span::styled(
                        format!("↑{ahead}"),
                        apply_token_style(Style::default().fg(palette.green), token.style),
                    ));
                }
                if *ahead > 0 && *behind > 0 {
                    spans.push(Span::styled(
                        " ",
                        apply_token_style(Style::default(), token.style),
                    ));
                }
                if *behind > 0 {
                    spans.push(Span::styled(
                        format!("↓{behind}"),
                        apply_token_style(Style::default().fg(palette.red), token.style),
                    ));
                }
            }
            ResolvedTokenKind::TerminalTitle(text) => {
                spans.push(Span::styled(
                    truncate_end(text, budgets[index]),
                    apply_token_style(custom_style, token.style),
                ));
            }
            ResolvedTokenKind::Custom(text) => match &custom_segments[index] {
                Some(segments) => {
                    for (segment_text, segment_style) in
                        sgr::truncate_segments(segments, budgets[index])
                    {
                        spans.push(Span::styled(
                            segment_text,
                            apply_token_style(custom_style.patch(segment_style), token.style),
                        ));
                    }
                }
                None => {
                    spans.push(Span::styled(
                        truncate_end(text, budgets[index]),
                        apply_token_style(custom_style, token.style),
                    ));
                }
            },
        }
    }
    spans
}

fn apply_token_style(mut style: Style, patch: crate::config::SidebarTokenStyle) -> Style {
    if let Some(foreground) = patch.fg {
        style = style.fg(foreground.ratatui());
    }
    if let Some(bold) = patch.bold {
        style = if bold {
            style.add_modifier(Modifier::BOLD)
        } else {
            style.remove_modifier(Modifier::BOLD)
        };
    }
    if let Some(dim) = patch.dim {
        style = if dim {
            style.add_modifier(Modifier::DIM)
        } else {
            style.remove_modifier(Modifier::DIM)
        };
    }
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn occurrence_foreground_flattens_composite_git_status_colors() {
        let config: crate::config::Config = toml::from_str(
            r##"[ui.sidebar.spaces]
rows = [[{ token = "git_status", fg = "#123456" }]]
"##,
        )
        .unwrap();
        let spans = resolved_token_spans(
            &[ResolvedToken {
                kind: ResolvedTokenKind::GitStatus {
                    ahead: 2,
                    behind: 1,
                },
                style: config.ui.sidebar.spaces.rows[0][0].parts().1,
            }],
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default(),
            &crate::app::state::AppState::test_new().palette,
            20,
        );

        assert_eq!(
            spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>(),
            "↑2 ↓1"
        );
        assert!(spans
            .iter()
            .all(|span| { span.style.fg == Some(ratatui::style::Color::Rgb(0x12, 0x34, 0x56)) }));
    }

    #[test]
    fn custom_token_with_embedded_sgr_renders_as_multiple_styled_spans() {
        let spans = resolved_token_spans(
            &[ResolvedToken::unstyled(ResolvedTokenKind::Custom(
                "\x1b[31mred\x1b[32mgreen".into(),
            ))],
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default(),
            &crate::app::state::AppState::test_new().palette,
            20,
        );

        assert_eq!(
            spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<Vec<_>>(),
            vec!["red", "green"]
        );
        assert_eq!(spans[0].style.fg, Some(ratatui::style::Color::Indexed(1)));
        assert_eq!(spans[1].style.fg, Some(ratatui::style::Color::Indexed(2)));
    }

    #[test]
    fn custom_token_config_style_overrides_embedded_sgr_color() {
        let config: crate::config::Config = toml::from_str(
            r##"[ui.sidebar.agents]
rows = [[{ token = "$summary", fg = "#123456" }]]
"##,
        )
        .unwrap();
        let spans = resolved_token_spans(
            &[ResolvedToken {
                kind: ResolvedTokenKind::Custom("\x1b[31mred".into()),
                style: config.ui.sidebar.agents.rows[0][0].parts().1,
            }],
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default(),
            &crate::app::state::AppState::test_new().palette,
            20,
        );

        assert_eq!(
            spans[0].style.fg,
            Some(ratatui::style::Color::Rgb(0x12, 0x34, 0x56))
        );
    }

    #[test]
    fn custom_token_sgr_escape_bytes_do_not_count_toward_width_budget() {
        let spans = resolved_token_spans(
            &[ResolvedToken::unstyled(ResolvedTokenKind::Custom(
                "\x1b[38;2;1;2;3mhello world".into(),
            ))],
            ("", Style::default()),
            Style::default(),
            Style::default(),
            Style::default(),
            Style::default(),
            &crate::app::state::AppState::test_new().palette,
            8,
        );

        let text = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(text, "hello w…");
        assert_eq!(display_width(&text), 8);
    }
}
