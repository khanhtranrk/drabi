use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Deserialize, Serialize)]
pub struct GameCard {
    pub id: String,
    pub name: String,
    pub grid: Option<PathBuf>,
    pub hero: Option<PathBuf>,
    pub logo: Option<PathBuf>,
    pub icon: Option<PathBuf>,
    pub publisher: Option<String>,
    pub version: Option<String>,
    pub last_played: Option<u64>,
    pub playtime: Option<u64>,
}

#[derive(Deserialize, Serialize)]
pub enum Reply {
    Select(String),
    Exit,
    System(SystemAction),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum SystemAction {
    Logout,
    Restart,
    Shutdown,
}
