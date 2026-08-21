//! PipeWire connection and per-app stream enumeration.
//!
//! Build-order stage 1 (PROJECT_SPEC.md Section 9): connect, list existing
//! app streams, log them. Stage 2 adds virtual group nodes and routing;
//! that's where DSP and the audio callback (Section 6) will live.

use pipewire as pipewire_rs;
use pipewire_rs::registry::GlobalObject;
use pipewire_rs::spa::utils::dict::DictRef;

use crate::config::Config;

/// A single detected application audio output stream.
#[derive(Debug, Clone)]
pub struct AppStream {
    pub id: u32,
    pub name: String,
}

pub fn run(_cfg: Config) -> Result<(), pipewire_rs::Error> {
    let mainloop = pipewire_rs::main_loop::MainLoopBox::new(None)?;
    let context = pipewire_rs::context::ContextBox::new(mainloop.loop_(), None)?;
    let core = context.connect(None)?;
    let registry = core.get_registry()?;

    let _listener = registry
        .add_listener_local()
        .global(on_global)
        .register();

    tracing::info!("connected to pipewire, watching for app streams (ctrl-c to quit)");
    mainloop.run();

    Ok(())
}

fn on_global(global: &GlobalObject<&DictRef>) {
    let Some(props) = global.props else {
        return;
    };
    let Some(media_class) = props.get("media.class") else {
        return;
    };

    // Playback streams from individual applications. Virtual group nodes we
    // create later will use a different media.class so they don't show up
    // here as routable "app" sources.
    if media_class != "Stream/Output/Audio" {
        return;
    }

    let name = props
        .get("application.name")
        .or_else(|| props.get("node.name"))
        .unwrap_or("unknown");

    tracing::info!(id = global.id, name, "app stream");
}
