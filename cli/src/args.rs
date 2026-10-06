//! The command tree. Nouns, then verbs; the same verbs everywhere
//! (`ls`, `show`, `create`, `rm`, `set`).

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::resolve::{level, num};

const AFTER_HELP: &str = "\
Groups, inputs, outputs, scenes and rules can be named by id, by name, by the
start of a name, or by an alias (see `sonicforge alias`). Levels are written
75%, -6dB or 0.75.";

#[derive(Parser)]
#[command(name = "sonicforge", version, about = "Control the SonicForge audio mixer from the terminal", after_help = AFTER_HELP)]
#[command(arg_required_else_help = true, subcommand_required = true, infer_subcommands = true)]
pub struct Cli {
    /// Print JSON instead of text
    #[arg(long, global = true)]
    pub json: bool,
    /// Say nothing on success
    #[arg(long, short, global = true)]
    pub quiet: bool,
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// The daemon and UI services, health and engine settings
    #[command(subcommand)]
    System(System),
    /// Groups: the buses that apps play into
    #[command(subcommand)]
    Group(Group),
    /// Inputs: the application streams
    #[command(subcommand)]
    Input(Input),
    /// Group and master levels, pan and meters
    #[command(subcommand)]
    Fader(Fader),
    /// Per-group EQ, filters, compressor, gate and plugins
    #[command(subcommand)]
    Dsp(Dsp),
    /// The matrix: which group (or master) plays on which output
    #[command(subcommand)]
    Mtx(Mtx),
    /// Output devices and the cue outputs
    #[command(subcommand)]
    Output(Output),
    /// Your own short names for inputs, outputs and groups
    #[command(subcommand)]
    Alias(AliasCmd),
    /// Saved snapshots of the whole mixer
    #[command(subcommand)]
    Scene(Scene),
    /// Automation rules
    #[command(subcommand)]
    Rule(Rule),
    /// Ducking: one group turning others down
    #[command(subcommand)]
    Duck(Duck),
    /// Record a group to a file
    #[command(subcommand)]
    Record(Record),
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Target {
    /// The audio daemon
    Backend,
    /// The UI (and the Quickshell widget, when it is in use)
    Frontend,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Setting {
    /// Ring-buffer cushion, 10-150 ms
    BufferMs,
    /// Ceiling for output delay compensation, 0-500 ms
    MaxCompensationMs,
}

#[derive(Subcommand)]
pub enum System {
    /// Services, load and health at a glance (fails if the daemon is down)
    Status,
    /// Start the daemon and the UI, or just one of them
    Start { target: Option<Target> },
    /// Stop the UI and the daemon, or just one of them
    Stop { target: Option<Target> },
    /// Restart the daemon and the UI, or just one of them
    Restart { target: Option<Target> },
    /// Run the health checks (fails if any check is an error)
    Doctor,
    /// The signal-flow graph: nodes and the links between them
    Graph,
    /// The daemon's log
    Logs {
        /// Keep printing new lines
        #[arg(short, long)]
        follow: bool,
        /// How many past lines to show
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: u32,
    },
    /// Change an engine setting
    Set {
        key: Setting,
        #[arg(value_parser = num)]
        value: f32,
    },
}

#[derive(Subcommand)]
pub enum Group {
    /// List the groups
    Ls,
    /// Everything about one group
    Show { group: String },
    /// Create a group
    Create { name: String },
    /// Delete a group; its inputs go back to Default
    Rm { group: String },
    /// Rename a group
    Rename { group: String, name: String },
    /// Make a group feed another group instead of the outputs (`none` undoes it)
    Feed { group: String, parent: String },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Side {
    #[value(alias = "left")]
    L,
    #[value(alias = "right")]
    R,
    Stereo,
}

#[derive(Subcommand)]
pub enum Input {
    /// List the application streams
    Ls,
    /// Move an input into a group
    Move { input: String, group: String },
    /// Send an input back to the Default group
    Unroute { input: String },
    /// Mute an input
    Mute { input: String },
    /// Unmute an input
    Unmute { input: String },
    /// Send an input to one channel of its group, or back to stereo
    Side {
        input: String,
        #[arg(ignore_case = true)]
        side: Side,
    },
    /// Give the input's app a group of its own ("Own DSP")
    Strip { input: String },
    /// Per-app profiles
    #[command(subcommand)]
    Profile(Profile),
    /// Per-input level meters
    Levels {
        /// Keep updating until interrupted
        #[arg(short, long)]
        watch: bool,
    },
}

#[derive(Subcommand)]
pub enum Profile {
    /// Remember the input's group and volume for its app
    Save { input: String, label: String },
}

#[derive(Args)]
pub struct Toggle {
    /// Enable it
    #[arg(long, conflicts_with = "off")]
    pub on: bool,
    /// Disable it
    #[arg(long)]
    pub off: bool,
}

impl Toggle {
    pub fn given(&self) -> bool {
        self.on || self.off
    }

    pub fn apply(&self, current: bool) -> bool {
        (current || self.on) && !self.off
    }
}

#[derive(Subcommand)]
pub enum Fader {
    /// Every group and the master: level, pan, mono
    Ls,
    /// Set a group's level, or the master's
    Set {
        /// A group, or `master`
        target: String,
        #[arg(value_parser = level, allow_hyphen_values = true)]
        level: f32,
    },
    /// Pan a group: -1 is hard left, 0 centre, 1 hard right
    Pan {
        group: String,
        #[arg(value_parser = num, allow_hyphen_values = true)]
        pan: Option<f32>,
        /// Sum left and right to mono
        #[arg(long, conflicts_with = "stereo")]
        mono: bool,
        /// Back to stereo
        #[arg(long)]
        stereo: bool,
    },
    /// Group meters: peaks and gain reduction
    Levels {
        /// Keep updating until interrupted
        #[arg(short, long)]
        watch: bool,
    },
}

#[derive(Subcommand)]
pub enum Dsp {
    /// Show the EQ, or change one band (bands count from 1)
    Eq {
        group: String,
        band: Option<u32>,
        /// Centre frequency, Hz
        #[arg(long, value_parser = num)]
        freq: Option<f32>,
        /// Band gain, dB
        #[arg(long, value_parser = num, allow_hyphen_values = true)]
        gain: Option<f32>,
        #[arg(long, value_parser = num)]
        q: Option<f32>,
        #[command(flatten)]
        toggle: Toggle,
    },
    /// Move the whole EQ curve up or down, -18 to 18 dB
    EqLevel {
        group: String,
        #[arg(value_parser = num, allow_hyphen_values = true)]
        db: Option<f32>,
    },
    /// Show or change the high-pass filter
    Hpf {
        group: String,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Show or change the low-pass filter
    Lpf {
        group: String,
        #[command(flatten)]
        filter: FilterArgs,
    },
    /// Show or change the compressor
    Comp {
        group: String,
        /// dB
        #[arg(long, value_parser = num, allow_hyphen_values = true)]
        threshold: Option<f32>,
        #[arg(long, value_parser = num)]
        ratio: Option<f32>,
        /// ms
        #[arg(long, value_parser = num)]
        attack: Option<f32>,
        /// ms
        #[arg(long, value_parser = num)]
        release: Option<f32>,
        /// Make-up gain, dB
        #[arg(long, value_parser = num, allow_hyphen_values = true)]
        makeup: Option<f32>,
        #[command(flatten)]
        toggle: Toggle,
    },
    /// Show or change the gate
    Gate {
        group: String,
        /// dB
        #[arg(long, value_parser = num, allow_hyphen_values = true)]
        threshold: Option<f32>,
        /// ms
        #[arg(long, value_parser = num)]
        attack: Option<f32>,
        /// ms
        #[arg(long, value_parser = num)]
        release: Option<f32>,
        /// ms
        #[arg(long, value_parser = num)]
        hold: Option<f32>,
        #[command(flatten)]
        toggle: Toggle,
    },
    /// The group's plugin chain
    #[command(subcommand)]
    Plugin(Plugin),
}

#[derive(Args)]
pub struct FilterArgs {
    /// Cut-off frequency, Hz
    #[arg(long, value_parser = num)]
    pub freq: Option<f32>,
    #[arg(long, value_parser = num)]
    pub q: Option<f32>,
    #[command(flatten)]
    pub toggle: Toggle,
}

#[derive(Subcommand)]
pub enum Plugin {
    /// A group's chain, in processing order
    Ls { group: String },
    /// The plugin types that can be added
    Available,
    /// Append a plugin to a group's chain
    Add { group: String, plugin: String },
    /// Remove a slot
    Rm { group: String, index: u32 },
    /// Bypass a slot
    Bypass {
        group: String,
        index: u32,
        /// Put it back in the chain
        #[arg(long)]
        off: bool,
    },
    /// Set one parameter of a slot
    Set {
        group: String,
        index: u32,
        param: String,
        #[arg(value_parser = num, allow_hyphen_values = true)]
        value: f32,
    },
    /// Move a slot to another position
    Move { group: String, from: u32, to: u32 },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum SyncMode {
    Off,
    #[value(alias = "auto")]
    Automatic,
    Manual,
}

#[derive(Subcommand)]
pub enum Mtx {
    /// Routes of every bus, or of one group or `master`
    Ls { target: Option<String> },
    /// Add a route, or change its level or state
    Set {
        /// A group, or `master`
        target: String,
        output: String,
        #[arg(long, value_parser = level, allow_hyphen_values = true)]
        gain: Option<f32>,
        #[command(flatten)]
        toggle: Toggle,
    },
    /// Remove a route
    Rm {
        /// A group, or `master`
        target: String,
        output: String,
    },
    /// Delay compensation for a route
    Sync {
        /// A group, or `master`
        target: String,
        output: String,
        mode: SyncMode,
        /// The delay for `manual`
        #[arg(value_parser = num)]
        delay_ms: Option<f32>,
    },
}

#[derive(Subcommand)]
pub enum Output {
    /// List the output devices
    Ls,
    /// Show or set a device's own volume
    Volume {
        output: String,
        #[arg(value_parser = level, allow_hyphen_values = true)]
        level: Option<f32>,
    },
    /// Mute a device
    Mute { output: String },
    /// Unmute a device
    Unmute { output: String },
    /// Show or choose the device for groups with no routes of their own (`none` clears it)
    Default { output: Option<String> },
    /// The cue (headphone preview) outputs
    #[command(subcommand)]
    Cue(Cue),
}

#[derive(Subcommand)]
pub enum Cue {
    /// List the cue outputs
    Ls,
    /// Make an output a cue output
    Add { output: String },
    /// Stop using an output for cue
    Rm { output: String },
    /// Show or set how loud the rest of the mix stays on cue outputs while cueing
    Duck {
        #[arg(value_parser = level)]
        level: Option<f32>,
    },
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
pub enum Kind {
    Input,
    Output,
    Group,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Input => "input",
            Kind::Output => "output",
            Kind::Group => "group",
        }
    }
}

#[derive(Subcommand)]
pub enum AliasCmd {
    /// List the aliases
    Ls,
    /// Name an input (its app), an output or a group
    Set {
        alias: String,
        /// What it stands for, as you would name it in any other command
        target: String,
        /// Needed only when the target could be more than one kind of thing
        #[arg(long)]
        kind: Option<Kind>,
    },
    /// Remove an alias
    Rm { alias: String },
}

#[derive(Subcommand)]
pub enum Scene {
    /// List the saved scenes
    Ls,
    /// Save the current state as a new scene
    Save { name: String },
    /// Overwrite a scene with the current state
    Update { scene: String },
    /// Copy a scene
    Dup { scene: String, name: String },
    /// Delete a scene
    Rm { scene: String },
    /// Apply a scene
    Apply { scene: String },
}

#[derive(Subcommand)]
pub enum Rule {
    /// List the rules
    Ls,
    /// One rule in full, as JSON
    Show { rule: String },
    /// Create or replace a rule from a JSON file (`-` reads stdin)
    Save { file: String },
    /// Enable a rule
    Enable { rule: String },
    /// Disable a rule
    Disable { rule: String },
    /// Delete a rule
    Rm { rule: String },
    /// Recent firings
    History,
}

#[derive(Subcommand)]
pub enum Duck {
    /// List the ducking rules
    Ls,
    /// When the trigger group is heard, turn the target groups down
    Create {
        trigger: String,
        #[arg(required = true)]
        targets: Vec<String>,
        /// How far down, dB
        #[arg(long, value_parser = num, default_value_t = 12.0)]
        reduction: f32,
        /// ms
        #[arg(long, value_parser = num, default_value_t = 50.0)]
        attack: f32,
        /// ms
        #[arg(long, value_parser = num, default_value_t = 200.0)]
        hold: f32,
        /// ms
        #[arg(long, value_parser = num, default_value_t = 500.0)]
        release: f32,
        /// Trigger level, dB
        #[arg(long, value_parser = num, default_value_t = -40.0, allow_hyphen_values = true)]
        threshold: f32,
    },
    /// Enable a ducking rule
    Enable { id: u32 },
    /// Disable a ducking rule
    Disable { id: u32 },
    /// Delete a ducking rule
    Rm { id: u32 },
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Wav16,
    Wav32,
    Flac,
}

#[derive(Subcommand)]
pub enum Record {
    /// List the recordings in progress
    Ls,
    /// Start recording a group
    Start {
        group: String,
        path: std::path::PathBuf,
        /// Defaults to flac for a .flac path, wav16 otherwise
        #[arg(long)]
        format: Option<Format>,
    },
    /// Stop a recording and finish its file
    Stop { id: u32 },
}
