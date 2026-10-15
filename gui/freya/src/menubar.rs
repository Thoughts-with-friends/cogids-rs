//! The menu bar (File / System / View / Config / Help) drawn from the menu
//! tree shared with the egui and Tauri GUIs (`ds_common::menu`), with
//! freya's `Menu` / `SubMenu` / `MenuButton`.
//!
//! Disabled entries are drawn greyed out with their reason as a tooltip;
//! checkboxes get a check mark and radio entries a dot.
use ds_common::menu::{Item, Menu as DsMenu};
use freya::prelude::*;

use crate::actions;
use crate::state::Ctx;
use crate::theme::{self, ACCENT, TEXT, TEXT_DIM, TEXT_FAINT};

#[derive(PartialEq)]
pub struct MenuBar {
    pub menus: Vec<DsMenu>,
}

impl Component for MenuBar {
    fn render(&self) -> impl IntoElement {
        let mut open = use_state(|| None::<usize>);
        let ctx = use_consume::<Ctx>();

        rect()
            .horizontal()
            .cross_align(Alignment::Center)
            .spacing(2.)
            .children(self.menus.iter().enumerate().map(|(i, menu)| {
                let is_open = open() == Some(i);
                let items = menu.items.clone();
                rect()
                    .key(i)
                    .child(TopButton {
                        label: menu.label.clone(),
                        active: is_open,
                        index: i,
                        open,
                    })
                    .maybe_child(is_open.then(|| {
                        rect().width(Size::px(0.)).height(Size::px(0.)).child(
                            rect().margin((4., 0., 0., 0.)).child(
                                Menu::new()
                                    .on_close(move |_| open.set(None))
                                    .children(items.iter().map(|it| entry(it, &ctx))),
                            ),
                        )
                    }))
                    .into_element()
            }))
    }
}

/// One title of the bar ("File", "System", ...).
#[derive(PartialEq)]
struct TopButton {
    label: String,
    active: bool,
    index: usize,
    open: State<Option<usize>>,
}

impl Component for TopButton {
    fn render(&self) -> impl IntoElement {
        let mut hover = use_state(|| false);
        let mut open = self.open;
        let index = self.index;
        let background = if self.active {
            theme::rgba(ACCENT, 70)
        } else if hover() {
            Color::from_argb(28, 255, 255, 255)
        } else {
            Color::TRANSPARENT
        };
        rect()
            .padding((5., 11.))
            .corner_radius(7.)
            .background(background)
            .color(theme::rgb(if self.active || hover() {
                TEXT
            } else {
                TEXT_DIM
            }))
            .font_size(13.)
            .on_pointer_enter(move |_| {
                hover.set(true);
                // Sliding along the bar while a menu is open opens the next.
                if open.peek().is_some_and(|i| i != index) {
                    open.set(Some(index));
                }
            })
            .on_pointer_leave(move |_| hover.set(false))
            .on_press(move |e: Event<PressEventData>| {
                e.stop_propagation();
                if open() == Some(index) {
                    open.set(None);
                } else {
                    open.set(Some(index));
                }
            })
            .child(self.label.clone())
    }
}

/// The left column of an entry: check mark, radio dot or nothing.
fn mark(checked: Option<bool>, radio: bool) -> Element {
    let text = match (checked, radio) {
        (Some(true), true) => "●",
        (Some(true), false) => "✓",
        _ => "",
    };
    rect()
        .width(Size::px(20.))
        .color(theme::rgb(ACCENT))
        .font_size(12.)
        .child(text)
        .into_element()
}

/// Label, then the shortcut aligned right.
fn row(label: &str, shortcut: Option<&str>, checked: Option<bool>, radio: bool) -> Element {
    rect()
        .horizontal()
        .cross_align(Alignment::Center)
        .min_width(Size::px(200.))
        .child(mark(checked, radio))
        .child(rect().font_size(13.).child(label.to_owned()))
        .maybe_child(shortcut.map(|s| {
            rect()
                .margin((0., 0., 0., 28.))
                .font_size(11.5)
                .color(theme::rgb(TEXT_FAINT))
                .child(s.to_owned())
        }))
        .into_element()
}

/// One menu entry as a freya element.
fn entry(item: &Item, ctx: &Ctx) -> Element {
    match item {
        Item::Entry {
            label,
            action,
            enabled,
            checked,
            radio,
            reason,
            shortcut,
            hint,
        } => {
            let row = row(label, shortcut.as_deref(), *checked, *radio);
            match (action, enabled) {
                (Some(action), true) => {
                    let action = action.clone();
                    let ctx = ctx.clone();
                    let button = MenuButton::new()
                        .on_press(move |_| actions::run(&ctx, action.clone()))
                        .child(row);
                    match hint {
                        Some(hint) => TooltipContainer::new(Tooltip::new_text(hint.clone()))
                            .position(AttachedPosition::Right)
                            .child(button)
                            .into_element(),
                        None => button.into_element(),
                    }
                }
                // Selected radio entry without an action (the only choice).
                (None, true) => disabled_row(row, None),
                _ => disabled_row(row, reason.clone()),
            }
        }
        Item::Submenu { label, items, .. } => SubMenu::new()
            .label(
                rect()
                    .horizontal()
                    .cross_align(Alignment::Center)
                    .child(mark(None, false))
                    .child(rect().font_size(13.).child(label.clone()))
                    .child(
                        rect()
                            .margin((0., 0., 0., 24.))
                            .color(theme::rgb(TEXT_FAINT))
                            .child("›"),
                    ),
            )
            .children(items.iter().map(|it| entry(it, ctx)))
            .into_element(),
        Item::Separator => rect()
            .width(Size::fill())
            .height(Size::px(1.))
            .margin((5., 8.))
            .background(theme::hairline())
            .into_element(),
        Item::Label { text } => rect()
            .padding((6., 14., 4., 34.))
            .font_size(11.5)
            .color(theme::rgb(TEXT_FAINT))
            .child(text.clone())
            .into_element(),
    }
}

/// An entry that cannot be used: greyed out, the reason on hover.
fn disabled_row(row: Element, reason: Option<String>) -> Element {
    let body = rect()
        .padding((8., 14.))
        .min_width(Size::px(105.))
        .width(Size::fill_minimum())
        .color(theme::rgb(TEXT_FAINT))
        .child(row);
    match reason {
        Some(reason) => TooltipContainer::new(Tooltip::new_text(reason))
            .position(AttachedPosition::Right)
            .child(body)
            .into_element(),
        None => body.into_element(),
    }
}
