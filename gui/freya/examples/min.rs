use freya::borderless::BorderlessPlugin;
use freya::prelude::*;

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut window = WindowConfig::new(|| {
        rect()
            .expanded()
            .center()
            .background((30, 30, 40))
            .child("hello")
    });
    if mode.contains('t') {
        window = window
            .with_transparency(true)
            .with_background(Color::TRANSPARENT)
            .with_decorations(false);
    }
    let mut config = LaunchConfig::new();
    if mode.contains('b') {
        config = config.with_plugin(BorderlessPlugin::new().with_corner_radius(14.));
    }
    eprintln!("launch {mode}");
    launch(config.with_window(window));
    eprintln!("launch returned");
}
