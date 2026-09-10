pub mod helper;
pub mod params;
pub mod photo;
pub mod process;
pub mod ui;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    Center,
    Up,
    Right,
    Bottom,
    Left,
}
