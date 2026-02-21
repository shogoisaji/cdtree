use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub theme: Theme,
    #[serde(default)]
    pub default_show_files: bool,
    #[serde(default)]
    pub default_show_hidden: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Theme {
    pub border_fg: String,
}

fn main() {
    let mut config = Config { theme: Theme::default(), default_show_files: false, default_show_hidden: false };
    config.default_show_files = true;
    let json = serde_json::to_string_pretty(&config).unwrap();
    println!("{}", json);
}
