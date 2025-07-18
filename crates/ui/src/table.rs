use std::{ops::Range, rc::Rc, time::Duration};

use crate::{
    actions::{Cancel, SelectNext, SelectPrev},
    context_menu::ContextMenuExt,
    h_flex,
    popup_menu::PopupMenu,
    scroll::{self, ScrollableMask, Scrollbar, ScrollbarState},
    v_flex, ActiveTheme, Icon, IconName, Sizable, Size, StyleSized as _, StyledExt,
};
use gpui::{
    actions, canvas, div, prelude::FluentBuilder, px, uniform_list, App, AppContext, Axis, Bounds,
    Context, Div, DragMoveEvent, Edges, Empty, EntityId, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyBinding, ListSizingBehavior, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point, Render, ScrollHandle, ScrollStrategy, ScrollWheelEvent,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled, Task, UniformListScrollHandle,
    Window,
};

mod loading;

actions!(table, [SelectPrevColumn, SelectNextColumn]);

pub fn init(cx: &mut App) {
    let context = Some("Table");
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, context),
        KeyBinding::new("up", SelectPrev, context),
        KeyBinding::new("down", SelectNext, context),
        KeyBinding::new("left", SelectPrevColumn, context),
        KeyBinding::new("right", SelectNextColumn, context),
    ]);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColFixed {
    Left,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ColGroup {
    pub(crate) width: Pixels,
    pub(crate) bounds: Bounds<Pixels>,
    pub(crate) sort: Option<ColSort>,
    pub(crate) fixed: Option<ColFixed>,
    pub(crate) paddings: Option<Edges<Pixels>>,
}

#[derive(Clone)]
pub(crate) struct DragCol {
    pub(crate) entity_id: EntityId,
    pub(crate) name: SharedString,
    pub(crate) width: Pixels,
    pub(crate) col_ix: usize,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ColSort {
    /// No sorting.
    Default,
    /// Sort in ascending order.
    Ascending,
    /// Sort in descending order.
    Descending,
}

impl Render for DragCol {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_4()
            .py_1()
            .bg(cx.theme().table_head)
            .text_color(cx.theme().muted_foreground)
            .opacity(0.9)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_md()
            .w(self.width)
            .min_w(px(100.))
            .max_w(px(450.))
            .child(self.name.clone())
    }
}

#[derive(Clone)]
pub struct ResizeCol(pub (EntityId, usize));
impl Render for ResizeCol {
    fn render(&mut self, _window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SelectionState {
    Column,
    Row,
}

#[derive(Clone)]
pub enum TableEvent {
    /// Single click or move to selected row.
    SelectRow(usize),
    /// Double click on the row.
    DoubleClickedRow(usize),
    SelectCol(usize),
    ColWidthsChanged(Vec<Pixels>),
    MoveCol(usize, usize),
}

#[derive(Clone, Copy, Default)]
struct FixedCols {
    left: usize,
}

/// The visible range of the rows and columns.
#[derive(Debug, Default)]
pub struct VisibleRangeState {
    /// The visible range of the rows.
    rows: Range<usize>,
    /// The visible range of the columns.
    cols: Range<usize>,
}

impl VisibleRangeState {
    /// Returns the visible range of the rows.
    pub fn rows(&self) -> Range<usize> {
        self.rows.clone()
    }

    /// Returns the visible range of the columns.
    pub fn cols(&self) -> Range<usize> {
        self.cols.clone()
    }
}

pub struct Table<D: TableDelegate> {
    focus_handle: FocusHandle,
    delegate: D,
    /// The bounds of the table container.
    bounds: Bounds<Pixels>,
    /// The bounds of the fixed head cols.
    fixed_head_cols_bounds: Bounds<Pixels>,

    col_groups: Vec<ColGroup>,
    fixed_cols: FixedCols,

    /// Whether the table can loop selection, default is true.
    ///
    /// When the prev/next selection is out of the table bounds, the selection will loop to the other side.
    pub loop_selection: bool,

    pub vertical_scroll_handle: UniformListScrollHandle,
    pub vertical_scroll_state: ScrollbarState,
    pub horizontal_scroll_handle: ScrollHandle,
    pub horizontal_scroll_state: ScrollbarState,

    scrollbar_visible: Edges<bool>,
    selected_row: Option<usize>,
    selection_state: SelectionState,
    right_clicked_row: Option<usize>,
    selected_col: Option<usize>,

    /// The column index that is being resized.
    resizing_col: Option<usize>,

    /// Set stripe style of the table.
    stripe: bool,
    /// Set to use border style of the table.
    border: bool,
    /// The cell size of the table.
    size: Size,
    /// The visible range of the rows and columns.
    visible_range: VisibleRangeState,

    _measure: Vec<Duration>,
    _load_more_task: Task<()>,
}

#[allow(unused)]
pub trait TableDelegate: Sized + 'static {
    /// Return the number of columns in the table.
    fn cols_count(&self, cx: &App) -> usize;
    /// Return the number of rows in the table.
    fn rows_count(&self, cx: &App) -> usize;

    /// Returns the name of the column at the given index.
    fn col_name(&self, col_ix: usize, cx: &App) -> SharedString;

    /// Returns whether the column at the given index can be resized. Default: true
    fn col_resizable(&self, col_ix: usize, cx: &App) -> bool {
        true
    }

    /// Returns whether the column at the given index can be selected. Default: false
    fn col_selectable(&self, col_ix: usize, cx: &App) -> bool {
        false
    }

    /// Returns the width of the column at the given index.
    /// Return None, use auto width.
    ///
    /// This is only called when the table initializes.
    ///
    /// Default: 100px
    fn col_width(&self, col_ix: usize, cx: &App) -> Pixels {
        px(100.)
    }

    /// Return the sort state of the column at the given index.
    ///
    /// This is only called when the table initializes.
    fn col_sort(&self, col_ix: usize, cx: &App) -> Option<ColSort> {
        None
    }

    /// Return the fixed side of the column at the given index.
    fn col_fixed(&self, col_ix: usize, cx: &App) -> Option<ColFixed> {
        None
    }

    /// Return the padding of the column at the given index to override the default padding.
    ///
    /// Return None, use the default padding.
    fn col_paddings(&self, col_ix: usize, cx: &App) -> Option<Edges<Pixels>> {
        None
    }

    /// Return true to enable column order change.
    fn col_movable(&self, col_ix: usize, cx: &App) -> bool {
        false
    }

    /// Perform sort on the column at the given index.
    fn perform_sort(
        &mut self,
        col_ix: usize,
        sort: ColSort,
        window: &mut Window,
        cx: &mut Context<Table<Self>>,
    ) {
    }

    /// Render the header cell at the given column index, default to the column name.
    fn render_th(
        &self,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<Table<Self>>,
    ) -> impl IntoElement {
        div().size_full().child(self.col_name(col_ix, cx))
    }

    /// Render the row at the given row and column.
    fn render_tr(
        &self,
        row_ix: usize,
        window: &mut Window,
        cx: &mut Context<Table<Self>>,
    ) -> Stateful<Div> {
        h_flex().id(("table-row", row_ix))
    }

    /// Render the context menu for the row at the given row index.
    fn context_menu(&self, row_ix: usize, menu: PopupMenu, window: &Window, cx: &App) -> PopupMenu {
        menu
    }

    /// Render cell at the given row and column.
    fn render_td(
        &self,
        row_ix: usize,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<Table<Self>>,
    ) -> impl IntoElement;

    /// Move the column at the given `col_ix` to insert before the column at the given `to_ix`.
    fn move_col(
        &mut self,
        col_ix: usize,
        to_ix: usize,
        window: &mut Window,
        cx: &mut Context<Table<Self>>,
    ) {
    }

    /// Return a Element to show when table is empty.
    fn render_empty(&self, window: &mut Window, cx: &mut Context<Table<Self>>) -> impl IntoElement {
        h_flex()
            .size_full()
            .justify_center()
            .text_color(cx.theme().muted_foreground.opacity(0.6))
            .child(Icon::new(IconName::Inbox).size_12())
            .into_any_element()
    }

    /// Return true to show the loading view.
    fn loading(&self, cx: &App) -> bool {
        false
    }

    /// Return a Element to show when table is loading, default is built-in Skeleton loading view.
    ///
    /// The size is the size of the Table.
    fn render_loading(
        &self,
        size: Size,
        window: &mut Window,
        cx: &mut Context<Table<Self>>,
    ) -> impl IntoElement {
        loading::Loading::new().size(size)
    }

    /// Return true to enable load more data when scrolling to the bottom.
    ///
    /// Default: true
    fn can_load_more(&self, cx: &App) -> bool {
        true
    }

    /// Returns a threshold value (n rows), of course, when scrolling to the bottom,
    /// the remaining number of rows triggers `load_more`.
    /// This should smaller than the total number of first load rows.
    ///
    /// Default: 20 rows
    fn load_more_threshold(&self) -> usize {
        20
    }

    /// Load more data when the table is scrolled to the bottom.
    ///
    /// This will performed in a background task.
    ///
    /// This is always called when the table is near the bottom,
    /// so you must check if there is more data to load or lock the loading state.
    fn load_more(&mut self, window: &mut Window, cx: &mut Context<Table<Self>>) {}

    /// Render the last empty column, default to empty.
    fn render_last_empty_col(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Table<Self>>,
    ) -> impl IntoElement {
        h_flex().w_3().h_full().flex_shrink_0()
    }

    /// Called when the visible range of the rows changed.
    ///
    /// NOTE: Make sure this method is fast, because it will be called frequently.
    ///
    /// This can used to handle some data update, to only update the visible rows.
    /// Please ensure that the data is updated in the background task.
    fn visible_rows_changed(
        &mut self,
        visible_range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Table<Self>>,
    ) {
    }

    /// Called when the visible range of the columns changed.
    ///
    /// NOTE: Make sure this method is fast, because it will be called frequently.
    ///
    /// This can used to handle some data update, to only update the visible rows.
    /// Please ensure that the data is updated in the background task.
    fn visible_cols_changed(
        &mut self,
        visible_range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Table<Self>>,
    ) {
    }
}

impl<D> Table<D>
where
    D: TableDelegate,
{
    pub fn new(delegate: D, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            focus_handle: cx.focus_handle(),
            delegate,
            col_groups: Vec::new(),
            fixed_cols: FixedCols::default(),
            horizontal_scroll_handle: ScrollHandle::new(),
            vertical_scroll_handle: UniformListScrollHandle::new(),
            vertical_scroll_state: ScrollbarState::default(),
            horizontal_scroll_state: ScrollbarState::default(),
            selection_state: SelectionState::Row,
            selected_row: None,
            right_clicked_row: None,
            selected_col: None,
            resizing_col: None,
            bounds: Bounds::default(),
            fixed_head_cols_bounds: Bounds::default(),
            stripe: false,
            border: true,
            size: Size::default(),
            scrollbar_visible: Edges::all(true),
            visible_range: VisibleRangeState::default(),
            loop_selection: true,
            _load_more_task: Task::ready(()),
            _measure: Vec::new(),
        };

        this.prepare_col_groups(cx);
        this
    }

    pub fn delegate(&self) -> &D {
        &self.delegate
    }

    pub fn delegate_mut(&mut self) -> &mut D {
        &mut self.delegate
    }

    /// Set to use stripe style of the table, default to false.
    pub fn stripe(mut self, stripe: bool) -> Self {
        self.stripe = stripe;
        self
    }

    pub fn set_stripe(&mut self, stripe: bool, cx: &mut Context<Self>) {
        self.stripe = stripe;
        cx.notify();
    }

    /// Set to use border style of the table, default to true.
    pub fn border(mut self, border: bool) -> Self {
        self.border = border;
        self
    }

    /// Set to loop selection, default to true.
    pub fn loop_selection(mut self, loop_selection: bool) -> Self {
        self.loop_selection = loop_selection;
        self
    }

    /// Set the size to the table.
    pub fn set_size(&mut self, size: Size, cx: &mut Context<Self>) {
        self.size = size;
        cx.notify();
    }

    /// Get the size of the table.
    pub fn size(&self) -> Size {
        self.size
    }

    /// Set scrollbar visibility.
    pub fn scrollbar_visible(mut self, vertical: bool, horizontal: bool) -> Self {
        self.scrollbar_visible = Edges {
            right: vertical,
            bottom: horizontal,
            ..Default::default()
        };
        self
    }

    /// When we update columns or rows, we need to refresh the table.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.prepare_col_groups(cx);
    }

    fn prepare_col_groups(&mut self, cx: &mut Context<Self>) {
        self.col_groups = (0..self.delegate.cols_count(cx))
            .map(|col_ix| ColGroup {
                width: self.delegate.col_width(col_ix, cx),
                paddings: self.delegate.col_paddings(col_ix, cx),
                bounds: Bounds::default(),
                sort: self.delegate.col_sort(col_ix, cx),
                fixed: self.delegate.col_fixed(col_ix, cx),
            })
            .collect();
        self.fixed_cols.left = self
            .col_groups
            .iter()
            .filter(|col| col.fixed == Some(ColFixed::Left))
            .count();
        cx.notify();
    }

    /// Scroll to the row at the given index.
    pub fn scroll_to_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        self.vertical_scroll_handle
            .scroll_to_item(row_ix, ScrollStrategy::Top);
        cx.notify();
    }

    // Scroll to the column at the given index.
    // TODO: Fix scroll to selected col, this was not working after fixed col.
    // pub fn scroll_to_col(&mut self, col_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
    //     self.horizontal_scroll_handle.scroll_to_item(col_ix);
    //     cx.notify();
    // }

    /// Returns the selected row index.
    pub fn selected_row(&self) -> Option<usize> {
        self.selected_row
    }

    /// Sets the selected row to the given index.
    pub fn set_selected_row(&mut self, row_ix: usize, cx: &mut Context<Self>) {
        self.selection_state = SelectionState::Row;
        self.right_clicked_row = None;
        self.selected_row = Some(row_ix);
        if let Some(row_ix) = self.selected_row {
            self.vertical_scroll_handle
                .scroll_to_item(row_ix, ScrollStrategy::Top);
        }
        cx.emit(TableEvent::SelectRow(row_ix));
        cx.notify();
    }

    /// Returns the selected column index.
    pub fn selected_col(&self) -> Option<usize> {
        self.selected_col
    }

    /// Sets the selected col to the given index.
    pub fn set_selected_col(&mut self, col_ix: usize, cx: &mut Context<Self>) {
        self.selection_state = SelectionState::Column;
        self.selected_col = Some(col_ix);
        if let Some(_col_ix) = self.selected_col {
            // TODO: Fix scroll to selected col, this was not working after fixed col.
            // if self.col_groups[col_ix].fixed.is_none() {
            //     self.horizontal_scroll_handle.scroll_to_item(col_ix);
            // }
        }
        cx.emit(TableEvent::SelectCol(col_ix));
        cx.notify();
    }

    /// Clear the selection of the table.
    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection_state = SelectionState::Row;
        self.selected_row = None;
        self.selected_col = None;
        cx.notify();
    }

    /// Returns the visible range of the rows and columns.
    pub fn visible_range(&self) -> &VisibleRangeState {
        &self.visible_range
    }

    fn on_row_click(
        &mut self,
        ev: &MouseDownEvent,
        row_ix: usize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ev.button == MouseButton::Right {
            self.right_clicked_row = Some(row_ix);
        } else {
            self.set_selected_row(row_ix, cx);

            if ev.click_count == 2 {
                cx.emit(TableEvent::DoubleClickedRow(row_ix));
            }
        }
    }

    fn on_col_head_click(&mut self, col_ix: usize, _: &mut Window, cx: &mut Context<Self>) {
        if !self.delegate.col_selectable(col_ix, cx) {
            return;
        }

        self.set_selected_col(col_ix, cx)
    }

    fn action_cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        self.clear_selection(cx);
    }

    fn action_select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        let rows_count = self.delegate.rows_count(cx);
        if rows_count < 1 {
            return;
        }

        let mut selected_row = self.selected_row.unwrap_or(0);
        if selected_row > 0 {
            selected_row = selected_row.saturating_sub(1);
        } else {
            if self.loop_selection {
                selected_row = rows_count.saturating_sub(1);
            }
        }

        self.set_selected_row(selected_row, cx);
    }

    fn action_select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let rows_count = self.delegate.rows_count(cx);
        if rows_count < 1 {
            return;
        }

        let selected_row = match self.selected_row {
            Some(selected_row) if selected_row < rows_count.saturating_sub(1) => selected_row + 1,
            Some(selected_row) => {
                if self.loop_selection {
                    0
                } else {
                    selected_row
                }
            }
            _ => 0,
        };

        self.set_selected_row(selected_row, cx);
    }

    fn action_select_prev_col(
        &mut self,
        _: &SelectPrevColumn,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut selected_col = self.selected_col.unwrap_or(0);
        let cols_count = self.delegate.cols_count(cx);
        if selected_col > 0 {
            selected_col = selected_col.saturating_sub(1);
        } else {
            if self.loop_selection {
                selected_col = cols_count.saturating_sub(1);
            }
        }
        self.set_selected_col(selected_col, cx);
    }

    fn action_select_next_col(
        &mut self,
        _: &SelectNextColumn,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut selected_col = self.selected_col.unwrap_or(0);
        if selected_col < self.delegate.cols_count(cx).saturating_sub(1) {
            selected_col += 1;
        } else {
            if self.loop_selection {
                selected_col = 0;
            }
        }

        self.set_selected_col(selected_col, cx);
    }

    /// Scroll table when mouse position is near the edge of the table bounds.
    fn scroll_table_by_col_resizing(&mut self, mouse_position: Point<Pixels>, col_group: ColGroup) {
        // Do nothing if pos out of the table bounds right for avoid scroll to the right.
        if mouse_position.x > self.bounds.right() {
            return;
        }

        let mut offset = self.horizontal_scroll_handle.offset();
        let col_bounds = col_group.bounds;

        if mouse_position.x < self.bounds.left()
            && col_bounds.right() < self.bounds.left() + px(20.)
        {
            offset.x += px(1.);
        } else if mouse_position.x > self.bounds.right()
            && col_bounds.right() > self.bounds.right() - px(20.)
        {
            offset.x -= px(1.);
        }

        self.horizontal_scroll_handle.set_offset(offset);
    }

    /// The `ix`` is the index of the col to resize,
    /// and the `size` is the new size for the col.
    fn resize_cols(&mut self, ix: usize, size: Pixels, _: &mut Window, cx: &mut Context<Self>) {
        const MIN_WIDTH: Pixels = px(10.0);
        const MAX_WIDTH: Pixels = px(1200.0);

        if !self.delegate.col_resizable(ix, cx) {
            return;
        }
        let size = size.floor();

        let old_width = self.col_groups[ix].width;
        let new_width = size;
        if new_width < MIN_WIDTH {
            return;
        }
        let changed_width = new_width - old_width;
        // If change size is less than 1px, do nothing.
        if changed_width > px(-1.0) && changed_width < px(1.0) {
            return;
        }
        self.col_groups[ix].width = new_width.min(MAX_WIDTH);

        // Resize next col, table not need to resize the right cols.
        // let next_width = self.col_groups[ix + 1].width.unwrap_or_default();
        // let next_width = (next_width - changed_width).max(MIN_WIDTH);
        // self.col_groups[ix + 1].width = Some(next_width);

        cx.notify();
    }

    fn perform_sort(&mut self, col_ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let sort = self.col_groups.get(col_ix).and_then(|g| g.sort);
        if sort.is_none() {
            return;
        }

        let sort = sort.unwrap();
        let sort = match sort {
            ColSort::Ascending => ColSort::Default,
            ColSort::Descending => ColSort::Ascending,
            ColSort::Default => ColSort::Descending,
        };

        for (ix, col_group) in self.col_groups.iter_mut().enumerate() {
            if ix == col_ix {
                col_group.sort = Some(sort);
            } else {
                if col_group.sort.is_some() {
                    col_group.sort = Some(ColSort::Default);
                }
            }
        }

        self.delegate_mut().perform_sort(col_ix, sort, window, cx);

        cx.notify();
    }

    fn move_col(
        &mut self,
        col_ix: usize,
        to_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if col_ix == to_ix {
            return;
        }

        self.delegate.move_col(col_ix, to_ix, window, cx);
        let col_group = self.col_groups.remove(col_ix);
        self.col_groups.insert(to_ix, col_group);

        cx.emit(TableEvent::MoveCol(col_ix, to_ix));
        cx.notify();
    }

    /// Dispatch delegate's `load_more` method when the visible range is near the end.
    fn load_more_if_need(
        &mut self,
        rows_count: usize,
        visible_end: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let threshold = self.delegate.load_more_threshold();
        // Securely handle subtract logic to prevent attempt to subtract with overflow
        if visible_end >= rows_count.saturating_sub(threshold) {
            if !self.delegate.can_load_more(cx) {
                return;
            }

            self._load_more_task = cx.spawn_in(window, async move |view, window| {
                _ = view.update_in(window, |view, window, cx| {
                    view.delegate.load_more(window, cx);
                });
            });
        }
    }

    fn update_visible_range_if_need(
        &mut self,
        visible_range: Range<usize>,
        axis: Axis,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Skip when visible range is only 1 item.
        // The visual_list will use first item to measure.
        if visible_range.len() <= 1 {
            return;
        }

        if axis == Axis::Vertical {
            if self.visible_range.rows == visible_range {
                return;
            }
            self.delegate_mut()
                .visible_rows_changed(visible_range.clone(), window, cx);
            self.visible_range.rows = visible_range;
        } else {
            if self.visible_range.cols == visible_range {
                return;
            }
            self.delegate_mut()
                .visible_cols_changed(visible_range.clone(), window, cx);
            self.visible_range.cols = visible_range;
        }
    }

    #[inline]
    fn render_cell(&self, col_ix: usize, _window: &mut Window, _cx: &mut Context<Self>) -> Div {
        let Some(col_group) = self.col_groups.get(col_ix) else {
            return div();
        };

        let col_width = col_group.width;
        let col_padding = col_group.paddings;

        div()
            .w(col_width)
            .h_full()
            .flex_shrink_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .table_cell_size(self.size)
            .map(|this| match col_padding {
                Some(padding) => this
                    .pl(padding.left)
                    .pr(padding.right)
                    .pt(padding.top)
                    .pb(padding.bottom),
                None => this,
            })
    }

    /// Show Column selection style, when the column is selected and the selection state is Column.
    fn render_col_wrap(&self, col_ix: usize, _: &mut Window, cx: &mut Context<Self>) -> Div {
        let el = h_flex().h_full();

        if self.delegate().col_selectable(col_ix, cx)
            && self.selected_col == Some(col_ix)
            && self.selection_state == SelectionState::Column
        {
            el.bg(cx.theme().table_active)
        } else {
            el
        }
    }

    fn render_vertical_scrollbar(
        &self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let state = self.vertical_scroll_state.clone();

        Some(
            div()
                .occlude()
                .absolute()
                .top(self.size.table_row_height())
                .right_0()
                .bottom_0()
                .w(scroll::WIDTH)
                .on_scroll_wheel(cx.listener(|_, _: &ScrollWheelEvent, _, cx| {
                    cx.notify();
                }))
                .child(Scrollbar::uniform_scroll(&state, &self.vertical_scroll_handle).max_fps(60)),
        )
    }

    fn render_horizontal_scrollbar(
        &self,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let state = self.horizontal_scroll_state.clone();

        div()
            .occlude()
            .absolute()
            .left(self.fixed_head_cols_bounds.size.width)
            .right_0()
            .bottom_0()
            .h(scroll::WIDTH)
            .on_scroll_wheel(cx.listener(|_, _: &ScrollWheelEvent, _, cx| {
                cx.notify();
            }))
            .child(Scrollbar::horizontal(
                &state,
                &self.horizontal_scroll_handle,
            ))
    }

    fn render_resize_handle(
        &self,
        ix: usize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        const HANDLE_SIZE: Pixels = px(2.);

        if !self.delegate.col_resizable(ix, cx) {
            return div().into_any_element();
        }

        let group_id = SharedString::from(format!("resizable-handle:{}", ix));

        h_flex()
            .id(("resizable-handle", ix))
            .group(group_id.clone())
            .occlude()
            .cursor_col_resize()
            .h_full()
            .w(HANDLE_SIZE)
            .ml(-(HANDLE_SIZE))
            .justify_end()
            .items_center()
            .child(
                div()
                    .h_full()
                    .justify_center()
                    .bg(cx.theme().table_row_border)
                    .group_hover(group_id, |this| this.bg(cx.theme().border).h_full())
                    .w(px(1.)),
            )
            .on_drag_move(
                cx.listener(move |view, e: &DragMoveEvent<ResizeCol>, window, cx| {
                    match e.drag(cx) {
                        ResizeCol((entity_id, ix)) => {
                            if cx.entity_id() != *entity_id {
                                return;
                            }

                            // sync col widths into real widths
                            for (_, col_group) in view.col_groups.iter_mut().enumerate() {
                                col_group.width = col_group.bounds.size.width;
                            }

                            let ix = *ix;
                            view.resizing_col = Some(ix);

                            let col_group =
                                *view.col_groups.get(ix).expect("BUG: invalid col index");

                            view.resize_cols(
                                ix,
                                e.event.position.x - HANDLE_SIZE - col_group.bounds.left(),
                                window,
                                cx,
                            );

                            // scroll the table if the drag is near the edge
                            view.scroll_table_by_col_resizing(e.event.position, col_group);
                        }
                    };
                }),
            )
            .on_drag(ResizeCol((cx.entity_id(), ix)), |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    if view.resizing_col.is_none() {
                        return;
                    }

                    view.resizing_col = None;

                    let new_widths = view.col_groups.iter().map(|g| g.width).collect();
                    cx.emit(TableEvent::ColWidthsChanged(new_widths));
                    cx.notify();
                }),
            )
            .into_any_element()
    }

    fn render_sort_icon(
        &self,
        col_ix: usize,
        col_group: &ColGroup,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let Some(sort) = col_group.sort else {
            return None;
        };

        let (icon, is_on) = match sort {
            ColSort::Ascending => (IconName::SortAscending, true),
            ColSort::Descending => (IconName::SortDescending, true),
            ColSort::Default => (IconName::ChevronsUpDown, false),
        };

        Some(
            div()
                .id(("icon-sort", col_ix))
                .p(px(2.))
                .rounded(cx.theme().radius / 2.)
                .map(|this| match is_on {
                    true => this,
                    false => this.opacity(0.5),
                })
                .hover(|this| this.bg(cx.theme().secondary).opacity(7.))
                .active(|this| this.bg(cx.theme().secondary_active).opacity(1.))
                .on_click(
                    cx.listener(move |table, _, window, cx| table.perform_sort(col_ix, window, cx)),
                )
                .child(
                    Icon::new(icon)
                        .size_3()
                        .text_color(cx.theme().secondary_foreground),
                ),
        )
    }

    /// Render the column header.
    /// The children must be one by one items.
    /// Because the horizontal scroll handle will use the child_item_bounds to
    /// calculate the item position for itself's `scroll_to_item` method.
    fn render_th(
        &self,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity_id = cx.entity_id();
        let col_group = self.col_groups.get(col_ix).expect("BUG: invalid col index");
        let movable = self.delegate.col_movable(col_ix, cx);
        let paddings = self.delegate.col_paddings(col_ix, cx);
        let name = self.delegate.col_name(col_ix, cx);

        h_flex()
            .child(
                self.render_cell(col_ix, window, cx)
                    .id(("col-header", col_ix))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            this.on_col_head_click(col_ix, window, cx);
                        }),
                    )
                    .child(
                        h_flex()
                            .size_full()
                            .justify_between()
                            .items_center()
                            .child(self.delegate.render_th(col_ix, window, cx))
                            .when_some(paddings, |this, paddings| {
                                // Leave right space for the sort icon, if this column have custom padding
                                let offset_pr =
                                    self.size.table_cell_padding().right - paddings.right;
                                this.pr(offset_pr.max(px(0.)))
                            })
                            .children(self.render_sort_icon(col_ix, &col_group, window, cx)),
                    )
                    .when(movable, |this| {
                        this.on_drag(
                            DragCol {
                                entity_id,
                                col_ix,
                                name,
                                width: col_group.width,
                            },
                            |drag, _, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| drag.clone())
                            },
                        )
                        .drag_over::<DragCol>(|this, _, _, cx| {
                            this.rounded_l_none()
                                .border_l_2()
                                .border_r_0()
                                .border_color(cx.theme().drag_border)
                        })
                        .on_drop(cx.listener(
                            move |table, drag: &DragCol, window, cx| {
                                // If the drag col is not the same as the drop col, then swap the cols.
                                if drag.entity_id != cx.entity_id() {
                                    return;
                                }

                                table.move_col(drag.col_ix, col_ix, window, cx);
                            },
                        ))
                    }),
            )
            // resize handle
            .child(self.render_resize_handle(col_ix, window, cx))
            // to save the bounds of this col.
            .child({
                let view = cx.entity().clone();
                canvas(
                    move |bounds, _, cx| {
                        view.update(cx, |r, _| r.col_groups[col_ix].bounds = bounds)
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            })
    }

    fn render_table_head(
        &mut self,
        left_cols_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();
        let horizontal_scroll_handle = self.horizontal_scroll_handle.clone();

        // Reset fixed head columns bounds, if no fixed columns are present
        if left_cols_count == 0 {
            self.fixed_head_cols_bounds = Bounds::default();
        }

        h_flex()
            .w_full()
            .h(self.size.table_row_height())
            .flex_shrink_0()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_color(cx.theme().table_head_foreground)
            .when(left_cols_count > 0, |this| {
                let view = view.clone();
                // Render left fixed columns
                this.child(
                    h_flex()
                        .relative()
                        .h_full()
                        .bg(cx.theme().table_head)
                        .children(
                            self.col_groups
                                .iter()
                                .filter(|col| col.fixed == Some(ColFixed::Left))
                                .enumerate()
                                .map(|(col_ix, _)| self.render_th(col_ix, window, cx)),
                        )
                        .child(
                            // Fixed columns border
                            div()
                                .absolute()
                                .top_0()
                                .right_0()
                                .bottom_0()
                                .w_0()
                                .flex_shrink_0()
                                .border_r_1()
                                .border_color(cx.theme().border),
                        )
                        .child(
                            canvas(
                                move |bounds, _, cx| {
                                    view.update(cx, |r, _| r.fixed_head_cols_bounds = bounds)
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .size_full(),
                        ),
                )
            })
            .child(
                // Columns
                h_flex()
                    .id("table-head")
                    .size_full()
                    .overflow_scroll()
                    .relative()
                    .track_scroll(&horizontal_scroll_handle)
                    .bg(cx.theme().table_head)
                    .child(
                        h_flex()
                            .relative()
                            .children(
                                self.col_groups
                                    .iter()
                                    .filter(|col| col.fixed == None)
                                    .enumerate()
                                    .map(|(col_ix, _)| {
                                        self.render_th(left_cols_count + col_ix, window, cx)
                                    }),
                            )
                            .child(self.delegate.render_last_empty_col(window, cx)),
                    ),
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_table_row(
        &mut self,
        row_ix: usize,
        rows_count: usize,
        left_cols_count: usize,
        col_sizes: Rc<Vec<gpui::Size<Pixels>>>,
        cols_count: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let horizontal_scroll_handle = self.horizontal_scroll_handle.clone();
        let is_stripe_row = self.stripe && row_ix % 2 != 0;
        let is_selected = self.selected_row == Some(row_ix);
        let view = cx.entity().clone();

        if row_ix < rows_count {
            self.delegate
                .render_tr(row_ix, window, cx)
                .h_flex()
                .w_full()
                .h(self.size.table_row_height())
                .border_b_1()
                .when(row_ix == rows_count, |this| {
                    this.border_color(gpui::transparent_white())
                })
                .border_color(cx.theme().table_row_border)
                .when(is_stripe_row, |this| this.bg(cx.theme().table_even))
                .hover(|this| {
                    if is_selected || self.right_clicked_row == Some(row_ix) {
                        this
                    } else {
                        this.bg(cx.theme().table_hover)
                    }
                })
                .when(left_cols_count > 0, |this| {
                    // Left fixed columns
                    this.child(
                        h_flex()
                            .relative()
                            .h_full()
                            .children({
                                let mut items = Vec::with_capacity(left_cols_count);

                                (0..left_cols_count).for_each(|col_ix| {
                                    items.push(self.render_col_wrap(col_ix, window, cx).child(
                                        self.render_cell(col_ix, window, cx).child(
                                            self.measure_render_td(row_ix, col_ix, window, cx),
                                        ),
                                    ));
                                });

                                items
                            })
                            .child(
                                // Fixed columns border
                                div()
                                    .absolute()
                                    .top_0()
                                    .right_0()
                                    .bottom_0()
                                    .w_0()
                                    .flex_shrink_0()
                                    .border_r_1()
                                    .border_color(cx.theme().border),
                            ),
                    )
                })
                .child(
                    h_flex()
                        .flex_1()
                        .h_full()
                        .overflow_hidden()
                        .relative()
                        .child(
                            crate::virtual_list::virtual_list(
                                view,
                                row_ix,
                                Axis::Horizontal,
                                col_sizes,
                                {
                                    move |table, visible_range: Range<usize>, _, window, cx| {
                                        table.update_visible_range_if_need(
                                            visible_range.clone(),
                                            Axis::Horizontal,
                                            window,
                                            cx,
                                        );

                                        let mut items = Vec::with_capacity(
                                            visible_range.end - visible_range.start,
                                        );

                                        visible_range.for_each(|col_ix| {
                                            let col_ix = col_ix + left_cols_count;
                                            let el =
                                                table.render_col_wrap(col_ix, window, cx).child(
                                                    table.render_cell(col_ix, window, cx).child(
                                                        table.measure_render_td(
                                                            row_ix, col_ix, window, cx,
                                                        ),
                                                    ),
                                                );

                                            items.push(el);
                                        });

                                        items
                                    }
                                },
                            )
                            .with_scroll_handle(&self.horizontal_scroll_handle),
                        )
                        .child(self.delegate.render_last_empty_col(window, cx)),
                )
                // Row selected style
                .when_some(self.selected_row, |this, _| {
                    this.when(
                        is_selected && self.selection_state == SelectionState::Row,
                        |this| {
                            this.border_color(gpui::transparent_white()).child(
                                div()
                                    .top(if row_ix == 0 { px(0.) } else { px(-1.) })
                                    .left(px(0.))
                                    .right(px(0.))
                                    .bottom_0()
                                    .absolute()
                                    .bg(cx.theme().table_active)
                                    .border_1()
                                    .border_color(cx.theme().table_active_border),
                            )
                        },
                    )
                })
                // Row right click row style
                .when(self.right_clicked_row == Some(row_ix), |this| {
                    this.border_color(gpui::transparent_white()).child(
                        div()
                            .top(if row_ix == 0 { px(0.) } else { px(-1.) })
                            .left(px(0.))
                            .right(px(0.))
                            .bottom_0()
                            .absolute()
                            .border_1()
                            .border_color(cx.theme().selection),
                    )
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, ev, window, cx| {
                        this.on_row_click(ev, row_ix, window, cx);
                    }),
                )
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, ev, window, cx| {
                        this.on_row_click(ev, row_ix, window, cx);
                    }),
                )
        } else {
            // Render fake rows to fill the rest table space
            self.delegate
                .render_tr(row_ix, window, cx)
                .h_flex()
                .w_full()
                .h_full()
                .border_t_1()
                .border_color(cx.theme().table_row_border)
                .when(is_stripe_row, |this| this.bg(cx.theme().table_even))
                .children((0..cols_count).map(|col_ix| {
                    h_flex()
                        .left(horizontal_scroll_handle.offset().x)
                        .child(self.render_cell(col_ix, window, cx))
                }))
                .child(self.delegate.render_last_empty_col(window, cx))
        }
    }

    /// Calculate the extra rows needed to fill the table empty space when `stripe` is true.
    fn calculate_extra_rows_needed(&self, rows_count: usize) -> usize {
        if !self.stripe {
            return 0;
        }

        let mut extra_rows_needed = 0;

        let row_height = self.size.table_row_height();
        let total_height = self
            .vertical_scroll_handle
            .0
            .borrow()
            .base_handle
            .bounds()
            .size
            .height;

        let actual_height = row_height * rows_count as f32;
        let remaining_height = total_height - actual_height;

        if remaining_height > px(0.) {
            extra_rows_needed = (remaining_height / row_height).ceil() as usize;
        }

        extra_rows_needed
    }

    #[inline]
    fn measure_render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if !crate::measure_enable() {
            return self
                .delegate
                .render_td(row_ix, col_ix, window, cx)
                .into_any_element();
        }

        let start = std::time::Instant::now();
        let el = self.delegate.render_td(row_ix, col_ix, window, cx);
        self._measure.push(start.elapsed());
        el.into_any_element()
    }

    fn measure(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        if !crate::measure_enable() {
            return;
        }

        // Print avg measure time of each td
        if self._measure.len() > 0 {
            let total = self
                ._measure
                .iter()
                .fold(Duration::default(), |acc, d| acc + *d);
            let avg = total / self._measure.len() as u32;
            eprintln!(
                "last render {} cells total: {:?}, avg: {:?}",
                self._measure.len(),
                total,
                avg,
            );
        }
        self._measure.clear();
    }
}

impl<D> Sizable for Table<D>
where
    D: TableDelegate,
{
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}
impl<D> Focusable for Table<D>
where
    D: TableDelegate,
{
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}
impl<D> EventEmitter<TableEvent> for Table<D> where D: TableDelegate {}

impl<D> Render for Table<D>
where
    D: TableDelegate,
{
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.measure(window, cx);

        let view = cx.entity().clone();
        let vertical_scroll_handle = self.vertical_scroll_handle.clone();
        let horizontal_scroll_handle = self.horizontal_scroll_handle.clone();
        let cols_count: usize = self.delegate.cols_count(cx);
        let left_cols_count = self.fixed_cols.left;
        let rows_count = self.delegate.rows_count(cx);
        let loading = self.delegate.loading(cx);
        let extra_rows_needed = self.calculate_extra_rows_needed(rows_count);

        let inner_table = v_flex()
            .key_context("Table")
            .id("table")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::action_cancel))
            .on_action(cx.listener(Self::action_select_next))
            .on_action(cx.listener(Self::action_select_prev))
            .on_action(cx.listener(Self::action_select_next_col))
            .on_action(cx.listener(Self::action_select_prev_col))
            .size_full()
            .overflow_hidden()
            .child(self.render_table_head(left_cols_count, window, cx))
            .context_menu({
                let view = view.clone();
                move |this, window: &mut Window, cx: &mut Context<PopupMenu>| {
                    if let Some(row_ix) = view.read(cx).right_clicked_row {
                        view.read(cx)
                            .delegate
                            .context_menu(row_ix, this, window, cx)
                    } else {
                        this
                    }
                }
            })
            .map(|this| {
                if rows_count == 0 {
                    this.child(
                        div()
                            .size_full()
                            .child(self.delegate.render_empty(window, cx)),
                    )
                } else {
                    this.child(
                        h_flex().id("table-body").flex_grow().size_full().child(
                            uniform_list(
                                "table-uniform-list",
                                rows_count + extra_rows_needed,
                                cx.processor(
                                    move |table, visible_range: Range<usize>, window, cx| {
                                        // We must calculate the col sizes here, because the col sizes
                                        // need render_th first, then that method will set the bounds of each col.
                                        let col_sizes: Rc<Vec<gpui::Size<Pixels>>> = Rc::new(
                                            table
                                                .col_groups
                                                .iter()
                                                .skip(left_cols_count)
                                                .map(|col| col.bounds.size)
                                                .collect(),
                                        );

                                        table.load_more_if_need(
                                            rows_count,
                                            visible_range.end,
                                            window,
                                            cx,
                                        );
                                        table.update_visible_range_if_need(
                                            visible_range.clone(),
                                            Axis::Vertical,
                                            window,
                                            cx,
                                        );

                                        if visible_range.end > rows_count {
                                            table.scroll_to_row(
                                                std::cmp::min(
                                                    visible_range.start,
                                                    rows_count.saturating_sub(1),
                                                ),
                                                cx,
                                            );
                                        }

                                        let mut items = Vec::with_capacity(
                                            visible_range.end.saturating_sub(visible_range.start),
                                        );

                                        // Render fake rows to fill the table
                                        visible_range.for_each(|row_ix| {
                                            // Render real rows for available data
                                            items.push(table.render_table_row(
                                                row_ix,
                                                rows_count,
                                                left_cols_count,
                                                col_sizes.clone(),
                                                cols_count,
                                                window,
                                                cx,
                                            ));
                                        });

                                        items
                                    },
                                ),
                            )
                            .flex_grow()
                            .size_full()
                            .with_sizing_behavior(ListSizingBehavior::Auto)
                            .track_scroll(vertical_scroll_handle)
                            .into_any_element(),
                        ),
                    )
                }
            });

        let view = cx.entity().clone();
        div()
            .size_full()
            .when(self.border, |this| {
                this.rounded(cx.theme().radius)
                    .border_1()
                    .border_color(cx.theme().border)
            })
            .bg(cx.theme().table)
            .when(loading, |this| {
                this.child(self.delegate().render_loading(self.size, window, cx))
            })
            .when(!loading, |this| {
                this.child(inner_table)
                    .child(ScrollableMask::new(
                        cx.entity().entity_id(),
                        Axis::Horizontal,
                        &horizontal_scroll_handle,
                    ))
                    .when(self.right_clicked_row.is_some(), |this| {
                        this.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            this.right_clicked_row = None;
                            cx.notify();
                        }))
                    })
            })
            .child(canvas(
                move |bounds, _, cx| view.update(cx, |r, _| r.bounds = bounds),
                |_, _, _, _| {},
            ))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .size_full()
                    .when(self.scrollbar_visible.bottom, |this| {
                        this.child(self.render_horizontal_scrollbar(window, cx))
                    })
                    .when(self.scrollbar_visible.right && rows_count > 0, |this| {
                        this.children(self.render_vertical_scrollbar(window, cx))
                    }),
            )
    }
}
