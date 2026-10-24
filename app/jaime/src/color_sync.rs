use aimer::Dimension::Percent;
use aimer::console::debug;
use aimer::style::{LayoutSpacing, Spacing};
use aimer::{
    AimerApp, AnyElement, AnyWidget, BuildContext, Color, Container, Row, SizedBox,
    StatelessWidget, Widget,
};

#[allow(unused)]
pub struct ColorSync;

#[allow(unused)]
pub fn start_color_sync() {
    AimerApp::start(ColorSync)
}

impl Widget for ColorSync {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        self.build(ctx)
            .to_element(ctx)
    }
}

impl aimer::PortableWidget for ColorSync {}

impl StatelessWidget for ColorSync {
    fn build(&self, _: &BuildContext) -> impl Widget {
        let colors = [
            Color::Rgb(255, 0, 0),
            Color::Rgb(255, 255, 0),
            Color::Rgb(255, 255, 255),
            Color::Rgb(0, 0, 255),
            Color::Rgb(0, 255, 0),
            Color::Rgb(0, 255, 255),
            Color::Rgb(255, 0, 255),
            Color::Rgb(255, 128, 0),
            Color::Rgb(255, 255, 128),
            Color::Rgb(128, 255, 0),
            Color::Rgb(128, 255, 128),
            Color::Rgb(0, 128, 255),
            Color::Rgb(128, 0, 255),
        ];
        debug!("Loading the colors:");
        let children: Vec<AnyWidget> = colors
            .iter()
            .map(|color| {
                SizedBox::new()
                    .width(Percent(100.0))
                    .color(*color)
                    .boxed()
            })
            .collect();
        Container::new()
            .width(Percent(100.0))
            .height(Percent(100.0))
            .padding(LayoutSpacing::all(Spacing::Px(10)))
            .child(Row::new().children(children))
    }
}
