pub mod detail;
pub mod encoding;
pub mod list;
pub mod popup;
pub mod status_bar;
pub mod theme;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::Style;
use ratatui::widgets::{Block, Widget};
use ratatui::Frame;

use crate::app::{ActiveView, App, ListRow};

pub fn draw(f: &mut Frame, app: &mut App) {
    // Fill entire background
    let bg = Block::default().style(Style::default().bg(theme::BG));
    bg.render(f.area(), f.buffer_mut());

    let inner = f.area();

    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(1)])
        .split(inner);

    match app.active_view {
        ActiveView::List => render_list_view(f, app, main_chunks[0]),
        ActiveView::Encoding => encoding::render_encoding_view(f, app, main_chunks[0]),
    }

    status_bar::render_status_bar(f, app, main_chunks[1]);

    // Render popup overlays last (on top of everything)
    if let Some(dialog) = &app.cleanup_dialog {
        popup::render_cleanup_dialog(f, dialog);
    }
    if let Some(picker) = &app.preset_picker {
        popup::render_preset_picker(f, picker, &app.presets, app.selected_preset, app.active_view);
    }
    if app.show_legend {
        popup::render_legend_popup(f, app);
    }
    if app.show_quit_confirm {
        popup::render_quit_confirm(f, app);
    }
}

fn render_list_view(f: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    if app.detail_open {
        // Snapshot the selected row so the immutable borrow ends before rendering the list
        let selected_row = app.selected_row();
        let detail_payload: Option<detail::DetailPayload> = match selected_row {
            Some(ListRow::Media(i)) => app
                .files
                .get(i)
                .cloned()
                .map(detail::DetailPayload::File),
            Some(ListRow::Folder(i)) => app
                .folders
                .get(i)
                .cloned()
                .map(detail::DetailPayload::Folder),
            None => None,
        };

        if let Some(payload) = detail_payload {
            let content_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(area);

            let list_active = !app.detail_focused;
            let detail_active = app.detail_focused;

            app.detail_view_height = content_chunks[1].height.saturating_sub(2);

            list::render_file_list(f, app, content_chunks[0], list_active);
            detail::render_detail(
                f,
                &payload,
                &app.root_path,
                content_chunks[1],
                detail_active,
                app.detail_scroll,
            );
        } else {
            list::render_file_list(f, app, area, true);
        }
    } else {
        list::render_file_list(f, app, area, true);
    }
}
