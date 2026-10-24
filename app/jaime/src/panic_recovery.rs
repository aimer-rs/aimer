use aimer::macros::widget;
use aimer::*;

/// Builds the panic-recovery showcase without starting an application.
pub fn panic_recovery_example() -> impl Widget {
    Container::new().child(PanicRecoveryExample::new())
}

pub fn start_panic_recovery_example() {
    AimerApp::start(panic_recovery_example())
}

struct MissingProviderValue;

#[derive(Clone)]
#[widget(Stateless)]
struct PanicRecoveryExample {}

impl PanicRecoveryExample {
    fn new() -> Self {
        Self {}
    }
}

impl StatelessWidget for PanicRecoveryExample {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let _missing = ProviderHandle::<MissingProviderValue>::of(ctx);

        Container::new().child(Text::new(
            "This is replaced by the recovered red error screen.",
        ))
    }
}
