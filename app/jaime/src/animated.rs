use std::time::Duration;

use aimer::animation::{AnimInstant, Animated, AnimationController};
use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::style::*;
use aimer::*;
use uuid::Uuid;

use crate::theme;

const ANIM_DURATION: Duration = Duration::from_millis(50);

pub fn start_my_animated_list() {
    AimerApp::start(theme::provide(MyAnimatedList))
}
#[allow(non_snake_case)]
#[widget(Stateful)]
pub struct MyAnimatedList;

pub struct ListItem {
    id: String,
    text: String,
    controller: AnimationController,
    pending_removal: bool,
    removal_started_at: Option<AnimInstant>,
}

pub struct MyListState {
    list: Vec<ListItem>,
    updater: StateUpdater<Self>,
}

impl Drop for ListItem {
    fn drop(&mut self) {
        println!("Dropping ListItem {}", self.id);
    }
}

impl StatefulWidget for MyAnimatedList {
    type State = MyListState;
    fn create_state(self) -> Self::State {
        MyListState {
            list: vec![],
            updater: StateUpdater::empty(),
        }
    }
}

impl State<MyAnimatedList> for MyListState {
    fn init_state(&mut self, _updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = _updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);

        {
            let now = AnimInstant::now();
            let has_dismissed = self
                .list
                .iter()
                .any(|item| {
                    item.pending_removal
                        && item
                            .removal_started_at
                            .map(|t| now.duration_since(t) >= ANIM_DURATION)
                            .unwrap_or(false)
                });
            if has_dismissed {}
        }

        Container::new()
            .child(
                Column::new()
                    .children(vec![
                        Container::new()
                            .height(Dimension::Px(90.0))
                            .box_decoration(BoxDecoration::new()
                                .background_color(theme.surface_color))
                            .child(
                                Row::new()
                                    .vertical_alignment(BoxAlignment::Center)
                                    .children(vec![
                                        Container::new()
                                            .child(
                                                Text::new(format!("Item in List: {}", self.list.iter().filter(|i| !i.pending_removal).count()))
                                                    .text_align(TextAlign::MidCenter)
                                                    .text_style(TextStyle::new()
                                                        .font_size(20)
                                                        .color(theme.on_surface_color))
                                            ).boxed(),
                                        Container::new()
                                             .height(Dimension::Px(50.0))
                                             .width(Dimension::Px(200.0))
                                            .child(
                                                Button::new()
                                                    .on_press({
                                                        let updater = self.updater;
                                                        move || {
                                                            updater.set_state(|state| {
                                                                let uuid = Uuid::new_v4().to_string();
                                                                let controller = AnimationController::new(
                                                                    Duration::from_millis(1000),
                                                                    curves::EASE_OUT,
                                                                );
                                                                controller.forward();
                                                                state.list.push(ListItem {
                                                                    id: uuid.clone(),
                                                                    text: uuid,
                                                                    controller,
                                                                    pending_removal: false,
                                                                    removal_started_at: None,
                                                                })
                                                            });
                                                        }
                                                    })
                                                    .decoration(
                                                        BoxDecoration::new()
                                                            .background_color(theme.primary_color),
                                                    )
                                                    .child(
                                                        Container::new()
                                                            .child(
                                                                Text::new("Add Item")
                                                                    .text_align(TextAlign::MidCenter)
                                                                    .text_style(TextStyle::new()
                                                                        .color(theme.on_primary_color)
                                                                        .font_size(15))
                                                            )
                                                    )
                                            ).boxed(),
                                    ]).boxed()
                            ).boxed(),
                        Scrollable::new()
                            .child(Column::new()
                                .children(self.list.iter().map(|item| {
                                    Animated::new(
                                        item.controller.clone(),
                                        AnimationEffect::SlideX { from: -1.0, to: 0.0 },
                                        Container::new()
                                            .margin(LayoutSpacing { top: Spacing::Px(10), ..Default::default() })
                                            .color(theme.surface_color)
                                            .height(Dimension::Px(50.0))
                                            .child(
                                                Row::new()
                                                    .children(vec![
                                                        Container::new()
                                                            .padding(LayoutSpacing { left: Spacing::Px(10), ..Default::default() })
                                                            .child(
                                                                Text::new(format!("Item : {}", item.text))
                                                                    .text_align(TextAlign::MidLeft)
                                                                    .text_style(TextStyle::new()
                                                                        .color(theme.on_surface_color)
                                                                        .font_size(15))
                                                            ).boxed(),
                                                        Container::new()
                                                            .width(Dimension::Px(200.0))
                                                            .height(Dimension::Px(50.0))
                                                            .child(
                                                                Button::new()
                                                                    .on_press({
                                                                        let item_id = item.id.clone();
                                                                        let updater = self.updater;
                                                                        move || {
                                                                            #[allow(clippy::collapsible_if)]
                                                                            updater.set_state_with(&item_id, |state, id| {
                                                                                if let Some(item) = state.list.iter_mut().find(|i| i.id == id) {
                                                                                    if !item.pending_removal {
                                                                                        item.pending_removal = true;
                                                                                        item.removal_started_at = Some(AnimInstant::now());
                                                                                        item.controller.reverse();
                                                                                    }
                                                                                }
                                                                            });
                                                                        }
                                                                    })
                                                                    .decoration(
                                                                        BoxDecoration::new()
                                                                            .background_color(theme.primary_color),
                                                                    )
                                                                    .child(
                                                                        Container::new()
                                                                            .height(Dimension::Px(50.0))
                                                                            .width(Dimension::Px(200.0))
                                                                            .child(
                                                                                Text::new("Delete")
                                                                                    .text_align(TextAlign::MidCenter)
                                                                                    .text_style(TextStyle::new()
                                                                                        .color(theme.on_primary_color)
                                                                                        .font_size(15))
                                                                            )
                                                                    )
                                                            ).boxed(),
                                                    ]).boxed()
                                            )
                                    ).boxed()
                                }).collect::<Vec<AnyWidget>>()
                                ))
                            .boxed()
                    ]).boxed()
            )
    }
}
