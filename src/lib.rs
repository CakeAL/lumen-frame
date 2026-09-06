pub mod generate;
pub mod params;
pub mod photo;
pub mod process;
pub mod helper;

#[derive(Debug, Clone, Copy)]
pub enum Position {
    Center,
    Up,
    Right,
    Bottom,
    Left,
}