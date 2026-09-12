pub mod config;
pub mod helper;
pub mod params;
pub mod photo;
pub mod process;
pub mod theme;
pub mod ui;
pub mod workspace;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Position {
    Center,
    Up,
    Right,
    Bottom,
    Left,
}
