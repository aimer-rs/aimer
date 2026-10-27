use aimer::input::TextEditingController;
use aimer::input::button::Button;
use aimer::input::input::{AsyncTextFieldCallback, InputType, TextField};
use aimer::macros::StatefulWidget;
use aimer::style::*;
use aimer::{AimerApp, *};
use uuid::Uuid;

#[allow(unused)]
pub fn start_my_list() {
    AimerApp::start(crate::theme::provide(MyList::new().boxed()))
}

#[derive(StatefulWidget)]
pub struct MyList {}

impl MyList {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Debug)]
pub struct ListItem {
    id: String,
    text: String,
}

pub struct MyListState {
    list: Vec<ListItem>,
    input_controller: TextEditingController,
    is_cooldown: bool,
    updater: StateUpdater<Self>,
}

impl Drop for ListItem {
    fn drop(&mut self) {
        println!("Dropping ListItem {}", self.id);
    }
}

impl StatefulWidget for MyList {
    type State = MyListState;
    fn create_state(self) -> Self::State {
        MyListState {
            list: vec![],
            updater: StateUpdater::empty(),
            input_controller: TextEditingController::new(),
            is_cooldown: false,
        }
    }
}

impl State<MyList> for MyListState {
    fn init_state(&mut self, _updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = _updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::copied(ctx);

        Container::new()
            .color(theme.background_color)
            .padding(LayoutSpacing { top: Spacing::Px(105), ..Default::default() })
            .child(
                Column::new()
                    .children(vec![
                        Container::new()
                            .height(Dimension::Px(90.0))
                            .box_decoration(BoxDecoration::new()
                                .background_color(theme.surface_color))
                            .padding(LayoutSpacing::vertical(Spacing::Px(10)))
                            .child(
                                Row::new()
                                    .gaps(LayoutSpacing::all(Spacing::Px(10)))
                                    .children(vec![
                                        Container::new()
                                            .width(Dimension::Px(50.0))
                                            .margin(LayoutSpacing { left: Spacing::Px(10), ..Default::default() })
                                            .child(
                                                Text::new(format!("Item {}", self.list.len()))
                                                    .text_align(TextAlign::MidLeft)
                                                    .text_style(TextStyle::new()
                                                        .font_size(20)
                                                        .color(theme.on_surface_color))
                                            ).boxed(),

                                        Expanded::new()

                                            .box_child(Container::new()
                                            .padding(LayoutSpacing::all(Spacing::Px(10)))
                                            .child(
                                                TextField::new()
                                                    .padding(LayoutSpacing::all(Spacing::Px(10)))
                                                    .text_align(TextAlign::MidLeft)
                                                    .text_style(TextStyle::new().font_weight(FontWeight::Bolder))
                                                    .controller(self.input_controller.clone())
                                                    .input_type(InputType::Text)
                                                    .on_changed({
                                                        let is_cooldown = self.is_cooldown;
                                                        AsyncTextFieldCallback(move |item: String| async move {
                                                            if !is_cooldown {
                                                                console::debug!("Input changed: {}", item);
                                                            }else{
                                                                console::debug!("Input changed: COOLDOWN");
                                                            }
                                                        })
                                                    })
                                                    .prompt("Input any here....")
                                                    .decoration(BoxDecoration::new()
                                                        .background_color(crate::theme::recessed_surface(&theme))
                                                        .border(BoxBorder::all(
                                                            BorderSlice::new()
                                                                .style(BorderStyle::Solid)
                                                                .color(theme.on_surface_color)
                                                                .stroke(2),
                                                        ))
                                                        .outline(BoxOutline::all(
                                                            BorderSlice::new()
                                                                .style(BorderStyle::Solid)
                                                                .color(theme.on_surface_color)
                                                                .stroke(2),
                                                        )))
                                                    .hover_decoration(BoxDecoration::new()
                                                        .background_color(theme.surface_color)
                                                        .border(BoxBorder::all(
                                                            BorderSlice::new()
                                                                .style(BorderStyle::Solid)
                                                                .color(theme.on_surface_color)
                                                                .stroke(2),
                                                        ))
                                                        .outline(BoxOutline::all(
                                                            BorderSlice::new()
                                                                .style(BorderStyle::Solid)
                                                                .color(theme.primary_color)
                                                                .stroke(2),
                                                        )))
                                                    .focus_decoration(BoxDecoration::new()
                                                        .background_color(theme.background_color)
                                                        .border(BoxBorder::all(
                                                            BorderSlice::new()
                                                                .style(BorderStyle::Solid)
                                                                .color(theme.primary_color)
                                                                .stroke(2),
                                                        ))
                                                        .outline(BoxOutline::all(
                                                            BorderSlice::new()
                                                                .style(BorderStyle::Solid)
                                                                .color(theme.on_surface_color)
                                                                .stroke(2),
                                                        )))
                                            ).boxed(),),


                                        Container::new()
                                            .padding(LayoutSpacing::all(Spacing::Px(10)))
                                            .width(Dimension::Px(100.0))
                                            .child(
                                                Button::new()
                                                    .on_press({
                                                        let updater = self.updater;
                                                         move || {
                                                            println!("Button pressed with text: {}", updater.read_state().input_controller.value().text());
                                                            updater.set_state(|state| {
                                                                let uuid = Uuid::new_v4().to_string();
                                                                let text = state.input_controller.value().text().to_owned();
                                                                state.input_controller.clear();
                                                                state.list.push(ListItem {
                                                                    id: uuid,
                                                                    text,
                                                                })
                                                            });
                                                        }
                                                    })
                                                    .decoration(BoxDecoration::new()
                                                        .background_color(theme.primary_color)
                                                        .border(BoxBorder::all(
                                                            BorderSlice::new()
                                                                .style(BorderStyle::Solid)
                                                                .color(theme.on_primary_color)
                                                                .stroke(2),
                                                        ))
                                                        .outline(BoxOutline::all(
                                                            BorderSlice::new()
                                                                .style(BorderStyle::Solid)
                                                                .color(Color::Transparent)
                                                                .stroke(2),
                                                        )))
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
                                    Container::new()
                                        .margin(LayoutSpacing { top: Spacing::Px(10), ..Default::default() })
                                        .color(theme.surface_color)
                                        .height(Dimension::Px(50.0))
                                        .child(
                                            Row::new()
                                                .children(vec![
                                                    Expanded::new()
                                                        .child(
                                                            Container::new()
                                                                .padding(LayoutSpacing { left: Spacing::Px(10), ..Default::default() })
                                                                .child(
                                                                    Text::new(format!("Item : {}", item.text))
                                                                        .text_align(TextAlign::MidLeft)
                                                                        .text_style(TextStyle::new()
                                                                            .color(theme.on_surface_color)
                                                                            .font_size(15))
                                                                ),
                                                        ).boxed(),
                                                    Container::new()
                                                        .width(Dimension::Px(100.0))
                                                        .height(Dimension::Px(50.0))
                                                        .child(
                                                            Button::new()
                                                                .on_press({
                                                                    let item_id = item.id.clone();
                                                                    let updater = self.updater;
                                                                    move || {
                                                                        let another_item_id = item_id.clone();
                                                                        updater.set_state( move |state| {
                                                                            state.list.retain(|i| i.id != another_item_id);
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
                                                                            Text::new("Delete")
                                                                                .text_align(TextAlign::MidCenter)
                                                                                .text_style(TextStyle::new()
                                                                                    .color(theme.on_primary_color)
                                                                                    .font_size(15))
                                                                        )
                                                                )
                                                        ).boxed(),
                                                ]).boxed()
                                        ).boxed()
                                }).collect::<Vec<AnyWidget>>()
                                ))
                            .boxed()
                    ]).boxed()
            )
    }
}
