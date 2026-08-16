use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "Template matching based video segment extraction tool (Rust version)."
)]
pub struct Args {
    /// Input video file path (can be passed via drag and drop)
    #[arg(index = 1, allow_hyphen_values = true)]
    pub input_positional: Option<String>,

    #[arg(short, long, allow_hyphen_values = true)]
    pub input: Option<String>,

    #[arg(long, default_value = "start.png")]
    pub start_template: String,

    #[arg(long, default_value = "end.png")]
    pub end_template: String,

    #[arg(long, default_value = "win.png")]
    pub win_template: String,

    #[arg(long, default_value = "lose.png")]
    pub lose_template: String,

    #[arg(short, long)]
    pub output: Option<String>,

    #[arg(long, default_value_t = 0.9)]
    pub threshold: f64,

    #[arg(long, default_value_t = 60)]
    pub step_frames: usize,

    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    pub start_offset: i64,

    #[arg(long, default_value_t = -120, allow_hyphen_values = true)]
    pub end_offset: i64,

    #[arg(long, default_value_t = 180)]
    pub win_offset: usize,

    #[arg(long)]
    pub start_roi: Option<String>,

    #[arg(long)]
    pub end_roi: Option<String>,

    #[arg(long)]
    pub win_roi: Option<String>,

    #[arg(long)]
    pub lose_roi: Option<String>,
}
